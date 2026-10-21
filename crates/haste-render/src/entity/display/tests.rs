//! The display renderers' numbers against vanilla `DisplayRenderer` and its subclasses, and
//! the text splitting and flattening the text display relies on.

use std::sync::Arc;

use glam::{DVec3, Mat4, Vec3, Vec4};
use haste_assets::TextFormat;
use haste_core::display::{
    Billboard, DisplayContent, DisplayDecoration, ItemDisplayContext, TextDisplayDecoration,
};
use haste_core::Component;

use super::block::block_display_light;
use super::item::{item_display_for, item_display_turn};
use super::text::{background_color, line_offset, shadow_color, text_color, text_pose};
use super::{orientation, should_render};
use crate::entity::ItemDisplay;
use crate::gui::{string_decomposer, string_splitter};

fn close(a: Mat4, b: Mat4) -> bool {
    a.abs_diff_eq(b, 1e-5)
}

fn deg(d: f32) -> f32 {
    d.to_radians()
}

#[test]
fn fixed_billboard_turns_by_the_entity_rotation() {
    // `rotationYXZ(-yRot, xRot, 0)` of the entity's own rotation.
    let q = orientation(Billboard::Fixed, (90.0, 30.0), (12.0, -40.0));
    let expected = Mat4::from_rotation_y(deg(-90.0)) * Mat4::from_rotation_x(deg(30.0));
    assert!(close(Mat4::from_quat(q), expected));
}

#[test]
fn billboards_take_the_camera_rotation_on_their_free_axes() {
    let entity = (90.0, 30.0);
    let camera = (10.0, 20.0);
    // `transformYRot = cameraYRot - 180`, `transformXRot = -cameraXRot`.
    let (cy, cx) = (10.0 - 180.0, -20.0);
    let cases = [
        (Billboard::Horizontal, 90.0, cx),
        (Billboard::Vertical, cy, 30.0),
        (Billboard::Center, cy, cx),
    ];
    for (billboard, y, x) in cases {
        let q = orientation(billboard, entity, camera);
        let expected = Mat4::from_rotation_y(deg(-y)) * Mat4::from_rotation_x(deg(x));
        assert!(close(Mat4::from_quat(q), expected), "{billboard:?}");
    }
}

/// The camera's look direction for `Camera.yRot()` / `xRot()` (degrees): yaw 0 looks south
/// (+Z), positive pitch looks down.
fn look(y_rot: f32, x_rot: f32) -> Vec3 {
    let (y, x) = (deg(y_rot), deg(x_rot));
    Vec3::new(-y.sin() * x.cos(), -x.sin(), y.cos() * x.cos())
}

#[test]
fn center_billboarded_text_faces_the_camera() {
    // The front of a glyph quad (vanilla's corner order, wound counter-clockwise seen from
    // the readable side) points back at the camera whatever its rotation, through the
    // billboard and the text pose.
    for (y_rot, x_rot) in [(0.0, 0.0), (90.0, 30.0), (-135.0, -45.0), (200.0, 10.0)] {
        let q = orientation(Billboard::Center, (0.0, 0.0), (y_rot, x_rot));
        let m = text_pose(Mat4::from_quat(q), 20, 1);
        let p = [(0.0, 0.0), (0.0, 8.0), (5.0, 8.0)]
            .map(|(x, y)| m.transform_point3(Vec3::new(x, y, 0.0)));
        let front = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
        let toward = front.dot(look(y_rot, x_rot));
        assert!((toward + 1.0).abs() < 1e-4, "{y_rot} {x_rot}: {toward}");
    }
}

#[test]
fn text_pose_centres_the_block_of_lines_above_the_origin() {
    // Width 20, two lines (height 19): `translate(1 - 10, -19)` in text pixels, then the
    // scale -0.025 and the half turn: the text's top-left corner at (-0.225, 0.475).
    let m = text_pose(Mat4::IDENTITY, 20, 2);
    let origin = m.transform_point3(Vec3::ZERO);
    assert!(
        origin.abs_diff_eq(Vec3::new(-0.225, 0.475, 0.0), 1e-6),
        "{origin}"
    );
    // The bottom of the last line (y = height) lands on the entity's height.
    let bottom = m.transform_point3(Vec3::new(9.0, 19.0, 0.0));
    assert!(bottom.abs_diff_eq(Vec3::ZERO, 1e-6), "{bottom}");
}

fn decoration(view_range: f32, width: f32, height: f32) -> DisplayDecoration {
    DisplayDecoration {
        transformation: Mat4::IDENTITY,
        billboard: Billboard::Fixed,
        entity_y_rot: 0.0,
        entity_x_rot: 0.0,
        brightness: None,
        view_range,
        width,
        height,
        glow_color_override: -1,
        content: DisplayContent::Block {
            state: haste_core::BlockStateId(1),
            light_emission: 0,
        },
    }
}

/// A frustum nothing is inside (one plane facing away from everything).
const NOTHING: [Vec4; 6] = [Vec4::new(0.0, 0.0, 0.0, -1.0); 6];
/// A frustum everything is inside.
const EVERYTHING: [Vec4; 6] = [Vec4::new(0.0, 0.0, 0.0, 1.0); 6];

#[test]
fn view_range_is_sixty_four_blocks_a_unit_times_view_scale() {
    let d = decoration(1.0, 0.0, 0.0);
    let at = |dist: f64, scale: f32| {
        let rel = DVec3::new(dist, 0.0, 0.0);
        should_render(&d, rel, dist * dist, &EVERYTHING, scale)
    };
    assert!(at(63.9, 1.0));
    assert!(!at(64.0, 1.0));
    assert!(at(127.9, 2.0));
    assert!(!at(128.0, 2.0));
    let far = decoration(2.5, 0.0, 0.0);
    assert!(should_render(
        &far,
        DVec3::ZERO,
        159.9 * 159.9,
        &EVERYTHING,
        1.0
    ));
    assert!(!should_render(
        &far,
        DVec3::ZERO,
        160.0 * 160.0,
        &EVERYTHING,
        1.0
    ));
}

#[test]
fn only_a_sized_culling_box_is_frustum_tested() {
    // Width or height 0: `affectedByCulling` is false, drawn whatever the frustum.
    for (w, h) in [(0.0, 0.0), (2.0, 0.0), (0.0, 2.0)] {
        let d = decoration(1.0, w, h);
        assert!(should_render(&d, DVec3::ONE, 3.0, &NOTHING, 1.0));
    }
    let d = decoration(1.0, 2.0, 2.0);
    assert!(!should_render(&d, DVec3::ONE, 3.0, &NOTHING, 1.0));
    assert!(should_render(&d, DVec3::ONE, 3.0, &EVERYTHING, 1.0));
    // The box spans `width` around the entity and `height` up, inflated by 0.5: against
    // the half-space x <= 0, an entity 1.4 east with width 2 still reaches x = -0.1; at 1.6
    // its box ends at x = 0.1.
    let left_of_origin = [Vec4::new(-1.0, 0.0, 0.0, 0.0); 6];
    let near = DVec3::new(1.4, 0.0, 0.0);
    assert!(should_render(&d, near, 2.0, &left_of_origin, 1.0));
    let past = DVec3::new(1.6, 0.0, 0.0);
    assert!(!should_render(&d, past, 2.6, &left_of_origin, 1.0));
}

#[test]
fn block_display_light_is_the_max_with_the_block_emission() {
    // `LightCoordsUtil.max(light, pack(emission, 0))`: block light raised, sky kept.
    assert_eq!(block_display_light((3, 10), 12), (12, 10));
    assert_eq!(block_display_light((14, 2), 7), (14, 2));
    assert_eq!(block_display_light((0, 15), 0), (0, 15));
}

#[test]
fn item_display_contexts_map_onto_the_item_renderer() {
    let cases = [
        (ItemDisplayContext::None, ItemDisplay::None),
        (
            ItemDisplayContext::ThirdPersonLeftHand,
            ItemDisplay::ThirdPersonLeft,
        ),
        (
            ItemDisplayContext::ThirdPersonRightHand,
            ItemDisplay::ThirdPersonRight,
        ),
        (
            ItemDisplayContext::FirstPersonLeftHand,
            ItemDisplay::FirstPersonLeft,
        ),
        (
            ItemDisplayContext::FirstPersonRightHand,
            ItemDisplay::FirstPerson,
        ),
        (ItemDisplayContext::Head, ItemDisplay::Head),
        (ItemDisplayContext::Gui, ItemDisplay::Gui),
        (ItemDisplayContext::Ground, ItemDisplay::Ground),
        (ItemDisplayContext::Fixed, ItemDisplay::Fixed),
        (ItemDisplayContext::OnShelf, ItemDisplay::OnShelf),
    ];
    for (context, display) in cases {
        assert_eq!(item_display_for(context), display, "{context:?}");
    }
    // Half a turn about Y before the item: x and z flip.
    let p = item_display_turn().transform_point3(Vec3::new(1.0, 2.0, 3.0));
    assert!(p.abs_diff_eq(Vec3::new(-1.0, 2.0, -3.0), 1e-6));
}

fn text(flags: u8, background: u32) -> TextDisplayDecoration {
    TextDisplayDecoration {
        text: Arc::new(Component::text("hi")),
        line_width: 200,
        opacity: -1,
        background,
        flags,
    }
}

#[test]
fn text_background_and_colours() {
    // The default background: `(int)(0.25 * 255) << 24`, black at alpha 63.
    let default = TextDisplayDecoration::FLAG_USE_DEFAULT_BACKGROUND;
    assert_eq!(background_color(&text(default, 0x1234_5678)), 0x3F00_0000);
    assert_eq!(background_color(&text(0, 0x4000_0000)), 0x4000_0000);
    // `ARGB.scaleRGB(c, 0.25)` keeps alpha, truncates each channel.
    assert_eq!(shadow_color(0xFFFF_FFFF), 0xFF3F_3F3F);
    assert_eq!(shadow_color(0x80FF_5555), 0x803F_1515);
    // A style colour replaces the rgb under the line's alpha.
    assert_eq!(text_color(0x80FF_FFFF, Some(0xFF5555)), 0x80FF_5555);
    assert_eq!(text_color(0x80FF_FFFF, None), 0x80FF_FFFF);
}

#[test]
fn lines_align_left_right_and_centre() {
    let left = TextDisplayDecoration::FLAG_ALIGN_LEFT;
    let right = TextDisplayDecoration::FLAG_ALIGN_RIGHT;
    assert_eq!(line_offset(left, 40, 25), 0.0);
    assert_eq!(line_offset(right, 40, 25), 15.0);
    assert_eq!(line_offset(0, 40, 25), 7.5);
    // Left wins when both bits are set (`getAlign` tests it first).
    assert_eq!(line_offset(left | right, 40, 25), 0.0);
}

fn styled(s: &str) -> Vec<(char, TextFormat)> {
    s.chars().map(|c| (c, TextFormat::default())).collect()
}

/// Every character six pixels wide, spaces four (the default font's widths for letters and
/// the space, advance included).
fn width_of(ch: char, _: TextFormat) -> f32 {
    if ch == ' ' {
        4.0
    } else {
        6.0
    }
}

fn split(s: &str, max: i32) -> Vec<String> {
    let chars = styled(s);
    let mut ranges = Vec::new();
    string_splitter::split_lines(&chars, max, width_of, &mut ranges);
    ranges
        .into_iter()
        .map(|r| chars[r].iter().map(|(c, _)| *c).collect())
        .collect()
}

#[test]
fn splits_lines_like_the_string_splitter() {
    // Breaks at the last space once the width passes the limit; the space is dropped.
    assert_eq!(split("hello world", 30), ["hello", "world"]);
    assert_eq!(split("hello world", 64), ["hello world"]);
    // No space: before the character that overflowed.
    assert_eq!(split("abcdefgh", 20), ["abc", "def", "gh"]);
    // The first character always stays, however narrow the limit.
    assert_eq!(split("abc", 1), ["a", "b", "c"]);
    // Newlines end lines; a trailing one adds an empty line; nothing gives no lines.
    assert_eq!(split("a\nb", 200), ["a", "b"]);
    assert_eq!(split("a\n", 200), ["a", ""]);
    assert!(split("", 200).is_empty());
    // `Font.width` rounds the summed advances up.
    let w = string_splitter::width(&styled("ab"), |_, _| 2.25);
    assert_eq!(w, 5);
}

#[test]
fn flattens_component_styles_and_legacy_codes() {
    let mut root = Component::text("a");
    root.bold = Some(true);
    let mut child = Component::text("b§cc");
    child.color = Some("#123456".into());
    root.extra.push(child);
    let mut out = Vec::new();
    string_decomposer::flatten(&root, TextFormat::default(), &mut out);
    let chars: String = out.iter().map(|(c, _)| *c).collect();
    assert_eq!(chars, "abc");
    assert!(out[0].1.bold && out[0].1.color.is_none());
    assert_eq!(out[1].1.color, Some(0x123456));
    assert!(out[1].1.bold, "children inherit the parent's style");
    // `§c` sets red (and, legacy, resets the other flags).
    assert_eq!(out[2].1.color, Some(0xFF5555));
    assert_eq!(string_decomposer::parse_color("gold"), Some(0xFFAA00));
    assert_eq!(string_decomposer::parse_color("nope"), None);
}
