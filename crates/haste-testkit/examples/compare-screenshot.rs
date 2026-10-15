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
    for r in &regions {
        let mut sum = 0u64;
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
                count += 1;
            }
        }
        if count > 0 {
            println!("region {} mean {:.2}", r.label, sum as f64 / count as f64);
        }
    }
    Ok(())
}
