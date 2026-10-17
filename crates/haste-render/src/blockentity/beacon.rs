// Beacon beams (vanilla `BeaconRenderer`: `submit` per beam section, `submitBeaconBeam`,
// `renderPart`, `renderQuad`), also used for the end gateway's beam. Textured quads into a
// beam-specific vertex stream: the beam ignores the lightmap (vanilla's `rendertype_beacon_beam`
// shader is texture * colour + fog), so these do not join the entity stream.

use glam::{Mat4, Vec3};
use haste_core::{BlockPos, Identifier};

use super::BlockEntityInstance;
use crate::entity::EntityVertex;

/// One coloured segment of a beacon beam (vanilla `BeaconBeamOwner.Section`): the ARGB colour
/// (the glass above, averaged when it changes mid-beam) and its height in blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeamSection {
    /// The section's colour, vanilla ARGB (`DyeColor.getTextureDiffuseColor` of the glass).
    pub color: u32,
    /// The section's height in blocks (1 for the beacon block's own segment).
    pub height: i32,
}

/// A beam's render data, shared between the game thread (which walks the glass above the
/// beacon, vanilla `BeaconBlockEntity.tick`) and the frame's instances: the sections only;
/// the animation time is per frame, so it rides on the instance
/// ([`BlockEntityInstance::beam_time`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BeamData {
    /// The beam sections bottom-up; empty draws no beam.
    pub sections: Vec<BeamSection>,
}

/// Vanilla `BeaconRenderState.animationTime`: `floorMod(gameTime, 40) + partialTicks`.
pub fn beam_animation_time(game_time: i64, partial: f32) -> f32 {
    game_time.rem_euclid(40) as f32 + partial
}

/// Vanilla `BeaconRenderer.submit`'s per-section heights: every section draws from the beam
/// start to its own height, the last one to `MAX_RENDER_Y` (2048) — the beam runs to the build
/// limit whatever is above.
pub const BEAM_MAX_RENDER_Y: i32 = 2048;

/// The beam radius scale (vanilla `BeaconRenderer.extract`): 1 within 96 blocks (always 1
/// while scoping), else the horizontal distance / 96, so distant beams stay visible.
pub fn beam_radius_scale(distance: f32, scoping: bool) -> f32 {
    if scoping {
        1.0
    } else {
        (distance / 96.0).max(1.0)
    }
}

/// The texture V offset of a beam (vanilla `submitBeaconBeam`): the scroll by animation time,
/// `frac(scroll * 0.2 - floor(scroll * 0.1))` with `scroll = -animationTime` for an upward
/// beam (`height >= 0`).
pub fn beam_tex_v_offset(animation_time: f32, height: i32) -> f32 {
    let scroll = if height < 0 {
        animation_time
    } else {
        -animation_time
    };
    (scroll * 0.2 - (scroll * 0.1).floor()).fract()
}

/// One beam quad's two V texture coordinates (vanilla `submitBeaconBeam`): `vv2 = -1 + off`
/// at the start, and `vv1` at the end — `height * scale * (0.5 / radius)` for the inner beam,
/// `height * scale` for the glow.
pub fn beam_tex_v1(height: f32, scale: f32, radius: f32, glow: bool, vv2: f32) -> f32 {
    if glow {
        height * scale + vv2
    } else {
        height * scale * (0.5 / radius) + vv2
    }
}

/// One beam quad pushed as two triangles, in the beam's own (camera-relative) space: `m` poses
/// it, `rect` is the atlas region of `beacon_beam.png` / `end_gateway_beam.png`. The colour is
/// the section's ARGB with the alpha the pass wants (255 inner, 32 glow).
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_beam_quad(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    p: [Vec3; 4],
    uv: [[f32; 2]; 4],
    color: [f32; 4],
    rect: [u32; 4],
) {
    let p = p.map(|v| m.transform_point3(v));
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let r16 = rect.map(|v| v as u16);
    let order = [0, 1, 2, 0, 2, 3];
    for i in order {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            // The beam shader ignores the lightmap; the field carries the light for the format.
            light: [15, 15, 0, 0],
        });
    }
}

/// Vanilla `renderPart`: four quads around the beam axis (west-north, east-north, east-south,
/// west-south corners as `(x, z)` pairs), each from `beamStart` to `beamEnd`.
#[allow(clippy::too_many_arguments)]
fn emit_beam_part(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    color: [f32; 4],
    beam_start: f32,
    beam_end: f32,
    corners: [(f32, f32); 4],
    vv1: f32,
    vv2: f32,
    rect: [u32; 4],
) {
    let quads = [
        (corners[0], corners[1]),
        (corners[3], corners[2]),
        (corners[1], corners[2]),
        (corners[0], corners[3]),
    ];
    for &(a, b) in &quads {
        push_beam_quad(
            out,
            m,
            [
                Vec3::new(a.0, beam_end, a.1),
                Vec3::new(a.0, beam_start, a.1),
                Vec3::new(b.0, beam_start, b.1),
                Vec3::new(b.0, beam_end, b.1),
            ],
            [[1.0, vv1], [1.0, vv2], [0.0, vv2], [0.0, vv1]],
            color,
            rect,
        );
    }
}

/// Vanilla `BeaconRenderer.submitBeaconBeam` for one beam (also the gateway's, with its own
/// texture, radii and colour): the inner beam (rotated by `animationTime * 2.25 - 45`
/// degrees, opaque) and the glow (axis-aligned, alpha 32/255), both full-bright.
/// `beam_start`/`height` are in blocks; `scale` is 1 for beacons, the gateway's spawn scale.
/// The inner and glow quads go to separate streams (each is one draw with its own pipeline).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_beam(
    inner: &mut Vec<EntityVertex>,
    glow: &mut Vec<EntityVertex>,
    inst_pos: BlockPos,
    camera: glam::DVec3,
    texture: &Identifier,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
    scale: f32,
    animation_time: f32,
    beam_start: i32,
    height: i32,
    color: u32,
    radius_scale: f32,
    solid_radius: f32,
    glow_radius: f32,
) {
    let beam_end = beam_start + height;
    let tex_v_off = beam_tex_v_offset(animation_time, height);
    let root =
        Mat4::from_translation((inst_pos.as_dvec3() - camera).as_vec3() + Vec3::new(0.5, 0.0, 0.5));
    let rect = rect_of(texture);
    // The beam colour as a linear-RGB multiplier: the atlas is sampled as sRGB (linear values
    // in the shader) and the target re-encodes, so vanilla's sRGB-space `texture * colour`
    // needs the colour linearised too (like the armour and banner tints, `argb_linear`).
    // Passing the raw sRGB bytes washed the gateway's magenta out to pink.
    let argb = |c: u32, a: u32| {
        let [r, g, b] = crate::entity::argb_linear(c);
        [r, g, b, a as f32 / 255.0]
    };
    let solid = solid_radius * radius_scale;
    let glow_r = glow_radius * radius_scale;
    // The inner beam turns with the animation; vanilla's `rotateDegrees(YP, ...)`.
    let rot = root * Mat4::from_rotation_y((animation_time * 2.25 - 45.0).to_radians());
    let vv2 = -1.0 + tex_v_off;
    let vv1 = beam_tex_v1(height as f32, scale, solid, false, vv2);
    emit_beam_part(
        inner,
        rot,
        argb(color, 255),
        beam_start as f32,
        beam_end as f32,
        [(0.0, solid), (solid, 0.0), (0.0, -solid), (-solid, 0.0)],
        vv1,
        vv2,
        rect,
    );
    // The glow is axis-aligned around the same axis at the larger radius, translucent.
    let vv1 = beam_tex_v1(height as f32, scale, glow_r, true, vv2);
    emit_beam_part(
        glow,
        root,
        argb(color, 32),
        beam_start as f32,
        beam_end as f32,
        [
            (-glow_r, -glow_r),
            (glow_r, -glow_r),
            (glow_r, glow_r),
            (-glow_r, glow_r),
        ],
        vv1,
        vv2,
        rect,
    );
}

/// The beacon's beam: one `emit_beam` per section, each from the running start to the
/// section's height, the last to [`BEAM_MAX_RENDER_Y`] (vanilla `BeaconRenderer.submit`).
pub(crate) fn emit_beacon_beam(
    inner: &mut Vec<EntityVertex>,
    glow: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    radius_scale: f32,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    let Some(data) = inst.beam.as_deref() else {
        return;
    };
    let texture = Identifier::minecraft("entity/beacon/beacon_beam");
    let mut beam_start = 0i32;
    let last = data.sections.len().saturating_sub(1);
    for (i, section) in data.sections.iter().enumerate() {
        let height = if i == last {
            BEAM_MAX_RENDER_Y
        } else {
            section.height
        };
        emit_beam(
            inner,
            glow,
            inst.pos,
            camera,
            &texture,
            rect_of,
            1.0,
            inst.beam_time,
            beam_start,
            height,
            section.color,
            radius_scale,
            0.2,
            0.25,
        );
        beam_start += section.height;
    }
}
