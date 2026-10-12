// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/renderer/chunk/mesher.rs (vertex packing and constants) and
// packing_consts.rs, reused under GPL-3.0 and reworked for Haste: light is
// carried as sky/block nibbles with the lightmap applied in the shader (so
// time of day needs no remesh), AO and face shade per vertex, tint as a palette
// index. See THIRD_PARTY.md.

/// Section-local position quantization: local coords (block 0..16 plus model
/// overhang) map into `[-POS_BIAS, POS_RANGE - POS_BIAS]` across a u16. Chosen
/// so a 16-block shift is an exact integer number of u16 steps
/// (16/24*65535 = 43690), so the same world position encodes identically in
/// adjacent sections — no seams.
pub const POS_RANGE: f32 = 24.0;
pub const POS_BIAS: f32 = 4.0;

/// Fixed-point scale for sprite-local terrain UVs. 4095 units per sprite keeps
/// every integer repeat boundary through 16 blocks exactly representable in
/// u16 (16 * 4095 = 65520), for greedy meshing later.
pub const TERRAIN_UV_FIXED_SCALE: f32 = 4095.0;
pub const TERRAIN_UV_MAX_REPEAT: f32 = 16.0;

/// Compact terrain GPU vertex (16 bytes). `uv` stores sprite-local coordinates
/// as u16 fixed point; the shader wraps them inside the sprite's atlas
/// rectangle (from the sprite-rect storage buffer), which avoids
/// atlas-boundary rounding and lets a greedy quad repeat one sprite later.
#[repr(C, align(4))]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, Debug)]
pub struct PackedVertex {
    /// Section-local position, quantized over `[-POS_BIAS, POS_RANGE-POS_BIAS]`,
    /// plus one pad u16 (alignment for Uint16x4).
    pub pos: [u16; 4],
    /// Sprite-local UV, u16 fixed point (`TERRAIN_UV_FIXED_SCALE` per unit).
    pub uv: [u16; 2],
    /// Packed per-vertex lighting: [0] sky<<4|block, [1] ao|shade<<2, [2] tint red,
    /// [3] tint green.
    pub light: [u8; 4],
    /// Index into the atlas sprite-rect buffer (0 = missing tile).
    pub sprite: u16,
    /// [0] tint blue, [1] pad (20-byte stride).
    pub tint_b: [u8; 2],
}

impl PackedVertex {
    /// Attribute layout: wgpu requires offsets aligned to the format size, so
    /// the u16s pack into 4-byte groups: pos as Uint16x4 (x, y, z, pad), uv as
    /// Uint16x2, sprite Uint16, light as Uint8x4; 2 pad bytes to stride 20.
    /// `light.0` = sky<<4 | block, `light.1` = ao (bits 0..1) | shade index
    /// (bits 2..4), `light.2`/`light.3` = tint red/green, `tint_b.0` = tint blue.
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: 20,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            // pos (x, y, z, pad)
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint16x4,
                offset: 0,
                shader_location: 0,
            },
            // uv
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint16x2,
                offset: 8,
                shader_location: 1,
            },
            // light [u8; 4] (4-byte alignment)
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x4,
                offset: 12,
                shader_location: 3,
            },
            // sprite
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint16,
                offset: 16,
                shader_location: 2,
            },
            // tint blue + pad
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x2,
                offset: 18,
                shader_location: 5,
            },
        ],
    };

    /// Instance-rate layout: one i32x3 section origin per section instance.
    pub const INSTANCE_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Sint32x3,
            offset: 0,
            shader_location: 4,
        }],
    };
}

/// `light[0]`: sky light (high nibble, 0..15) and block light (low nibble).
pub fn pack_light(sky: u8, block: u8) -> u8 {
    ((sky & 0xF) << 4) | (block & 0xF)
}

/// `light[1]`: AO level (0 darkest..3) in bits 0..1, face shade index in
/// bits 2..4 (0..5 = down/up/north/south/west/east, 6 = unshaded).
pub fn pack_ao_shade(ao_level: u8, shade_index: u8) -> u8 {
    (ao_level & 3) | ((shade_index & 7) << 2)
}

fn unorm_to_u16(x: f32) -> u16 {
    (x.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

fn quantize_coord(local: f32) -> u16 {
    unorm_to_u16((local + POS_BIAS) / POS_RANGE)
}

fn pack_sprite_uv(x: f32) -> u16 {
    (x.clamp(0.0, TERRAIN_UV_MAX_REPEAT) * TERRAIN_UV_FIXED_SCALE + 0.5) as u16
}

/// A meshed quad's raw geometry, before section-local packing.
#[derive(Clone, Copy)]
pub struct QuadVertex {
    pub position: [f32; 3],
    /// Sprite-local UV (0..1 per block face; may exceed 1 with greedy merging).
    pub uv: [f32; 2],
}

/// Pack one vertex: positions are section-local floats, UVs sprite-local. `light` is
/// [sky<<4|block, ao|shade<<2], `tint` 0xRRGGBB (0xFFFFFF = untinted).
pub fn pack_vertex(v: &QuadVertex, sprite: u16, light: [u8; 2], tint: u32) -> PackedVertex {
    PackedVertex {
        pos: [
            quantize_coord(v.position[0]),
            quantize_coord(v.position[1]),
            quantize_coord(v.position[2]),
            0,
        ],
        uv: [pack_sprite_uv(v.uv[0]), pack_sprite_uv(v.uv[1])],
        light: [light[0], light[1], (tint >> 16) as u8, (tint >> 8) as u8],
        sprite,
        tint_b: [tint as u8, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_vertex_is_20_bytes() {
        assert_eq!(std::mem::size_of::<PackedVertex>(), 20);
    }

    #[test]
    fn packed_uv_preserves_integer_repeat_boundaries_exactly() {
        for uv in 0..=16 {
            let decoded = pack_sprite_uv(uv as f32) as f32 / TERRAIN_UV_FIXED_SCALE;
            assert_eq!(decoded, uv as f32);
        }
    }

    #[test]
    fn packed_uv_keeps_fractional_precision() {
        for uv in [0.25_f32, 1.25, 8.5, 15.75] {
            let decoded = pack_sprite_uv(uv) as f32 / TERRAIN_UV_FIXED_SCALE;
            assert!(
                (decoded - uv).abs() <= 0.5 / TERRAIN_UV_FIXED_SCALE,
                "uv {uv} -> {decoded}"
            );
        }
    }

    #[test]
    fn adjacent_blocks_wrap_to_the_same_sprite_position() {
        let wrapped = |x: f32| x - x.floor();
        let a = pack_sprite_uv(0.25) as f32 / TERRAIN_UV_FIXED_SCALE;
        let b = pack_sprite_uv(1.25) as f32 / TERRAIN_UV_FIXED_SCALE;
        assert!((wrapped(a) - wrapped(b)).abs() <= 1.0 / TERRAIN_UV_FIXED_SCALE);
    }

    #[test]
    fn position_quantization_is_seamless_across_sections() {
        // The same world coordinate must encode identically regardless of which
        // section it is local to: 16.0 in one section == 0.0 in the next.
        let a = quantize_coord(16.0);
        let b = quantize_coord(0.0);
        let step = 16.0 / POS_RANGE * 65535.0;
        assert!((a as f32 - b as f32 - step).abs() < 1.0);
    }

    #[test]
    fn light_packing_round_trips() {
        for sky in 0..16u8 {
            for block in 0..16u8 {
                let packed = pack_light(sky, block);
                assert_eq!(packed >> 4, sky);
                assert_eq!(packed & 0xF, block);
            }
        }
        let packed = pack_ao_shade(2, 5);
        assert_eq!(packed & 3, 2);
        assert_eq!((packed >> 2) & 7, 5);
    }
}
