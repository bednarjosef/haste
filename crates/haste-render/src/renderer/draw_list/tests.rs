use haste_assets::RenderLayer;

use super::*;

fn draw(arena: u32, slot: u32) -> SectionDraw {
    SectionDraw {
        layer: RenderLayer::Solid,
        arena,
        key: 0.0,
        first_index: 0,
        index_count: 6,
        base_vertex: 0,
        kind: crate::quad_format::SpanKind::Indexed,
        origin: [0; 3],
        slot,
        lo: [0.0; 3],
        hi: [1.0; 3],
    }
}

#[test]
fn draws_are_grouped_by_arena_in_visit_order() {
    let src = [draw(2, 0), draw(0, 1), draw(2, 2), draw(1, 3), draw(0, 4)];
    let mut out = vec![draw(7, 99)];
    let mut offsets = Vec::new();
    extend_grouped_by_arena(&mut out, &src, 3, &mut offsets);
    let got: Vec<(u32, u32)> = out.iter().map(|d| (d.arena, d.slot)).collect();
    assert_eq!(got, [(7, 99), (0, 1), (0, 4), (1, 3), (2, 0), (2, 2)]);
    extend_grouped_by_arena(&mut out, &[], 3, &mut offsets);
    assert_eq!(out.len(), 6);
}
