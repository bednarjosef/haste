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
    // A coarse 8x8 grid of the mean per-channel delta per cell, to localize the worst
    // areas (row 0 = top of the image).
    if args.get(2).map(|a| a == "--grid").unwrap_or(false) {
        let g = 8u32;
        println!("grid: mean delta-R per cell (rows top to bottom)");
        for gy in 0..g {
            let mut row = String::new();
            for gx in 0..g {
                let x0 = gx * w / g;
                let x1 = (gx + 1) * w / g;
                let y0 = gy * h / g;
                let y1 = (gy + 1) * h / g;
                let (vr, vg, vb, _, _) = grid_means(&vanilla, x0, x1, y0, y1);
                let (hr, hg, hb, _, _) = grid_means(&haste, x0, x1, y0, y1);
                let d = ((hr - vr) + (hg - vg) + (hb - vb)) / 3.0;
                row.push_str(&format!("{d:+6.1}"));
            }
            println!("{row}");
        }
    }
    // The haste image's dominant colour per grid cell: `blue > red + 30 and blue >
    // green + 20` counts as "blue-ish" (water/sky-like), reported as a fraction of the
    // cell. To find where unexpected blue comes from.
    if args.get(2).map(|a| a == "--blue").unwrap_or(false) {
        let g = 16u32;
        println!("blue-ish fraction per cell (vanilla | haste), rows top to bottom");
        for gy in 0..g {
            let mut row = String::new();
            for gx in 0..g {
                let x0 = gx * w / g;
                let x1 = (gx + 1) * w / g;
                let y0 = gy * h / g;
                let y1 = (gy + 1) * h / g;
                let bv = blue_fraction(&vanilla, x0, x1, y0, y1);
                let bh = blue_fraction(&haste, x0, x1, y0, y1);
                row.push_str(&format!(" {:>3.0}/{:<3.0}", bv * 100.0, bh * 100.0));
            }
            println!("{row}");
        }
    }
    Ok(())
}

fn blue_fraction(img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32) -> f64 {
    let mut n = 0u64;
    let mut blue = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y);
            if p[2] as i32 > p[0] as i32 + 30 && p[2] as i32 > p[1] as i32 + 20 {
                blue += 1;
            }
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        blue as f64 / n as f64
    }
}

fn grid_means(
    img: &image::RgbaImage,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
) -> (f64, f64, f64, f64, u64) {
    band_means_rect(img, x0, x1, y0, y1)
}

fn band_means_rect(
    img: &image::RgbaImage,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
) -> (f64, f64, f64, f64, u64) {
    let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
    let mut n = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
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
