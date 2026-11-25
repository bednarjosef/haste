// Tests for the quad-pulling record: packing round-trips against the vertex
// path (`pack_vertex`), the AO-flip rotation and the rect acceptance rules.

use super::*;

/// An "up" face of a block at (2, 3, 5), vanilla corner order.
fn up_face() -> ([[f32; 3]; 4], [[f32; 2]; 4]) {
    let o = [2.0, 3.0, 5.0];
    let positions = [
        [o[0], o[1] + 1.0, o[2]],
        [o[0], o[1] + 1.0, o[2] + 1.0],
        [o[0] + 1.0, o[1] + 1.0, o[2] + 1.0],
        [o[0] + 1.0, o[1] + 1.0, o[2]],
    ];
    let uvs = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    (positions, uvs)
}

fn lights(sky: u8, block: u8, ao: u8, shade: u8) -> [[u8; 3]; 4] {
    [[sky, ao | shade << 3, block]; 4]
}

#[test]
fn packed_quad_is_32_bytes() {
    assert_eq!(std::mem::size_of::<PackedQuad>(), 32);
}

#[test]
fn unit_face_packs_with_expected_fields() {
    let (positions, uvs) = up_face();
    let light = lights(240, 96, 0, 1);
    let q = pack_rect_quad(&positions, &uvs, &light, 0xFF00FF, 77).unwrap();
    assert_eq!(q.plane, crate::vertex::quantize_coord(4.0));
    assert_eq!(q.u0, crate::vertex::quantize_coord(2.0));
    assert_eq!(q.u1, crate::vertex::quantize_coord(3.0));
    assert_eq!(q.v0, crate::vertex::quantize_coord(5.0));
    assert_eq!(q.v1, crate::vertex::quantize_coord(6.0));
    assert_eq!(q.uv0, [0, 0]);
    assert_eq!(q.uv1, [4095, 4095]);
    assert_eq!(q.sprite, 77);
    assert_eq!(q.tint, [0xFF, 0x00, 0xFF]);
    // Normal axis y = 1; uv.u driven by tangent a (x), uv.v by tangent b (z).
    assert_eq!(q.header & 3, 1);
    assert_eq!((q.header >> 2) & 1, 0);
    assert_eq!((q.header >> 3) & 1, 1);
    // Shade 1 at bit 60, uniform light: sky 240 -> 60, block 96 -> 24, ao 0.
    let lo = q.light[0] as u64 | (q.light[1] as u64) << 32;
    assert_eq!(lo >> 60, 1);
    for corner in 0..4 {
        let bits = (lo >> (15 * corner)) & 0x7FFF;
        assert_eq!(bits & 0x3F, 60);
        assert_eq!((bits >> 6) & 0x3F, 24);
        assert_eq!((bits >> 12) & 7, 0);
    }
}

/// Decoded corner `i` of a record: (position, uv, sky16, block16, ao, shade).
fn decode(q: &PackedQuad, i: usize) -> ([f32; 3], [f32; 2], u16, u16, u8, u8) {
    let n = (q.header & 3) as usize;
    let (a, b) = match n {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let dequant = |v: u16| v as f32 / 65535.0 * crate::vertex::POS_RANGE - crate::vertex::POS_BIAS;
    let start = (q.header >> 4) & 3;
    let step = if (q.header >> 6) & 1 == 1 { 3u8 } else { 1u8 };
    let g = (start + step * i as u8) & 3;
    let (sa, sb) = match g {
        0 => (false, false),
        1 => (true, false),
        2 => (true, true),
        _ => (false, true),
    };
    let mut pos = [0.0f32; 3];
    pos[n] = dequant(q.plane);
    pos[a] = dequant(if sa { q.u1 } else { q.u0 });
    pos[b] = dequant(if sb { q.v1 } else { q.v0 });
    let pick = |lo: u16, hi: u16, s: bool| {
        (if s { hi } else { lo }) as f32 / crate::vertex::TERRAIN_UV_FIXED_SCALE
    };
    let du = ((q.header >> 2) & 1) == 1;
    let dv = ((q.header >> 3) & 1) == 1;
    let uv = [
        pick(q.uv0[0], q.uv1[0], if du { sb } else { sa }),
        pick(q.uv0[1], q.uv1[1], if dv { sb } else { sa }),
    ];
    let lo = q.light[0] as u64 | (q.light[1] as u64) << 32;
    let bits = (lo >> (15 * i)) & 0x7FFF;
    let shade = ((lo >> 60) & 7) as u8;
    (
        pos,
        uv,
        (bits & 0x3F) as u16 * 4,
        ((bits >> 6) & 0x3F) as u16 * 4,
        ((bits >> 12) & 7) as u8,
        shade,
    )
}

/// The record's corners carry the same values the vertex path would pack, in
/// the AO-flip rotation order (`[1, 2, 3, 0]` when corners 0+2 are darker).
#[test]
fn record_corners_match_the_vertex_path() {
    let (positions, uvs) = up_face();
    let light = [
        [128, 1 << 3, 32],
        [160, 2 | 1 << 3, 48],
        [128, 2 | 1 << 3, 32],
        [96, 1 << 3, 16],
    ];
    // ao(0)+ao(2) = 0 + 2 > ao(1)+ao(3) = 2 + 0: no flip.
    let q = pack_rect_quad(&positions, &uvs, &light, 0xFFFFFF, 3).unwrap();
    for i in 0..4 {
        let (pos, uv, sky, block, ao, _shade) = decode(&q, i);
        let v = crate::vertex::pack_vertex(
            &crate::vertex::QuadVertex {
                position: positions[i],
                uv: uvs[i],
            },
            3,
            [light[i][0], light[i][1], light[i][2]],
            0xFFFFFF,
        );
        let vp = |k: usize| v.pos[k] as f32 / 65535.0 * 24.0 - 4.0;
        for (axis, p) in pos.iter().enumerate() {
            assert_eq!(*p, vp(axis), "corner {i} axis {axis}");
        }
        assert_eq!(
            uv,
            [
                v.uv[0] as f32 / crate::vertex::TERRAIN_UV_FIXED_SCALE,
                v.uv[1] as f32 / crate::vertex::TERRAIN_UV_FIXED_SCALE,
            ]
        );
        assert_eq!(
            (sky, block, ao),
            (light[i][0] as u16, light[i][2] as u16, light[i][1] & 7)
        );
    }
}

#[test]
fn ao_flip_rotates_the_corner_order() {
    let (positions, uvs) = up_face();
    // ao(0)+ao(2) = 0+0 < ao(1)+ao(3) = 2+2: flipped, corner order [1, 2, 3, 0].
    let light = [
        [128, 1 << 3, 32],
        [160, 2 | 1 << 3, 48],
        [128, 1 << 3, 32],
        [96, 2 | 1 << 3, 16],
    ];
    let q = pack_rect_quad(&positions, &uvs, &light, 0xFFFFFF, 3).unwrap();
    let order = [1usize, 2, 3, 0];
    for (slot, &corner) in order.iter().enumerate() {
        let (_, _, sky, block, ao, _) = decode(&q, slot);
        assert_eq!(
            (sky, block, ao),
            (
                light[corner][0] as u16,
                light[corner][2] as u16,
                light[corner][1] & 7
            )
        );
    }
}

#[test]
fn rotated_uv_drivers_pack_both_ways() {
    // uv.u follows the z sign (tangent b), uv.v the x sign.
    let (positions, _) = up_face();
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let light = lights(64, 0, 0, 1);
    let q = pack_rect_quad(&positions, &uvs, &light, 0xFFFFFF, 1).unwrap();
    assert_eq!(((q.header >> 2) & 1), 1, "uv.u driven by tangent b");
    assert_eq!(((q.header >> 3) & 1), 0, "uv.v driven by tangent a");
    for (i, want) in uvs.iter().enumerate() {
        let (_, uv, ..) = decode(&q, i);
        assert_eq!(&uv, want);
    }
}

#[test]
fn rejects_non_rectangular_quads() {
    let light = lights(64, 0, 0, 1);
    // A cross plane: no constant axis.
    let cross = [
        [0.146, 0.0, 0.146],
        [0.854, 0.0, 0.854],
        [0.854, 1.0, 0.854],
        [0.146, 1.0, 0.146],
    ];
    assert!(pack_rect_quad(&cross, &[[0.0, 0.0]; 4], &light, 0xFFFFFF, 1).is_none());
    // A tilted quad (constant x, but y and z each take three values).
    let tilted = [
        [0.0, 0.0, 0.0],
        [0.0, 0.5, 1.0],
        [0.0, 1.0, 1.0],
        [0.0, 0.5, 0.0],
    ];
    assert!(pack_rect_quad(&tilted, &[[0.0, 0.0]; 4], &light, 0xFFFFFF, 1).is_none());
    // A rect with uv not following any corner sign.
    let (positions, _) = up_face();
    let bad_uv = [[0.0, 0.0], [0.3, 0.0], [1.0, 1.0], [0.7, 1.0]];
    assert!(pack_rect_quad(&positions, &bad_uv, &light, 0xFFFFFF, 1).is_none());
}

#[test]
fn constant_uv_still_packs() {
    let (positions, _) = up_face();
    let uvs = [[0.5, 0.25]; 4];
    let light = lights(64, 0, 0, 1);
    let q = pack_rect_quad(&positions, &uvs, &light, 0xFFFFFF, 1).unwrap();
    for i in 0..4 {
        let (_, uv, ..) = decode(&q, i);
        // The same fixed-point values the vertex path packs (0.5 -> 2048).
        assert_eq!(uv, [2048.0 / 4095.0, 1024.0 / 4095.0]);
    }
}

#[test]
fn span_indexed_draws() {
    // (index count, base vertex): 4 vertices per record before the base.
    assert_eq!(SpanKind::Rect.indexed_draw(7, 3), (18, 28));
    assert_eq!(SpanKind::General.indexed_draw(7, 3), (18, 28));
    assert_eq!(SpanKind::Indexed.indexed_draw(0, 15), (15, 0));
}

/// Decoded corner `i` of a general record.
fn decode_general(g: &PackedGeneral, i: usize) -> ([f32; 3], [f32; 2], u16, u16, u8) {
    let dequant = |v: u16| v as f32 / 65535.0 * crate::vertex::POS_RANGE - crate::vertex::POS_BIAS;
    let pos = [
        dequant(g.corners[i * 3]),
        dequant(g.corners[i * 3 + 1]),
        dequant(g.corners[i * 3 + 2]),
    ];
    let uv = [
        g.uvs[i * 2] as f32 / crate::vertex::TERRAIN_UV_FIXED_SCALE,
        g.uvs[i * 2 + 1] as f32 / crate::vertex::TERRAIN_UV_FIXED_SCALE,
    ];
    let lo = g.light[0] as u64 | (g.light[1] as u64) << 32;
    let bits = (lo >> (15 * i)) & 0x7FFF;
    (
        pos,
        uv,
        (bits & 0x3F) as u16 * 4,
        ((bits >> 6) & 0x3F) as u16 * 4,
        ((bits >> 12) & 7) as u8,
    )
}

/// The general record's corners carry the same values the vertex path packs,
/// in the AO-flip rotation order.
#[test]
fn general_record_corners_match_the_vertex_path() {
    // A cross plane: no constant axis, so no rect record exists for it.
    let positions = [
        [0.146, 0.0, 0.146],
        [0.854, 0.0, 0.854],
        [0.854, 1.0, 0.854],
        [0.146, 1.0, 0.146],
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    // ao(0)+ao(2) = 0+0 < ao(1)+ao(3) = 2+2: flipped, order [1, 2, 3, 0].
    let light = [
        [128, 1 << 3, 32],
        [160, 2 | 1 << 3, 48],
        [128, 1 << 3, 32],
        [96, 2 | 1 << 3, 16],
    ];
    let g = pack_general_quad(&positions, &uvs, &light, 0xAABBCC, 9);
    assert_eq!(std::mem::size_of::<PackedGeneral>(), 64);
    let order = [1usize, 2, 3, 0];
    for (slot, &corner) in order.iter().enumerate() {
        let (pos, uv, sky, block, ao) = decode_general(&g, slot);
        let v = crate::vertex::pack_vertex(
            &crate::vertex::QuadVertex {
                position: positions[corner],
                uv: uvs[corner],
            },
            9,
            [light[corner][0], light[corner][1], light[corner][2]],
            0xAABBCC,
        );
        let vp = |k: usize| v.pos[k] as f32 / 65535.0 * 24.0 - 4.0;
        for (axis, p) in pos.iter().enumerate() {
            assert_eq!(*p, vp(axis), "corner {slot} axis {axis}");
        }
        assert_eq!(
            uv,
            [
                v.uv[0] as f32 / crate::vertex::TERRAIN_UV_FIXED_SCALE,
                v.uv[1] as f32 / crate::vertex::TERRAIN_UV_FIXED_SCALE,
            ]
        );
        assert_eq!(
            (sky, block, ao),
            (
                light[corner][0] as u16,
                light[corner][2] as u16,
                light[corner][1] & 7
            )
        );
    }
    assert_eq!(g.sprite, 9);
    assert_eq!(g.tint, [0xAA, 0xBB, 0xCC]);
}

/// A fan triangle packs with the apex repeated fourth, so the index
/// pattern's second triangle is degenerate and the first is the fan's.
#[test]
fn general_triangle_repeats_the_apex_fourth() {
    let light = [96, 1 << 3, 16];
    let g = pack_general_triangle(
        ([0.0, 1.0, 0.0], [0.0, 0.0]),
        ([5.0, 1.0, 0.0], [3.0, 0.0]),
        ([5.0, 1.0, 4.0], [3.0, 2.0]),
        light,
        0xFFFFFF,
        4,
    );
    let (p0, uv0, ..) = decode_general(&g, 0);
    let (p3, uv3, ..) = decode_general(&g, 3);
    assert_eq!(p0, p3);
    assert_eq!(uv0, uv3);
    // Corners 1 and 2 keep their own values (on the position grid).
    let q = |x: f32| crate::vertex::quantize_coord(x) as f32 / 65535.0 * 24.0 - 4.0;
    let (p1, ..) = decode_general(&g, 1);
    let (p2, ..) = decode_general(&g, 2);
    assert_eq!(p1, [q(5.0), q(1.0), q(0.0)]);
    assert_eq!(p2, [q(5.0), q(1.0), q(4.0)]);
}
