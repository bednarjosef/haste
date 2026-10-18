// Tests for the block entity extraction's parsing (moved from app.rs with the code).

use super::banner::parse_banner_nbt;
use super::gateway::gateway_beam_at;
use super::sign::{empty_sign_side, first_wrapped_line, parse_sign_nbt};
use super::*;

/// A fixed-width "font" for the wrap tests: every character is 6 wide, `i` is 2.
struct WrapFont;
impl haste_core::gui::TextMetrics for WrapFont {
    fn width(&self, text: &str) -> i32 {
        text.chars().map(|c| if c == 'i' { 2 } else { 6 }).sum()
    }
}

/// The first wrapped line follows vanilla `StringSplitter.splitLines`: break at the last
/// space before the width runs out, hard-cut without one, always keep the first character.
#[test]
fn first_wrapped_line_breaks_at_spaces() {
    let f = &WrapFont;
    // 3 characters of 6 = 18 fits 20.
    assert_eq!(first_wrapped_line("abc", f, 20), "abc");
    // 4 characters of 6 = 24 > 20: no space, cut before the 4th.
    assert_eq!(first_wrapped_line("abcd", f, 20), "abc");
    // A space before the cut: the line ends at the space (dropped).
    assert_eq!(first_wrapped_line("ab cd", f, 20), "ab");
    // Newlines end the line.
    assert_eq!(first_wrapped_line("ab\ncd", f, 100), "ab");
    // The first character is kept even when wider than the limit.
    assert_eq!(first_wrapped_line("abcd", f, 1), "a");
    // Empty stays empty.
    assert_eq!(first_wrapped_line("", f, 90), "");
}

/// Sign NBT parses to both sides' lines, colour and glow; the plain and the compound
/// component forms both work, and missing sides are `SignText.EMPTY`.
#[test]
fn sign_nbt_parses_both_sides() {
    use haste_protocol::nbt::serializer::{NbtWriteHelper, NbtWriteHelperJava};
    use haste_protocol::nbt::{NbtCompound, NbtTag, COMPOUND_ID};
    let mut front = NbtCompound::new();
    front.put_list(
        "messages",
        vec![
            NbtTag::String("hello".into()),
            NbtTag::Compound({
                let mut c = NbtCompound::new();
                c.put_string("text", "world".into());
                c
            }),
            NbtTag::String("".into()),
            NbtTag::String("line4".into()),
        ],
    );
    front.put_string("color", "red".into());
    front.put_bool("has_glowing_text", true);
    let mut root = NbtCompound::new();
    root.put_compound("front_text", front);
    // Network NBT: the type byte and payload with no root name.
    let mut bytes = Vec::new();
    {
        let mut w = NbtWriteHelperJava::new(&mut bytes);
        w.write_u8(COMPOUND_ID).unwrap();
        root.clone().serialize_content(&mut w).unwrap();
    }
    let parsed = parse_sign_nbt(&bytes, &WrapFont, 90).unwrap();
    assert_eq!(parsed.front.lines[0], "hello");
    assert_eq!(parsed.front.lines[1], "world");
    assert_eq!(parsed.front.lines[3], "line4");
    assert_eq!(parsed.front.color, 14);
    assert!(parsed.front.glowing);
    // The back is missing: vanilla's empty default.
    assert_eq!(parsed.back, empty_sign_side());
    // Not NBT at all: None, never a panic.
    assert!(parse_sign_nbt(&[1, 2, 3], &WrapFont, 90).is_none());
}

/// Banner NBT: the `patterns` list (registry id or inline `{asset_id}` form) and the base
/// colour from the block id; a banner without patterns is the plain base.
#[test]
fn banner_nbt_parses_patterns_and_base() {
    use haste_protocol::nbt::serializer::{NbtWriteHelper, NbtWriteHelperJava};
    use haste_protocol::nbt::{NbtCompound, NbtTag, COMPOUND_ID};
    let nbt_bytes = |root: NbtCompound| {
        let mut bytes = Vec::new();
        let mut w = NbtWriteHelperJava::new(&mut bytes);
        w.write_u8(COMPOUND_ID).unwrap();
        root.clone().serialize_content(&mut w).unwrap();
        bytes
    };
    // A red wall banner with two patterns.
    let mut root = NbtCompound::new();
    root.put_list(
        "patterns",
        vec![
            NbtTag::Compound({
                let mut c = NbtCompound::new();
                c.put_string("pattern", "minecraft:stripe_bottom".into());
                c.put_string("color", "blue".into());
                c
            }),
            NbtTag::Compound({
                let mut c = NbtCompound::new();
                c.put_string("pattern", "minecraft:half_vertical".into());
                c.put_string("color", "white".into());
                c
            }),
        ],
    );
    let block = haste_core::Identifier::minecraft("red_wall_banner");
    let data = parse_banner_nbt(&nbt_bytes(root), &block).unwrap();
    assert_eq!(data.base, 14);
    assert_eq!(data.layers.len(), 2);
    assert_eq!(data.layers[0].pattern.path(), "stripe_bottom");
    assert_eq!(data.layers[0].color, 11);
    assert_eq!(data.layers[1].pattern.path(), "half_vertical");
    assert_eq!(data.layers[1].color, 0);
    // No patterns: the plain base (black from `minecraft:banner`).
    let plain = parse_banner_nbt(
        &nbt_bytes(NbtCompound::new()),
        &haste_core::Identifier::minecraft("banner"),
    )
    .unwrap();
    assert_eq!(plain.base, 15);
    assert!(plain.layers.is_empty());
    // The inline asset_id form.
    let mut inline = NbtCompound::new();
    inline.put_list(
        "patterns",
        vec![NbtTag::Compound({
            let mut c = NbtCompound::new();
            c.put_string("asset_id", "minecraft:cross".into());
            c.put_string("color", "lime".into());
            c
        })],
    );
    let data = parse_banner_nbt(
        &nbt_bytes(inline),
        &haste_core::Identifier::minecraft("white_banner"),
    )
    .unwrap();
    assert_eq!(data.layers.len(), 1);
    assert_eq!(data.layers[0].pattern.path(), "cross");
    assert_eq!(data.layers[0].color, 5);
}

/// The gateway beam matches vanilla `TheEndGatewayRenderer.extractRenderState`: hidden from
/// age 200, `sin(clamp((age + partial) / 200, 0, 1) * PI)` scaled to the build limit, magenta.
#[test]
fn gateway_beam_matches_vanilla() {
    // Age 0, partial 0: sin(0) = 0, height 0.
    let b = gateway_beam_at(0, 0.0, 319).unwrap();
    assert_eq!(b.height, 0);
    assert_eq!(b.color, haste_render::blockentity::DYE_DIFFUSE_COLOR[2]);
    // Age 100: exactly half way, sin(PI/2) = 1: the full build limit.
    let b = gateway_beam_at(100, 0.0, 319).unwrap();
    assert!((b.scale - 1.0).abs() < 1e-6);
    assert_eq!(b.height, 319);
    // Age 200 and beyond: no beam.
    assert!(gateway_beam_at(200, 0.0, 319).is_none());
    assert!(gateway_beam_at(1000, 0.5, 319).is_none());
    // The partial tick moves inside the tick's span only.
    let half = gateway_beam_at(0, 0.5, 319).unwrap();
    assert!(half.scale > 0.0 && half.scale < gateway_beam_at(1, 0.0, 319).unwrap().scale);
}

/// The partial fold leaves per-tick data alone and lerps only the animatable fields.
#[test]
fn fold_partial_lerps_animatable_fields_only() {
    let mut tick = BlockEntityTickFrame::default();
    tick.instances.push(haste_render::BlockEntityInstance {
        pos: BlockPos::new(0, 0, 0),
        facing: Direction::South,
        half: haste_render::ChestHalf::Single,
        kind: haste_render::BlockEntityKind::Chest(haste_render::ChestMaterial::Normal),
        open: haste_render::TickPair {
            start: 0.0,
            end: 1.0,
        },
        light: (15, 0),
        sign_angle: 0.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: haste_render::TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        beam_time: 0.0,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    });
    tick.instances.push(haste_render::BlockEntityInstance {
        kind: haste_render::BlockEntityKind::Banner { wall: false },
        anim: Some(haste_render::BlockEntityAnim::Bell {
            ticks: haste_render::TickPair {
                start: 0.0,
                end: 2.0,
            },
            click: None,
        }),
        ..tick.instances[0].clone()
    });
    let mut out = Vec::new();
    fold_partial(&tick, 0.5, &mut out);
    // The chest's lid pair is per-tick data the renderer combines itself: unchanged.
    assert_eq!(out[0].open.end, 1.0);
    // The banner's phase gains the half partial tick over 100, the bell's anim is lerped.
    assert!((out[1].banner_phase - 0.005).abs() < 1e-6);
    assert!(matches!(
        out[1].anim,
        Some(haste_render::BlockEntityAnim::Bell {
            ticks: haste_render::TickPair {
                start: 0.0,
                end: 1.0
            },
            ..
        })
    ));
}
