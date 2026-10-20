use super::*;

#[test]
fn range_alloc_first_fit_and_coalesce() {
    let mut r = RangeAlloc::new(100);
    let a = r.alloc(10).unwrap();
    let b = r.alloc(20).unwrap();
    let c = r.alloc(30).unwrap();
    assert_eq!((a, b, c), (0, 10, 30));
    r.free(b, 20);
    assert_eq!(r.alloc(5), Some(10), "first fit reuses the hole");
    r.free(10, 5);
    r.free(a, 10);
    r.free(c, 30);
    assert_eq!(r.free, vec![(0, 100)], "all ranges merge back");
    assert_eq!(r.alloc(101), None);
    assert_eq!(r.alloc(100), Some(0));
}

/// A mesh of `quads` quads in one layer and facing group, `fans` extra indices.
fn mesh(x: i32, quads: u32, extra: u32) -> SectionMesh {
    let vertices = vec![<PackedVertex as bytemuck::Zeroable>::zeroed(); quads as usize * 4];
    let mut indices: Vec<u32> = (0..quads)
        .flat_map(|q| [0, 1, 2, 2, 3, 0].map(|i| q * 4 + i))
        .collect();
    indices.extend((0..extra).map(|i| i % (quads * 4)));
    let mut dirs = [(0, 0); 7];
    dirs[1] = (0, indices.len() as u32);
    SectionMesh {
        pos: SectionPos::new(x, 0, 0),
        vertices,
        quads: Vec::new(),
        generals: Vec::new(),
        layers: vec![crate::mesher::SectionLayerMesh {
            layer: RenderLayer::Solid,
            indices,
            dirs,
            spans: Vec::new(),
        }],
        aabb: ([0.0; 3], [16.0; 3]),
        visibility: crate::occlusion::VisibilitySet::none(),
        removed: false,
    }
}

/// Uploads take u16 index ranges (even-sized, so every copy stays 4-byte aligned), a
/// section past 65536 vertices gets a u32 arena, and freeing a section gives its index
/// range back (it leaked from 2026-10-19 until this test). Skips without a GPU.
#[test]
fn uploads_use_u16_ranges_and_free_their_indices() {
    if quad_format_enabled() {
        return;
    }
    let Ok(gpu) = crate::pipeline::Gpu::new(None) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut arenas = MeshArenas::new(gpu.device.clone(), gpu.queue.clone(), None);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    // 10 quads plus 3 fan indices: 63 indices, padded to 64.
    let a = arenas.upload(&mesh(0, 10, 3), &mut encoder).unwrap();
    assert_eq!(arenas.index_format(a.arena), wgpu::IndexFormat::Uint16);
    assert_eq!(a.index_total, 64);
    let b = arenas.upload(&mesh(1, 10, 0), &mut encoder).unwrap();
    assert_eq!(b.index_start, a.index_start + 64);
    let big = arenas.upload(&mesh(2, 16_385, 0), &mut encoder).unwrap();
    assert_ne!(big.arena, a.arena);
    assert_eq!(arenas.index_format(big.arena), wgpu::IndexFormat::Uint32);
    arenas.end_batch(encoder);
    let used = |arenas: &MeshArenas, i: u32| arenas.arenas[i as usize].indices.used();
    assert_eq!(used(&arenas, a.arena), 64 + 60);
    arenas.free(&a);
    arenas.free(&b);
    assert_eq!(used(&arenas, a.arena), 0);
    arenas.free(&big);
    assert_eq!(used(&arenas, big.arena), 0);
}
