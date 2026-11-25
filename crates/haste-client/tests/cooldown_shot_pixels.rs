//! The on-screen check's pixel evidence for the item cooldown overlay (ignored: needs the
//! shots the pearl check script saved to `target/shots/`): the hotbar's white
//! half-transparent sweep (`GuiGraphicsExtractor.itemCooldown`) whitens the pearl's icon
//! while the cooldown runs. The shots were taken around 80% (`cooldown-early`), 50%
//! (`cooldown-mid`) and after the end (`cooldown-done`, the plain icon) of an ender pearl's
//! 20 ticks, so the pixels the sweep lightens must cover most of the icon early on, about
//! half in the middle and none at the end.

use image::{DynamicImage, GenericImageView};

fn shot(name: &str) -> DynamicImage {
    let path = format!(
        "{}/../../target/shots/script-{name}.png",
        env!("CARGO_MANIFEST_DIR")
    );
    image::open(&path).unwrap_or_else(|e| panic!("the check's shot {path}: {e}"))
}

/// How many pixels of the first hotbar slot's 16x16 icon the sweep lightened: the count of
/// pixels brighter in `shot` than in the reference (the cooldown-over shot) by a margin, in
/// the icon's rectangle (vanilla's hotbar layout: the icon sits at guiW/2 - 88, guiH - 19 of
/// a 320x240-class GUI, at the window's GUI scale).
fn lightened(shot: &DynamicImage, reference: &DynamicImage) -> u64 {
    let (w, h) = shot.dimensions();
    let s = h / 270; // the GUI scale of a 480x270 GUI (auto at the default 960x540)
    let gui_w = w / s;
    let x0 = (gui_w / 2 - 88) * s;
    let y0 = (h / s - 19) * s;
    let side = 16 * s;
    let mut n = 0;
    for dy in 0..side {
        for dx in 0..side {
            let (x, y) = (x0 + dx, y0 + dy);
            let a = shot.get_pixel(x, y).0;
            let b = reference.get_pixel(x, y).0;
            if a[0] as i32 - b[0] as i32 > 24
                && a[1] as i32 - b[1] as i32 > 24
                && a[2] as i32 - b[2] as i32 > 24
            {
                n += 1;
            }
        }
    }
    n
}

#[test]
#[ignore = "needs target/shots/script-cooldown-*.png from the pearl check script"]
fn the_sweep_recedes_as_the_cooldown_runs_out() {
    let early = shot("cooldown-early");
    let mid = shot("cooldown-mid");
    let done = shot("cooldown-done");
    let side = 16 * (early.dimensions().1 / 270);
    let total = u64::from(side) * u64::from(side);
    let early = lightened(&early, &done);
    let mid = lightened(&mid, &done);
    // The sweep at ~80% of the cooldown covers most of the icon; half-way about half (the
    // vanilla floor/ceil split covers the bottom rows); none at the end (the reference).
    assert!(
        early * 2 > total,
        "early {early} of {total}: the sweep covers most of the icon"
    );
    assert!(mid > total / 4, "mid {mid} of {total}: about half");
    assert!(early > mid, "early {early} covers more than mid {mid}");
}
