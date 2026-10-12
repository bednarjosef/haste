//! Screenshot comparison: per-pixel tolerance + percentage + diff image.

use std::path::Path;

use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};
use serde::Deserialize;

const KNOWN_VERSION: u32 = 1;

/// Camera/world metadata next to a recorded screenshot (FORMATS.md).
#[derive(Clone, Debug, Deserialize)]
pub struct ScreenshotMeta {
    pub version: u32,
    pub name: String,
    pub camera: CameraMeta,
    pub time_of_day: i64,
    pub render_distance: i32,
    #[serde(default = "true_")]
    pub gui_hidden: bool,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct CameraMeta {
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
}

fn true_() -> bool {
    true
}

impl ScreenshotMeta {
    pub fn load(path: &Path) -> Result<ScreenshotMeta> {
        let meta: ScreenshotMeta = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
        ))
        .with_context(|| format!("parsing {}", path.display()))?;
        if meta.version != KNOWN_VERSION {
            anyhow::bail!("{}: unsupported version {}", path.display(), meta.version);
        }
        Ok(meta)
    }
}

/// Result of comparing two screenshots.
#[derive(Clone, Debug)]
pub struct DiffResult {
    pub width: u32,
    pub height: u32,
    /// Pixels differing by more than the tolerance, as a fraction of all pixels (0..=1).
    pub diff_fraction: f64,
    /// Mean per-pixel channel error over all pixels (0..=255).
    pub mean_error: f64,
    /// Max per-pixel channel error (0..=255).
    pub max_error: u8,
    /// Red-highlighted diff image (same size); `None` when identical.
    pub diff_image: Option<RgbaImage>,
}

/// Compares two screenshots pixel by pixel.
///
/// A pixel "differs" when any channel is off by more than `tolerance` (0..=255). Alpha is
/// compared too (GUI-off captures should be opaque everywhere). The diff image is white where
/// pixels are within tolerance and red-channel-scaled where they differ.
pub fn compare_screenshot(
    a: &image::RgbaImage,
    b: &image::RgbaImage,
    tolerance: u8,
) -> Result<DiffResult> {
    let (aw, ah) = a.dimensions();
    let (bw, bh) = b.dimensions();
    if (aw, ah) != (bw, bh) {
        anyhow::bail!("size mismatch: {aw}x{ah} vs {bw}x{bh}");
    }
    let mut diff = RgbaImage::new(aw, ah);
    let mut differing = 0u64;
    let mut sum_error = 0u64;
    let mut max_error = 0u8;
    for ((x, y, pa), (_, _, pb)) in a.enumerate_pixels().zip(b.enumerate_pixels()) {
        let err = channel_error(pa, pb);
        sum_error += err as u64;
        max_error = max_error.max(err);
        if err > tolerance {
            differing += 1;
            diff.put_pixel(x, y, Rgba([255, 0, 0, 255]));
        } else {
            diff.put_pixel(x, y, Rgba([err, err, err, 255]));
        }
    }
    let total = aw as u64 * ah as u64;
    Ok(DiffResult {
        width: aw,
        height: ah,
        diff_fraction: differing as f64 / total as f64,
        mean_error: sum_error as f64 / total as f64,
        max_error,
        diff_image: if differing == 0 { None } else { Some(diff) },
    })
}

/// Max per-channel absolute difference between two RGBA pixels (alpha included).
fn channel_error(a: &Rgba<u8>, b: &Rgba<u8>) -> u8 {
    (0..4)
        .map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs())
        .max()
        .unwrap_or(0) as u8
}

/// Loads a PNG from disk.
pub fn load_png(path: &Path) -> Result<image::RgbaImage> {
    let img = image::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(img.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: u8) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([c, c, c, 255]))
    }

    #[test]
    fn identical_images_have_no_diff() {
        let a = solid(16, 16, 128);
        let b = solid(16, 16, 128);
        let d = compare_screenshot(&a, &b, 0).unwrap();
        assert_eq!(d.diff_fraction, 0.0);
        assert!(d.diff_image.is_none());
        assert_eq!(d.max_error, 0);
    }

    #[test]
    fn within_tolerance_is_not_a_diff() {
        let a = solid(4, 4, 100);
        let b = solid(4, 4, 105);
        let d = compare_screenshot(&a, &b, 5).unwrap();
        assert_eq!(d.diff_fraction, 0.0);
        assert_eq!(d.max_error, 5);
        // one over tolerance
        let d = compare_screenshot(&a, &b, 4).unwrap();
        assert_eq!(d.diff_fraction, 1.0);
        assert!(d.diff_image.is_some());
    }

    #[test]
    fn partial_diff_counts_fraction() {
        let mut a = solid(2, 2, 0);
        let mut b = solid(2, 2, 0);
        b.put_pixel(1, 0, Rgba([255, 255, 255, 255]));
        a.put_pixel(1, 0, Rgba([0, 0, 0, 255]));
        let d = compare_screenshot(&a, &b, 10).unwrap();
        assert!((d.diff_fraction - 0.25).abs() < 1e-9);
        let img = d.diff_image.unwrap();
        assert_eq!(img.get_pixel(0, 0), &Rgba([0, 0, 0, 255]));
        assert_eq!(img.get_pixel(1, 0), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn size_mismatch_is_an_error() {
        let a = solid(4, 4, 0);
        let b = solid(8, 8, 0);
        assert!(compare_screenshot(&a, &b, 0).is_err());
    }

    #[test]
    fn mean_error_over_all_pixels() {
        let a = solid(2, 1, 0);
        let mut b = solid(2, 1, 0);
        b.put_pixel(0, 0, Rgba([100, 0, 0, 255]));
        let d = compare_screenshot(&a, &b, 0).unwrap();
        assert_eq!(d.mean_error, 50.0);
        assert_eq!(d.max_error, 100);
    }
}
