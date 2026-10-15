//! Prints the RGBA of one pixel (and a small average around it) of a PNG.
//! usage: pixel-probe <png> <x> <y>

use anyhow::{bail, Result};
use haste_testkit::screenshots::load_png;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        bail!("usage: pixel-probe <png> <x> <y>");
    }
    let img = load_png(Path::new(&args[0]))?;
    let x: u32 = args[1].parse()?;
    let y: u32 = args[2].parse()?;
    let p = img.get_pixel(x, y);
    println!(
        "pixel ({x},{y}): rgba {}",
        p.0.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    // 5x5 mean around it
    let mut sum = [0u64; 4];
    let mut n = 0u64;
    for dy in -2i32..=2 {
        for dx in -2i32..=2 {
            let px = (x as i32 + dx).clamp(0, img.width() as i32 - 1) as u32;
            let py = (y as i32 + dy).clamp(0, img.height() as i32 - 1) as u32;
            let p = img.get_pixel(px, py);
            for (i, v) in p.0.iter().enumerate() {
                sum[i] += *v as u64;
            }
            n += 1;
        }
    }
    println!(
        "5x5 mean: {}",
        sum.iter()
            .map(|v| ((*v as f64 / n as f64).round() as u64).to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(())
}
