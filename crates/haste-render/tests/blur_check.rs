//! Dev-only check (ignored test): the two evidence screenshots from
//! `target/menu-blur-script.txt` — the pause screen (blurred world) and the chat screen
//! (sharp world) — must differ in local detail: a blurred region has a much smaller
/// Laplacian energy than a sharp one.
use image::GenericImageView;

#[test]
#[ignore = "dev evidence check: needs target/shots from the blur script run"]
fn pause_blur_reduces_local_detail() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap();
    let blurred = image::open(root.join("target/shots/script-pause-blurred.png")).unwrap();
    let sharp = image::open(root.join("target/shots/script-chat-not-blurred.png")).unwrap();
    assert_eq!(blurred.dimensions(), sharp.dimensions());

    // Laplacian energy over grayscale, one region away from the GUI (top-left quadrant of
    // the world area below the hotbar): sum of squared second differences.
    let laplacian = |img: &image::DynamicImage| -> f64 {
        let gray = img.to_luma8();
        let (w, h) = gray.dimensions();
        let mut sum = 0.0;
        let mut n = 0u64;
        for y in 1..h / 2 {
            for x in 1..w / 2 {
                let c = f64::from(gray.get_pixel(x, y)[0]);
                let l = f64::from(gray.get_pixel(x - 1, y)[0]);
                let r = f64::from(gray.get_pixel(x + 1, y)[0]);
                let u = f64::from(gray.get_pixel(x, y - 1)[0]);
                let d = f64::from(gray.get_pixel(x, y + 1)[0]);
                sum += (4.0 * c - l - r - u - d).powi(2);
                n += 1;
            }
        }
        sum / n as f64
    };
    let b = laplacian(&blurred);
    let s = laplacian(&sharp);
    eprintln!("laplacian energy: blurred {b:.1} sharp {s:.1}");
    assert!(
        b < s * 0.7,
        "the pause screen's world should be blurrier than the chat screen's: {b:.1} vs {s:.1}"
    );
}
