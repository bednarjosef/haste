//! Checks an RCON-placed skull screenshot for head pixels. The shot is produced by
//! `scripts/checks/skulls.txt`: skeleton, creeper, zombie and dragon heads at y 80 (z -3),
//! a piglin head at (5,81,-5) and a player wall head at (1,81,-4), the player at (3.5,81,2)
//! looking north-west. Not part of CI: the PNG is local evidence.

use std::path::Path;

#[test]
#[ignore = "needs target/shots/script-skulls.png from scripts/checks/skulls.txt on the test server"]
fn skull_screenshot_has_head_pixels() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/script-skulls.png");
    if !path.exists() {
        eprintln!("ignored: {} does not exist", path.display());
        return;
    }
    let img = image::open(&path).unwrap().to_rgba8();
    // The skeleton skull is light grey-bone (all channels high and close together, but not
    // the white cloud); the creeper head's face is green-dominant. Both must appear.
    let (mut bone, mut green) = (0u32, 0u32);
    for p in img.pixels() {
        let [r, g, b, _] = p.0;
        let lo = r.min(g).min(b) as i32;
        let hi = r.max(g).max(b) as i32;
        if lo > 100 && hi - lo < 30 && hi < 235 {
            bone += 1;
        }
        if g > 60 && g as i32 - r as i32 > 15 && g as i32 - b as i32 > 15 {
            green += 1;
        }
    }
    assert!(bone > 300, "bone-coloured skull pixels: {bone}");
    assert!(green > 150, "creeper green pixels: {green}");
}
