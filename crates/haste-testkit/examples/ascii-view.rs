//! Prints a coarse ASCII rendering of a PNG: each cell is the mean colour of a block of
//! pixels, classified into a letter (sky-blue, green, brown/dirt, dark, bright). For
//! discussing screenshots without opening them.
//! usage: ascii-view <png> [cols] [rows]

use anyhow::{bail, Result};
use haste_testkit::screenshots::load_png;
use std::path::Path;

fn classify(r: u8, g: u8, b: u8) -> char {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let lum = (r + g + b) / 3;
    if b > r + 25 && b > g + 15 {
        if lum > 130 {
            'S' // bright blue (sky)
        } else {
            'w' // darker blue (water-ish)
        }
    } else if g > r + 10 && g > b + 10 {
        'g' // green
    } else if r > g && g > b && r - b > 15 {
        'd' // brown/dirt
    } else if lum < 45 {
        '#' // dark
    } else if lum > 180 {
        ' ' // bright
    } else {
        '.' // neutral
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        bail!("usage: ascii-view <png> [cols] [rows]");
    }
    let img = load_png(Path::new(&args[0]))?;
    let cols: u32 = args.get(1).map(|v| v.parse()).transpose()?.unwrap_or(110);
    let rows: u32 = args.get(2).map(|v| v.parse()).transpose()?.unwrap_or(34);
    let (w, h) = img.dimensions();
    for cy in 0..rows {
        let mut line = String::with_capacity(cols as usize);
        for cx in 0..cols {
            let x0 = cx * w / cols;
            let x1 = ((cx + 1) * w / cols).max(x0 + 1);
            let y0 = cy * h / rows;
            let y1 = ((cy + 1) * h / rows).max(y0 + 1);
            let (mut sr, mut sg, mut sb, mut n) = (0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = img.get_pixel(x, y);
                    sr += p[0] as u64;
                    sg += p[1] as u64;
                    sb += p[2] as u64;
                    n += 1;
                }
            }
            line.push(classify((sr / n) as u8, (sg / n) as u8, (sb / n) as u8));
        }
        println!("{line}");
    }
    Ok(())
}
