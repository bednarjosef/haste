//! Checks an RCON-placed banner screenshot for banner pixels. The shot is produced by
//! `scripts/checks/banners.txt`: a red patterned standing banner at (-3,80,-3), a blue
//! standing banner at (-5,80,-3) and a white wall banner at (-6,80,-3), the player at
//! (-2.5,81,2) looking north. Not part of CI: the PNG is local evidence.

use std::path::Path;

#[test]
#[ignore = "needs target/shots/script-banners.png from scripts/checks/banners.txt on the test server"]
fn banner_screenshot_has_banner_colours() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/script-banners.png");
    if !path.exists() {
        eprintln!("ignored: {} does not exist", path.display());
        return;
    }
    let img = image::open(&path).unwrap().to_rgba8();
    // The banners occupy distinct hues: the red base (r dominant), the blue standing banner
    // (b dominant) and the white wall banner's near-white flag.
    let (mut red, mut blue, mut white) = (0u32, 0u32, 0u32);
    for p in img.pixels() {
        let [r, g, b, _] = p.0;
        if r > 90 && r as i32 - g.max(b) as i32 > 40 {
            red += 1;
        }
        if b > 90 && b as i32 - r.max(g) as i32 > 40 {
            blue += 1;
        }
        if r > 200 && g > 200 && b > 200 {
            white += 1;
        }
    }
    // The red banner sits behind the sign from this angle, so only its top shows.
    assert!(red > 50, "red banner pixels: {red}");
    assert!(blue > 500, "blue banner pixels: {blue}");
    assert!(white > 200, "white wall banner pixels: {white}");
}
