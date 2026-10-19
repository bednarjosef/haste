//! Tests for the map renderer's pure parts: the framed map pose, the two-handed tilt, the
//! decoration pose, and the `MapDecorationTypes` table against the decompiled source.

use super::map::{framed_map_transform, map_tilt, MapMarker};
use super::*;

/// The framed map pose: rotation `% 4` quarter turns, the 180-degree flip, the 1/128 pixel
/// scale, the centre origin, and one pixel back. A point at the map's centre stays at the
/// frame's item plane minus 1/128 in z (however the frame turns).
#[test]
fn framed_map_pose_centres_and_pushes_back() {
    let m = framed_map_transform(0, false);
    // The map's centre (64, 64): at the origin, 1.01 map pixels (the contents' z and the
    // push-back translate, both through the 1/128 scale) behind the 0.4375 plane.
    let centre = m.transform_point3(Vec3::new(64.0, 64.0, -0.01));
    assert!((centre.x).abs() < 1e-5, "{}", centre.x);
    assert!((centre.y).abs() < 1e-5, "{}", centre.y);
    assert!(
        (centre.z - (0.4375 - 1.01 / 128.0)).abs() < 1e-5,
        "{}",
        centre.z
    );
    // Rotation 1 is one quarter turn (90 degrees) of the contents; rotation 5 is the same
    // quarter turn (`rotation % 4`).
    let (r1, r5) = (
        framed_map_transform(1, false),
        framed_map_transform(5, false),
    );
    let p = Vec3::new(64.0, 100.0, 0.0);
    let (a, b) = (r1.transform_point3(p), r5.transform_point3(p));
    assert!((a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5);
    // An invisible frame's plane sits flush against the wall (0.5, like the item).
    let invisible = framed_map_transform(0, true);
    let z = invisible.transform_point3(Vec3::new(0.0, 0.0, 0.0)).z;
    assert!((z - (0.5 - 1.0 / 128.0)).abs() < 1e-5, "{z}");
}

/// `calculateMapTilt`: a level look (pitch 0) is the full tilt, straight up pins it at 1
/// and straight down at 0 (the input `1 - xRot/45 + 0.1` clamped over the eased cosine);
/// pitch 27 degrees is the ease's middle.
#[test]
fn map_tilt_eases_over_the_pitch() {
    assert!((map_tilt(0.0) - 1.0).abs() < 1e-5);
    assert!((map_tilt(-90.0) - 1.0).abs() < 1e-5);
    assert!((map_tilt(90.0)).abs() < 1e-5);
    assert!((map_tilt(27.0) - 0.5).abs() < 1e-4);
}

/// A decoration icon's pose (`MapRenderer.render`): at the map position `x/2+64, y/2+64`,
/// an 8-pixel icon around it. The marker x=-128 (clampMapCoordinate's edge) lands at the
/// map's left edge.
#[test]
fn decoration_pose_places_the_icon() {
    let d = MapMarker {
        kind: 0,
        x: -128,
        y: 0,
        rot: 0,
        name: None,
    };
    let m = Mat4::IDENTITY
        * Mat4::from_translation(Vec3::new(
            d.x as f32 / 2.0 + 64.0,
            d.y as f32 / 2.0 + 64.0,
            -0.02,
        ))
        * Mat4::from_rotation_z((d.rot as f32 * 360.0 / 16.0).to_radians())
        * Mat4::from_scale(Vec3::new(4.0, 4.0, 3.0))
        * Mat4::from_translation(Vec3::new(-0.125, 0.125, 0.0));
    // The icon quad spans 8 map pixels around its centre; vanilla's (-0.125, 0.125)
    // translate comes before the scale, so it lands at half a pixel.
    let tl = m.transform_point3(Vec3::new(-1.0, 1.0, 0.0));
    let br = m.transform_point3(Vec3::new(1.0, -1.0, 0.0));
    assert!((tl.x + 4.5).abs() < 1e-4, "{}", tl.x);
    assert!((tl.y - 68.5).abs() < 1e-4, "{}", tl.y);
    assert!((br.x - 3.5).abs() < 1e-4, "{}", br.x);
    assert!((br.y - 60.5).abs() < 1e-4, "{}", br.y);
}

/// The vanilla `MapDecorationTypes` table against the decompiled source (the registry
/// names, sprite assets and item-frame flags; skips when the source is not in the cache).
#[test]
#[ignore = "needs the decompiled 26.3 client source in the cache"]
fn decoration_types_match_vanilla_source() {
    let source = haste_core::paths::cache_dir().join(
        "source/26.3/client/net/minecraft/world/level/saveddata/maps/MapDecorationTypes.java",
    );
    let Ok(text) = std::fs::read_to_string(&source) else {
        eprintln!("ignored: no decompiled MapDecorationTypes.java in the cache");
        return;
    };
    let mut count = 0;
    for line in text.lines() {
        let Some(rest) = line
            .trim()
            .strip_prefix("public static final Holder<MapDecorationType> ")
        else {
            continue;
        };
        // `NAME = register("name", "asset", showOnItemFrame, trackCount);`
        let Some(args) = rest.split("register(").nth(1) else {
            continue;
        };
        let parts: Vec<&str> = args
            .trim_end_matches(");")
            .split(',')
            .map(str::trim)
            .collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[0].trim_matches('"');
        let asset = parts[1].trim_matches('"');
        let on_frame = parts[2].parse::<bool>().unwrap();
        let key = Identifier::minecraft(name);
        let t = super::map_decoration_types::by_registry_name(&key)
            .unwrap_or_else(|| panic!("map decoration type {name} missing from the table"));
        assert_eq!(t.asset, asset, "{name}'s sprite");
        assert_eq!(t.show_on_frame, on_frame, "{name}'s item-frame flag");
        count += 1;
    }
    assert_eq!(count, 40, "vanilla defines 40 map decoration types");
    // A type we do not know resolves to nothing (draws nothing, never panics).
    let unknown = Identifier::parse("mod:custom").expect("parses");
    assert!(super::map_decoration_types::by_registry_name(&unknown).is_none());
}
