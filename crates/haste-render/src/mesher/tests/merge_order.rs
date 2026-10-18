//! The greedy merge is order-independent: the same staged quads, staged in any
//! order, must produce the same merged rectangles and the same direct quads —
//! only their emission order may differ (a rectangle emits at its first
//! member's staged index). A reordered bake or atlas must not change what
//! merges, only where in the buffer the rectangles land.

use super::*;
use haste_assets::Direction;

/// One mergeable full unit face facing up at `cell` (sprite-local UVs on the
/// exact 0/1 corners, so `merge_face` classifies it).
fn up_face(cell: [u8; 3], sprite: u16, tint: u32) -> StagedQuad {
    let quad = haste_assets::BakedQuad::from_element(
        Direction::Up,
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
        None,
        None,
    );
    let uvs = quad.uvs;
    let face = merge_face(&quad, &uvs).expect("a full unit face is mergeable");
    StagedQuad::Merge(MergeQuad {
        key: MergeKey {
            layer: layer_index(RenderLayer::Solid) as u8,
            dir: Direction::Up.index() as u8,
            sprite,
            tint,
            light: [15, 3, 0],
            face,
        },
        cell,
    })
}

/// A quad that cannot merge (a partial face), emitted directly.
fn direct_quad() -> StagedQuad {
    StagedQuad::Direct {
        layer: RenderLayer::Solid,
        dir: Direction::Up.index() as u8,
        positions: [
            [0.2, 0.5, 0.2],
            [0.2, 0.5, 0.8],
            [0.8, 0.5, 0.8],
            [0.8, 0.5, 0.2],
        ],
        uvs: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
        lights: [[15, 3, 0]; 4],
        tint: 0x00ff_ffff,
        sprite: 7,
    }
}

/// The staged quads of one flush: a 6x5 grid of up-faces with a hole at
/// (u=3, v=3), a separate 3-cell run of another sprite, and direct quads
/// interleaved between them.
fn staged_quads() -> Vec<StagedQuad> {
    let mut staged = Vec::new();
    for v in 0..5u8 {
        for u in 0..6u8 {
            if u == 3 && v == 3 {
                continue; // the hole
            }
            staged.push(up_face([u, 0, v], 11, 0x00ff_ffff));
        }
    }
    for u in 0..3u8 {
        staged.push(up_face([10 + u, 0, 0], 42, 0x00ff_ffff));
    }
    staged.insert(4, direct_quad());
    staged.insert(20, direct_quad());
    staged.push(direct_quad());
    staged
}

/// Runs `flush_staged` on `staged` and returns the emitted quads as a sorted
/// multiset of (layer, the quad's four packed vertices) — the merge result,
/// independent of emission order.
fn flushed_quads(staged: &mut [StagedQuad]) -> Vec<(u8, Vec<u8>)> {
    let mut sink = MeshSink::new();
    flush_staged(staged, &mut sink);
    let (vertices, layers) = sink.take_layers();
    let mut quads = Vec::new();
    for layer in &layers {
        for indices in layer.indices.as_chunks::<6>().0 {
            // The six indices are two triangles over one quad's four
            // consecutive vertices; the lowest index is the quad's base.
            let base = *indices.iter().min().unwrap() as usize;
            let mut bytes = Vec::with_capacity(4 * std::mem::size_of::<PackedVertex>());
            for v in &vertices[base..base + 4] {
                bytes.extend_from_slice(bytemuck::bytes_of(v));
            }
            quads.push((layer_index(layer.layer) as u8, bytes));
        }
    }
    quads.sort();
    quads
}

#[test]
fn greedy_merge_is_order_independent() {
    let staged = staged_quads();
    // The hole splits the 6x5 grid into four rectangles (6x3, 6x1, 3x1, 2x1),
    // the second sprite's run is one more, and the three direct quads stay.
    let expect_quads = 4 + 1 + 3;
    let reference = flushed_quads(&mut staged.clone());
    assert_eq!(
        reference.len(),
        expect_quads,
        "29 unit faces + a 3-run must merge into 5 quads plus 3 direct quads"
    );

    // Permutations of the same staged quads: reversed, rotations, and a
    // stride shuffle (deterministic; no RNG needed). All must flush to the
    // same multiset of quads.
    let mut permutations: Vec<Vec<StagedQuad>> =
        vec![staged.iter().rev().cloned().collect(), staged.clone()];
    for r in [1usize, 7, 17] {
        permutations.push(
            staged
                .iter()
                .skip(r)
                .chain(staged.iter().take(r))
                .cloned()
                .collect(),
        );
    }
    permutations.push(
        (0..staged.len())
            .map(|i| staged[i * 13 % staged.len()].clone())
            .collect(),
    );
    for p in permutations {
        assert_eq!(
            flushed_quads(&mut p.clone()),
            reference,
            "staging the same quads in a different order changed the merge result"
        );
    }
}
