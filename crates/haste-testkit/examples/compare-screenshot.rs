//! Compares a Haste screenshot with the vanilla reference one and writes a side-by-side +
//! difference PNG. Used by `scripts/compare-vanilla.sh`.
//!
//! usage: compare-screenshot <vanilla.png> <haste.png> <out.png> [tolerance]
//! Prints one line: `<percent differing> <mean error> <max error>`; the out PNG is
//! vanilla | haste | difference (red = over tolerance), left to right.

use anyhow::{bail, Result};
use haste_testkit::screenshots::{compare_screenshot, load_png};
use image::{GenericImage, Rgba, RgbaImage};
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        bail!("usage: compare-screenshot <vanilla.png> <haste.png> <out.png> [tolerance]");
    }
    let tolerance: u8 = args.get(3).map(|t| t.parse()).transpose()?.unwrap_or(8);
    let vanilla = load_png(Path::new(&args[0]))?;
    let haste = load_png(Path::new(&args[1]))?;
    let diff = compare_screenshot(&vanilla, &haste, tolerance)?;
    let (w, h) = vanilla.dimensions();
    let mut out = RgbaImage::from_pixel(w * 3, h, Rgba([0, 0, 0, 255]));
    out.copy_from(&vanilla, 0, 0)?;
    out.copy_from(&haste, w, 0)?;
    if let Some(d) = &diff.diff_image {
        out.copy_from(d, w * 2, 0)?;
    }
    out.save(&args[2])?;
    println!(
        "{:.2} {:.2} {}",
        diff.diff_fraction * 100.0,
        diff.mean_error,
        diff.max_error
    );
    Ok(())
}
