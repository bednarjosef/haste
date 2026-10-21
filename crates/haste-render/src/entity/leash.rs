//! The leash rope (`LeashFeatureRenderer`): a strip of vertex pairs from the attached end to
//! the holder, a flat brown rope with a darker stripe on every other step, lit per step between
//! the two ends. The rope has no normal, so it is unshaded (a white texel times the colour) and
//! each triangle faces the camera rather than culling away.

use glam::{DVec3, Vec3};

use super::vertex::EntityVertex;
use super::EntityRenderer;

impl EntityRenderer {
    /// The leash ropes for this frame (`LeashFeatureRenderer`): replaced whole each frame, the
    /// buffer is reused so nothing is allocated once it has grown to the rope count.
    pub fn set_leash_lines(&mut self, lines: &[LeashLine]) {
        self.leash_lines.clear();
        self.leash_lines.extend_from_slice(lines);
    }
}

/// One rope on screen: both ends in world space, and the block and sky light at each end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeashLine {
    pub start: DVec3,
    pub end: DVec3,
    /// Block and sky light (0..=15) at the attached end.
    pub start_light: (u8, u8),
    /// Block and sky light (0..=15) at the holder end.
    pub end_light: (u8, u8),
}

/// `LeashFeatureRenderer.addVertexPair`: the last step of each pass (`k` runs 0..=24 and back).
const LAST_STEP: i32 = 24;
/// The rope's width (`0.05`); the two vertices of a pair sit across it.
const WIDTH: f32 = 0.05;
/// The rope's colour, multiplied by [`DARK`] on every other step.
const ROPE_COLOUR: [f32; 3] = [0.5, 0.4, 0.3];
const DARK: f32 = 0.7;

/// Appends one rope's triangles to `out`: positions camera-relative (`camera` is subtracted in
/// f64 first), uv on the centre of `rect` (the white texel), light lerped per step.
pub(crate) fn push_leash(
    out: &mut Vec<EntityVertex>,
    line: &LeashLine,
    camera: DVec3,
    rect: [u32; 4],
) {
    let origin = (line.start - camera).as_vec3();
    let d = (line.end - line.start).as_vec3();
    // `Mth.invSqrt` of a zero horizontal length is infinite in vanilla, which would turn the
    // pair into NaN; a vertical rope has no horizontal direction to be wide across, so it stays
    // flat (the factor is zero).
    let horizontal = d.x.hypot(d.z);
    let factor = if horizontal > 0.0 {
        WIDTH / 2.0 / horizontal
    } else {
        0.0
    };
    let dx_off = d.z * factor;
    let dz_off = d.x * factor;
    let uv = [
        rect[0] as f32 + rect[2] as f32 * 0.5,
        rect[1] as f32 + rect[3] as f32 * 0.5,
    ];
    let rect16 = rect.map(|v| v as u16);
    let mut prev: Option<[EntityVertex; 2]> = None;
    // Forward pass (fudge `WIDTH`), then backward (fudge 0), each step `k` once.
    for backwards in [false, true] {
        for i in 0..=LAST_STEP {
            let k = if backwards { LAST_STEP - i } else { i };
            let progress = k as f32 / LAST_STEP as f32;
            let fudge = if backwards { 0.0 } else { WIDTH };
            let shade = if k % 2 == i32::from(backwards) {
                DARK
            } else {
                1.0
            };
            let colour = [
                ROPE_COLOUR[0] * shade,
                ROPE_COLOUR[1] * shade,
                ROPE_COLOUR[2] * shade,
                1.0,
            ];
            let light = [
                lerp_light(line.start_light.0, line.end_light.0, progress),
                lerp_light(line.start_light.1, line.end_light.1, progress),
                0,
                0,
            ];
            // `slack` is false for leashes: the rope runs straight from end to end.
            let x = d.x * progress;
            let y = d.y * progress;
            let z = d.z * progress;
            let left = origin + Vec3::new(x - dx_off, y + fudge, z + dz_off);
            let right = origin + Vec3::new(x + dx_off, y + WIDTH - fudge, z - dz_off);
            let vertex = |pos: Vec3| EntityVertex {
                pos: pos.to_array(),
                uv,
                color: colour,
                rect: rect16,
                light,
            };
            let pair = [vertex(left), vertex(right)];
            if let Some(p) = prev {
                push_triangle(out, p[0], p[1], pair[1]);
                push_triangle(out, p[0], pair[1], pair[0]);
            }
            prev = Some(pair);
        }
    }
}

/// `Mth.lerp` on a light level, truncated like vanilla's `(int)` cast.
fn lerp_light(a: u8, b: u8, t: f32) -> u16 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u16
}

/// One triangle, wound so its front faces the camera at the origin (camera-relative positions).
fn push_triangle(out: &mut Vec<EntityVertex>, a: EntityVertex, b: EntityVertex, c: EntityVertex) {
    let (pa, pb, pc) = (Vec3::from(a.pos), Vec3::from(b.pos), Vec3::from(c.pos));
    // The normal points at the viewer when it opposes the centroid's offset from the origin.
    let facing = (pb - pa).cross(pc - pa).dot(pa + pb + pc);
    if facing > 0.0 {
        out.extend([a, c, b]);
    } else {
        out.extend([a, b, c]);
    }
}
