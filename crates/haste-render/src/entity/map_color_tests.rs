//! Tests for the `MapColor` palette: the packed-id decoding against hand-checked values,
//! and the whole table verified against vanilla's own constant list in the decompiled
//! 26.3 source when the cache has it (an oracle test, like the recorder mod's dumps).

use super::map_color::color_from_packed_id;

/// NONE (id 0) is fully transparent at every brightness.
#[test]
fn none_is_transparent() {
    for brightness in 0..4u8 {
        assert_eq!(color_from_packed_id(brightness), [0, 0, 0, 0]);
    }
}

/// `ARGB.scaleRGB(color, modifier)`: `channel * modifier / 255`, truncated — stone
/// (112, 112, 112) at NORMAL (220) is 96, not the rounded 97.
#[test]
fn brightness_truncates_like_argb_scalergb() {
    // STONE is id 11; brightness NORMAL is 1, HIGH 2, LOW 0.
    assert_eq!(color_from_packed_id(11 << 2 | 1), [96, 96, 96, 255]);
    // HIGH (2) leaves the colour as it is.
    assert_eq!(color_from_packed_id(11 << 2 | 2), [112, 112, 112, 255]);
    // LOW (0) is 180/255.
    assert_eq!(color_from_packed_id(11 << 2), [79, 79, 79, 255]);
}

/// The palette and brightness list against vanilla's `MapColor` in the decompiled source
/// (the constants are read from the cache; the test skips when the source is not there).
#[test]
#[ignore = "needs the decompiled 26.3 client source in the cache"]
fn table_matches_vanilla_source() {
    let source = haste_core::paths::cache_dir()
        .join("source/26.3/client/net/minecraft/world/level/material/MapColor.java");
    let Ok(text) = std::fs::read_to_string(&source) else {
        eprintln!("ignored: no decompiled MapColor.java in the cache");
        return;
    };
    // Vanilla's field list: `public static final MapColor NAME = new MapColor(id, col);`.
    let mut colors = [0u32; 64];
    let mut found = 0;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("public static final MapColor ") else {
            continue;
        };
        let Some(args) = rest.split("new MapColor(").nth(1) else {
            continue;
        };
        let mut parts = args.trim_end_matches(");").split(',');
        let (Some(id), Some(col)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (id, col): (usize, u32) = match (id.trim().parse(), col.trim().parse()) {
            (Ok(id), Ok(col)) => (id, col),
            _ => continue,
        };
        assert!(id < 64, "MapColor id {id} out of range");
        colors[id] = col;
        found += 1;
    }
    assert_eq!(found, 62, "vanilla defines 62 map colours");
    for (id, (&vanilla, ours)) in colors.iter().zip(super::map_color::COLORS).enumerate() {
        assert_eq!(vanilla, ours, "MapColor {id}");
    }
    // The brightness modifiers, from `Brightness`'s own list (`LOW(0, 180)` and friends).
    for (id, modifier) in [(0usize, 180), (1, 220), (2, 255), (3, 135)] {
        assert!(
            text.contains(&format!("({id}, {modifier})")),
            "brightness {id} -> {modifier} not found"
        );
    }
}
