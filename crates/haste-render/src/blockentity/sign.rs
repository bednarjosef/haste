// Sign text (vanilla `AbstractSignRenderer`, `StandingSignRenderer` and `HangingSignRenderer`):
// the dark and glowing text colours, the text transform per attachment and the line metrics.

use glam::{Mat4, Vec3};
use haste_core::BlockPos;

use super::SignSideText;

/// Vanilla `DyeColor.getTextColor` (the ARGB text colours), by dye id.
const DYE_TEXT_COLOR: [u32; 16] = [
    0xFFFF_FFFF,
    0xFFFF_681F,
    0xFFFF_00FF,
    0xFF9A_C0CD,
    0xFFFF_FF00,
    0xFFBF_FF00,
    0xFFFF_69B4,
    0xFF80_8080,
    0xFFD3_D3D3,
    0xFF00_FFFF,
    0xFFA0_20F0,
    0xFF00_00FF,
    0xFF8B_4513,
    0xFF00_FF00,
    0xFFFF_0000,
    0xFF00_0000,
];

/// The outline colour vanilla uses for black glowing text (`AbstractSignRenderer.BLACK_TEXT_OUTLINE_COLOR`).
pub(super) const BLACK_TEXT_OUTLINE_COLOR: u32 = 0xFFF0_EBCC;

/// Vanilla `DyeColor.getTextureDiffuseColor` (the dye's map colour), by dye id.
pub const DYE_DIFFUSE_COLOR: [u32; 16] = [
    16383998, 16351261, 13061821, 3847130, 16701501, 8439583, 15961002, 4673362, 10329495, 1481884,
    8991416, 3949738, 8606770, 6192150, 11546150, 1908001,
];

/// Vanilla `AbstractSignRenderer.getDarkColor`: the colour non-glowing text is drawn with —
/// the dye colour scaled by 0.4 (per channel, clamped), except black glowing text which keeps
/// its special outline colour instead of scaling to near-black.
pub fn sign_dark_color(side: &SignSideText) -> u32 {
    let color = DYE_TEXT_COLOR[side.color as usize % 16];
    if side.color == 15 && side.glowing {
        BLACK_TEXT_OUTLINE_COLOR
    } else {
        let scale = |c: u32| ((c * 2) / 5).min(255);
        0xFF00_0000
            | scale((color >> 16) & 0xFF) << 16
            | scale((color >> 8) & 0xFF) << 8
            | scale(color & 0xFF)
    }
}

/// The colour glowing text is drawn with (the dye's own text colour), vanilla
/// `AbstractSignRenderer.submitSignText`.
pub fn sign_glow_color(side: &SignSideText) -> u32 {
    DYE_TEXT_COLOR[side.color as usize % 16]
}

/// Vanilla `StandingSignRenderer.textTransformation` / `HangingSignRenderer.textTransformation`:
/// the matrix a side's text is drawn through. `angle` is the sign's yaw in vanilla degrees
/// (standing: the `rotation` segment, wall: the wall's `facing.toYRot()`); `front` picks the
/// side (the back adds vanilla's extra 180-degree turn).
pub(crate) fn sign_text_transform(
    hanging: bool,
    angle: f32,
    front: bool,
    camera: glam::DVec3,
    pos: BlockPos,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let mut m = if hanging {
        // Hanging: (0.5, 0.9375, 0.5), rotate -angle, translate (0, -0.3125, 0).
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.9375, 0.5))
            * Mat4::from_rotation_y((-angle).to_radians())
            * Mat4::from_translation(Vec3::new(0.0, -0.3125, 0.0))
    } else {
        // Standing: (0.5, 0.5, 0.5), rotate -angle, wall signs drop to (0, -0.3125, -0.4375).
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.5, 0.5))
            * Mat4::from_rotation_y((-angle).to_radians())
    };
    if !front {
        m *= Mat4::from_rotation_y(180.0f32.to_radians());
    }
    let s = if hanging { 0.0140625 } else { 0.010416667 };
    let offset = if hanging {
        Vec3::new(0.0, -0.32, 0.073)
    } else {
        Vec3::new(0.0, 1.0 / 3.0, 0.046666667)
    };
    m * Mat4::from_translation(offset) * Mat4::from_scale(Vec3::new(s, -s, s))
}

/// Vanilla `SignBlockEntity.getTextLineHeight` (10) and `HangingSignBlockEntity` (9);
/// `getMaxTextLineWidth` is 90 standing, 60 hanging.
pub fn sign_line_metrics_pub(hanging: bool) -> (i32, i32) {
    sign_line_metrics(hanging)
}

pub(crate) fn sign_line_metrics(hanging: bool) -> (i32, i32) {
    if hanging {
        (9, 60)
    } else {
        (10, 90)
    }
}
