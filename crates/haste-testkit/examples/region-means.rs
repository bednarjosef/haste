//! Prints per-region mean colour of two screenshots side by side (vanilla vs haste), for
//! finding which part of the picture a difference lives in. Regions are fixed bands of the
//! image (sky, upper, middle, lower) plus a centre crop; each row is `region vanilla-R,G,B
//! haste-R,G,B delta-R,G,B` plus mean alpha. Used by the visual-parity work: numbers only,
//! the images are never opened.
//!
//! usage: region-means <vanilla.png> <haste.png>

use anyhow::{bail, Result};
use haste_testkit::screenshots::load_png;
use std::path::Path;

/// A named horizontal band: rows `y0..y1` (fractions of the height, top down), full width.
const BANDS: &[(&str, f32, f32)] = &[
    ("sky-top", 0.0, 0.15),
    ("sky-horizon", 0.15, 0.30),
    ("upper", 0.30, 0.55),
    ("middle", 0.55, 0.75),
    ("lower", 0.75, 1.0),
];

fn band_means(img: &image::RgbaImage, y0: u32, y1: u32) -> (f64, f64, f64, f64, u64) {
    let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
    let mut n = 0u64;
    for y in y0..y1 {
        for x in 0..img.width() {
            let p = img.get_pixel(x, y);
            r += p[0] as u64;
            g += p[1] as u64;
            b += p[2] as u64;
            a += p[3] as u64;
            n += 1;
        }
    }
    if n == 0 {
        return (0.0, 0.0, 0.0, 0.0, 0);
    }
    (
        r as f64 / n as f64,
        g as f64 / n as f64,
        b as f64 / n as f64,
        a as f64 / n as f64,
        n,
    )
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        bail!("usage: region-means <vanilla.png> <haste.png>");
    }
    let vanilla = load_png(Path::new(&args[0]))?;
    let haste = load_png(Path::new(&args[1]))?;
    if vanilla.dimensions() != haste.dimensions() {
        bail!(
            "size mismatch: {:?} vs {:?}",
            vanilla.dimensions(),
            haste.dimensions()
        );
    }
    let (w, h) = vanilla.dimensions();
    println!("{w}x{h}: region        vanilla-RGB      haste-RGB       delta-RGB  pixels");
    for &(name, f0, f1) in BANDS {
        let y0 = (f0 * h as f32) as u32;
        let y1 = ((f1 * h as f32) as u32).clamp(y0 + 1, h);
        let (vr, vg, vb, va, n) = band_means(&vanilla, y0, y1);
        let (hr, hg, hb, ha, _) = band_means(&haste, y0, y1);
        println!(
            "{name:>12}: ({vr:5.1},{vg:5.1},{vb:5.1})a{va:5.1} ({hr:5.1},{hg:5.1},{hb:5.1})a{ha:5.1} ({:+5.1},{:+5.1},{:+5.1}) {n}",
            hr - vr,
            hg - vg,
            hb - vb
        );
    }
    Ok(())
}
