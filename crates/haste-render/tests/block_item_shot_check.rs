//! Checks an RCON-placed block entity screenshot for item pixels. The shot comes from
//! `scripts/checks/block-items.txt`: a campfire with four items (diamond, gold ingot,
//! emerald, iron ingot) at (9,71,10), a spawner holding a pig at (11,71,10), a vault with a
//! stack of gold blocks at (7,71,10), a shelf with a book, an apple and bread at (9,71,12)
//! and a brushed suspicious sand with an emerald at (11,71,12); the player stands at
//! (9.5,71,6.5) looking south at pitch 10. Not part of CI: the PNG is local evidence.

use std::path::Path;

#[test]
#[ignore = "needs target/shots/script-block-items.png from scripts/checks/block-items.txt on the test server"]
fn block_items_screenshot_has_item_pixels() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots/script-block-items.png");
    if !path.exists() {
        eprintln!("ignored: {} does not exist", path.display());
        return;
    }
    let img = image::open(&path).unwrap().to_rgba8();
    let (w, h) = (img.width(), img.height());
    let (mut cyan, mut yellow, mut pink, mut green, mut grey) = (0u32, 0u32, 0u32, 0u32, 0u32);
    // The camera looks down over a floor, so every item sprite is in the lower half: the sky
    // above is not an item colour and is skipped.
    for y in (h / 2)..h {
        for x in 0..w {
            let [r, g, b, _] = img.get_pixel(x, y).0;
            let (r, g, b) = (r as i32, g as i32, b as i32);
            // The campfire's diamond: cyan, green and blue far above its red.
            if g > r + 60 && b > r + 60 && g > 100 {
                cyan += 1;
            }
            // The vault's gold blocks and the campfire's gold ingot.
            if r > 120 && g > 100 && r - g > 20 && g - b > 40 {
                yellow += 1;
            }
            // The spawner's pig: pink (red well above green, blue above green).
            if r > 130 && r - g > 40 && b > g {
                pink += 1;
            }
            // The shelf's and the suspicious sand's emeralds, and the campfire's emerald.
            if g > 60 && g - r > 30 && g - b > 30 {
                green += 1;
            }
            if r > 90 && g > 90 && b > 90 && (r - g).abs() < 18 && (g - b).abs() < 18 {
                grey += 1;
            }
        }
    }
    assert!(
        cyan > 40 && yellow > 100 && pink > 30 && green > 30 && grey > 40,
        "cyan {cyan}, yellow {yellow}, pink {pink}, green {green}, grey {grey}"
    );
}
