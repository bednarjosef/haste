//! Compares a Haste screenshot with the vanilla reference one and writes a side-by-side +
//! difference PNG. Used by `scripts/compare-vanilla.sh`.
//!
//! usage: compare-screenshot <vanilla.png> <haste.png> <out.png> [tolerance]
//!        [--region x,y,w,h]... [--grid NxN]
//! Prints one line: `<percent differing> <mean error> <max error>`; with `--region`/`--grid`,
//! one `region` line per rectangle after it (`mean error` over that rectangle). The out PNG
//! is vanilla | haste | difference (red = over tolerance), left to right.

use anyhow::{bail, Result};
use haste_testkit::screenshots::{compare_screenshot, load_png};
use image::{GenericImage, Rgba, RgbaImage};
use std::path::Path;

/// One rectangle in vanilla-shot pixel coordinates.
struct Region {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    label: String,
}

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut regions: Vec<Region> = Vec::new();
    let mut grid: Option<(u32, u32)> = None;
    // `--cells nx,ny,x,y,w,h`: per-cell mean colours of both images and the
    // signed difference (for classifying what carries a bias).
    let mut cells: Option<(u32, u32, Region)> = None;
    let mut positional = Vec::new();
    while let Some(arg) = args.first().cloned() {
        match arg.as_str() {
            "--region" => {
                args.remove(0);
                let spec = args.remove(0);
                let v: Vec<&str> = spec.split(',').collect();
                if v.len() != 4 {
                    bail!("--region wants x,y,w,h");
                }
                let [x, y, w, h] = [v[0].parse()?, v[1].parse()?, v[2].parse()?, v[3].parse()?];
                regions.push(Region {
                    x,
                    y,
                    w,
                    h,
                    label: spec.clone(),
                });
            }
            "--grid" => {
                args.remove(0);
                let spec = args.remove(0);
                let v: Vec<&str> = spec.split('x').collect();
                if v.len() != 2 {
                    bail!("--grid wants NxN");
                }
                grid = Some((v[0].parse()?, v[1].parse()?));
            }
            "--cells" => {
                args.remove(0);
                let spec = args.remove(0);
                let v: Vec<&str> = spec.split(',').collect();
                if v.len() != 6 {
                    bail!("--cells wants nx,ny,x,y,w,h");
                }
                let [nx, ny, x, y, w, h] = [
                    v[0].parse()?,
                    v[1].parse()?,
                    v[2].parse()?,
                    v[3].parse()?,
                    v[4].parse()?,
                    v[5].parse()?,
                ];
                cells = Some((
                    nx,
                    ny,
                    Region {
                        x,
                        y,
                        w,
                        h,
                        label: String::new(),
                    },
                ));
            }
            _ => positional.push(args.remove(0)),
        }
    }
    if positional.len() < 3 {
        bail!("usage: compare-screenshot <vanilla.png> <haste.png> <out.png> [tolerance] [--region x,y,w,h]... [--grid NxN]");
    }
    let tolerance: u8 = positional
        .get(3)
        .map(|t| t.parse())
        .transpose()?
        .unwrap_or(8);
    let vanilla = load_png(Path::new(&positional[0]))?;
    // The reference's recorded metadata (same stem, .json): the time of day and
    // render distance the vanilla shot was taken at (stderr: the summary line
    // on stdout is parsed by scripts/compare-vanilla.sh).
    if let Some(stem) = Path::new(&positional[0]).extension().and_then(|_| {
        positional[0]
            .strip_suffix(".png")
            .map(|s| format!("{s}.json"))
    }) {
        if let Ok(meta) = haste_testkit::screenshots::ScreenshotMeta::load(Path::new(&stem)) {
            eprintln!(
                "meta: time_of_day {} render_distance {}",
                meta.time_of_day, meta.render_distance
            );
        }
    }
    let haste = load_png(Path::new(&positional[1]))?;
    let diff = compare_screenshot(&vanilla, &haste, tolerance)?;
    let (w, h) = vanilla.dimensions();
    let mut out = RgbaImage::from_pixel(w * 3, h, Rgba([0, 0, 0, 255]));
    out.copy_from(&vanilla, 0, 0)?;
    out.copy_from(&haste, w, 0)?;
    if let Some(d) = &diff.diff_image {
        out.copy_from(d, w * 2, 0)?;
    }
    out.save(&positional[2])?;
    println!(
        "{:.2} {:.2} {}",
        diff.diff_fraction * 100.0,
        diff.mean_error,
        diff.max_error
    );

    if let Some((nx, ny)) = grid {
        let tw = w / nx;
        let th = h / ny;
        for gy in 0..ny {
            for gx in 0..nx {
                regions.push(Region {
                    x: gx * tw,
                    y: gy * th,
                    w: tw,
                    h: th,
                    label: format!("grid {},{}", gx, gy),
                });
            }
        }
    }
    if let Some((nx, ny, area)) = cells {
        // Per cell: both images' mean colour and the signed r/g/b difference,
        // so cells can be classified (sky gaps are blue and bright, canopy
        // green, fog light and flat).
        let cw = area.w / nx;
        let ch = area.h / ny;
        for gy in 0..ny {
            for gx in 0..nx {
                let (x0, y0) = (area.x + gx * cw, area.y + gy * ch);
                let mut vs = [0u64; 3];
                let mut hs = [0u64; 3];
                let mut n = 0u64;
                for y in y0..(y0 + ch).min(h) {
                    for x in x0..(x0 + cw).min(w) {
                        let pa = vanilla.get_pixel(x, y);
                        let pb = haste.get_pixel(x, y);
                        for c in 0..3 {
                            vs[c] += u64::from(pa.0[c]);
                            hs[c] += u64::from(pb.0[c]);
                        }
                        n += 1;
                    }
                }
                let n = usize::try_from(n).unwrap_or(0);
                if n > 0 {
                    let v: Vec<u64> = vs.iter().map(|s| s / n as u64).collect();
                    let s: Vec<i64> = (0..3)
                        .map(|c| (hs[c] as i64 - vs[c] as i64) / n as i64)
                        .collect();
                    println!(
                        "cell {},{} vanilla ({:3},{:3},{:3}) haste ({:3},{:3},{:3}) d ({:+4},{:+4},{:+4})",
                        gx,
                        gy,
                        v[0],
                        v[1],
                        v[2],
                        hs[0] / n as u64,
                        hs[1] / n as u64,
                        hs[2] / n as u64,
                        s[0],
                        s[1],
                        s[2]
                    );
                }
            }
        }
    }
    for r in &regions {
        let mut sum = 0u64;
        let mut signed = 0i64;
        let mut count = 0u64;
        for y in r.y..r.y.saturating_add(r.h).min(h) {
            for x in r.x..r.x.saturating_add(r.w).min(w) {
                let pa = vanilla.get_pixel(x, y);
                let pb = haste.get_pixel(x, y);
                let err =
                    pa.0.iter()
                        .zip(pb.0.iter())
                        .map(|(a, b)| a.abs_diff(*b) as u64)
                        .sum::<u64>();
                sum += err / 4;
                signed += pb.0[0] as i64 - pa.0[0] as i64;
                count += 1;
            }
        }
        if count > 0 {
            let mut channels = [0i64; 3];
            for y in r.y..r.y.saturating_add(r.h).min(h) {
                for x in r.x..r.x.saturating_add(r.w).min(w) {
                    let pa = vanilla.get_pixel(x, y);
                    let pb = haste.get_pixel(x, y);
                    for (ch, (&a, &b)) in pa.0.iter().zip(pb.0.iter()).take(3).enumerate() {
                        channels[ch] += b as i64 - a as i64;
                    }
                }
            }
            println!(
                "region {} mean {:.2} signed {:+.2} (r{:+.1} g{:+.1} b{:+.1})",
                r.label,
                sum as f64 / count as f64,
                signed as f64 / count as f64,
                channels[0] as f64 / count as f64,
                channels[1] as f64 / count as f64,
                channels[2] as f64 / count as f64
            );
        }
    }
    Ok(())
}
