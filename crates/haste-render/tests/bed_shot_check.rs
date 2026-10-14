//! Checks an RCON-placed bed screenshot for bed-coloured pixels. The shot is produced by
//! `scripts/checks/beds.txt` (run by hand or run-feature-checks.sh): beds placed at (0,80,-6)
//! and (3,80,-6) on the test server, the player at (1.5,81,2) looking north. Not part of CI:
//! the PNG is local evidence.

use std::path::Path;

#[test]
#[ignore = "needs target/shots/script-beds.png from scripts/checks/beds.txt on the test server"]
fn bed_screenshot_has_bed_colours() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/script-beds.png");
    if !path.exists() {
        eprintln!("ignored: {} does not exist", path.display());
        return;
    }
    let img = image::open(&path).unwrap().to_rgba8();
    // The red bed's blanket texture is dominantly red (r much greater than g and b); the blue
    // bed's is blue. Count pixels clearly of each hue.
    let (mut red, mut blue) = (0u32, 0u32);
    for p in img.pixels() {
        let [r, g, b, _] = p.0;
        if r > 90 && r as i32 - g.max(b) as i32 > 40 {
            red += 1;
        }
        if b > 90 && b as i32 - r.max(g) as i32 > 40 {
            blue += 1;
        }
    }
    assert!(red > 500, "red bed pixels: {red}");
    assert!(blue > 500, "blue bed pixels: {blue}");
}
