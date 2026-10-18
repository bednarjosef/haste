//! Shared entity vertex plumbing: the vertex buffer helper, textured quads, shading, hurt
//! colours, name-tag and shadow-light curves.

use super::*;

pub(super) fn make_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("entity-vertices"),
        size: (capacity * std::mem::size_of::<EntityVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(super) fn parse_tex(id: &str) -> Identifier {
    Identifier::parse(id).unwrap_or_else(|_| Identifier::minecraft("missingno"))
}

/// One textured quad (`uv` 0..1 over `rect`) through `m`, wound counter-clockwise seen from
/// the side its normal points to, shaded like entities.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_world_quad(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    normal_matrix: Mat3,
    p: [Vec3; 4],
    uv: [[f32; 2]; 4],
    n: Vec3,
    tint: [f32; 3],
    light: (u8, u8),
    rect: [u32; 4],
) {
    let p = p.map(|v| m.transform_point3(v));
    let nw = (normal_matrix * n).normalize_or_zero();
    let s = entity_shade(nw);
    let color = [tint[0] * s, tint[1] * s, tint[2] * s, 1.0];
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let ccw = (p[1] - p[0]).cross(p[2] - p[0]).dot(nw) >= 0.0;
    let order: [usize; 6] = if ccw {
        [0, 1, 2, 0, 2, 3]
    } else {
        [0, 2, 1, 0, 3, 2]
    };
    let r16 = rect.map(|v| v as u16);
    for i in order {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            light: [light.0 as u16, light.1 as u16, 0, 0],
        });
    }
}

/// World-space entity shading: two directional lights at 0.6 and 0.4 ambient (the same
/// lighting model as GUI items, with the light directions in world space).
pub(crate) fn entity_shade(n: Vec3) -> f32 {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    ((n.dot(l0).max(0.0) + n.dot(l1).max(0.0)) * 0.6 + 0.4).min(1.0)
}

/// One shadow quad (four vertices, two triangles) with a flat white colour at `alpha`. The
/// shadow texture is sampled at `uv` (0..1) inside `rect` in the entity atlas; the light is
/// full-bright (vanilla's `15728880`), so `entity.wgsl` leaves the texture colour unchanged.
pub(super) fn push_shadow_quad(
    out: &mut Vec<EntityVertex>,
    p: [Vec3; 4],
    uv: [[f32; 2]; 4],
    alpha: f32,
    rect: [u32; 4],
) {
    let color = [1.0, 1.0, 1.0, alpha];
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let r16 = rect.map(|v| v as u16);
    for i in [0usize, 1, 2, 0, 2, 3] {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            light: [15, 15, 0, 0],
        });
    }
}

/// `Level.getMaxLocalRawBrightness(pos)`: `max(blockLight, skyLight - skyDarken)`
/// (`LevelLightEngine.getRawBrightness`). `sky_darken` is `Level.getSkyDarken`.
pub(crate) fn max_local_raw_brightness(
    world: &dyn BlockView,
    pos: BlockPos,
    sky_darken: i32,
) -> i32 {
    let block = world.block_light(pos) as i32;
    let sky = world.sky_light(pos) as i32 - sky_darken;
    block.max(sky)
}

/// `EntityRenderer.extractNameTags` + `LivingEntityRenderer.shouldShowName`: a name tag shows
/// within 64 blocks, and only within 32 while crouching (`isDiscrete`, distanceSq >= 1024).
pub fn name_tag_visible(dist_sq: f64, is_discrete: bool) -> bool {
    dist_sq < 64.0 * 64.0 && !(is_discrete && dist_sq >= 32.0 * 32.0)
}

/// `Lightmap.getBrightness(dimensionType, level)`: the curved light curve
/// `v / (4 - 3v)` with the dimension's ambient light floor, `lerp(ambient, curved, 1)`.
pub fn lightmap_brightness(ambient: f32, level: i32) -> f32 {
    let v = level as f32 / 15.0;
    let curved = v / (4.0 - 3.0 * v);
    curved + ambient * (1.0 - curved)
}

/// `EntityRenderer.extractShadowPiece`'s per-piece alpha:
/// `clamp(powerAtDepth * 0.5 * brightness, 0, 1)` with
/// `powerAtDepth = pow - (entity.y - pos.y) * 0.5`.
pub fn shadow_piece_alpha(power_at_depth: f32, brightness: f32) -> f32 {
    (power_at_depth * 0.5 * brightness).clamp(0.0, 1.0)
}

/// Hurt flash red overlay strength. Vanilla overlays red at about 30 % (`hurt` in
/// `EntityRenderState` mixes `0.3 * hurt` towards (1, 0, 0)); we use the same 0.3.
pub(crate) const HURT_RED_STRENGTH: f32 = 0.3;

/// The per-channel colour of a hurt entity: shade and light with red mixed in at vanilla's
/// about-30 % overlay strength. This is the CPU-side vertex colour the shader multiplies the
/// texture by (linear RGB), so the tint shows without shader changes.
pub(crate) fn hurt_color(s: f32, hurt: f32) -> [f32; 3] {
    let r = (s + HURT_RED_STRENGTH * hurt).min(1.0);
    let g = s * (1.0 - HURT_RED_STRENGTH * hurt);
    [r, g, g]
}
