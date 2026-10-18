use std::sync::Arc;

use super::lighting::{
    ao_corner_blend, face_axes, face_corner_offsets, face_side0, smooth_blend, vertex_ao_level,
    AoSample,
};
use super::*;

/// A synthetic flat world: one layer of `block` over bedrock, for the
/// end-to-end mesh tests (real jar, ignored without cache).
struct FlatWorld {
    ground: BlockStateId,
    /// Layers y = 0..layers.
    layers: i32,
}

impl haste_core::view::BlockView for FlatWorld {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        // A finite 16x16 platform (one section footprint), not an infinite
        // plane: side faces of the edge blocks must survive culling.
        if (0..self.layers).contains(&pos.y) && (0..16).contains(&pos.x) && (0..16).contains(&pos.z)
        {
            self.ground
        } else {
            BlockStateId::AIR
        }
    }
    fn is_chunk_loaded(&self, _pos: haste_core::ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, pos: BlockPos) -> u8 {
        if pos.y <= 0 {
            0
        } else {
            15
        }
    }
    fn block_light(&self, _pos: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        384
    }
}

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn state_cache_rekeys_per_atlas() {
    // The per-state cache is thread-local and persists across sections;
    // it is only valid for the (baked, atlas, fluids) set it was built
    // from. A second Renderer in the same process (reconnect, a new
    // world) reuses the mesh thread, so meshing with a different atlas
    // on the same thread must rebuild, not reuse the stale entries.
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas1 = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas1).unwrap();

    // A different atlas over the same baked models: only stone is packed,
    // so stone's sprite id differs from the full atlas's.
    let stone = haste_core::Identifier::parse("minecraft:block/stone").unwrap();
    let atlas2 = haste_assets::AtlasBuilder::build(&mut assets, &[stone], false).unwrap();
    let sprite1 = atlas1.region("minecraft:block/stone").sprite as u16;
    let sprite2 = atlas2.region("minecraft:block/stone").sprite as u16;
    assert_ne!(
        sprite1, sprite2,
        "test premise: atlases place stone differently"
    );

    // One layer of stone: every emitted vertex comes from the stone
    // texture, so every vertex's sprite names the atlas it was meshed with.
    let world = FlatWorld {
        ground: BlockStateId(1),
        layers: 1,
    };
    let snapshot = MeshSnapshot::capture(&world, SectionPos::new(0, 0, 0));
    let pos = SectionPos::new(0, 0, 0);

    // A fresh MeshTables per call: each set carries its own generation,
    // so a set whose allocator addresses reuse a previous set's (the
    // reconnect ABA) still re-keys the cache.
    let mesh1 = mesh_section_with(&snapshot, &baked, &atlas1, &MeshTables::default(), pos);
    assert!(
        mesh1.vertices.iter().all(|v| v.sprite == sprite1),
        "atlas1 mesh must carry atlas1's stone sprite"
    );

    let mesh2 = mesh_section_with(&snapshot, &baked, &atlas2, &MeshTables::default(), pos);
    // The vertex count need not match mesh1's: the baked UVs are atlas1's,
    // so atlas2's sprite-local mapping lands off the 0/1 corners and the
    // greedy merge (legitimately) keeps those quads whole.
    assert!(
        mesh2.vertices.len() >= mesh1.vertices.len(),
        "atlas2's unmerged mesh has at least mesh1's vertices"
    );
    assert!(
        mesh2.vertices.iter().all(|v| v.sprite == sprite2),
        "atlas2 mesh must be rebuilt with atlas2's stone sprite, not the cached one"
    );

    let mesh3 = mesh_section_with(&snapshot, &baked, &atlas1, &MeshTables::default(), pos);
    assert!(
        mesh3.vertices.iter().all(|v| v.sprite == sprite1),
        "switching back to atlas1 must re-key again"
    );
}

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn jar_mesh_flat_stone_section() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();

    // stone state id 1.
    let world = FlatWorld {
        ground: BlockStateId(1),
        layers: 1,
    };
    let snapshot = MeshSnapshot::capture(&world, SectionPos::new(0, 0, 0));
    let mesh = mesh_section(&snapshot, &baked, &atlas, SectionPos::new(0, 0, 0));

    // One layer of stone: only the top faces of y=0 survive (sides culled
    // between neighbours, bottom at world bottom is against air but y=-1
    // is air so the bottom face shows too => 16*16 top + 16*16 bottom
    // + 4 edges * 16 = quads).
    let total_quads: usize = mesh.layers.iter().map(|(_, idx)| idx.len() / 6).sum();
    eprintln!("total quads: {total_quads}");
    // Histogram by cullface direction to debug face emission.
    use std::collections::HashMap;
    let mut hist: HashMap<Option<haste_assets::Direction>, usize> = HashMap::new();
    for (sid, quads) in [(BlockStateId(1), baked.quads(BlockStateId(1)))] {
        let _ = sid;
        for q in quads {
            *hist.entry(q.cullface).or_default() += 1;
        }
    }
    eprintln!("stone baked quads by cullface: {hist:?}");
    // All in the solid layer.
    assert_eq!(mesh.layers.len(), 1);
    assert_eq!(mesh.layers[0].0, haste_assets::RenderLayer::Solid);
    // Vertices packed, sprite ids resolved from the atlas.
    assert!(mesh.vertices.iter().all(|v| v.sprite != 0));
    // Greedy merge: the uniform-light top and bottom faces each merge into
    // one 16x16 quad (UVs repeat 16x, the format's repeat limit), the
    // light-graded side faces stay per block: 1 + 1 + 64 = 66.
    assert_eq!(total_quads, 66, "merged flat stone section");
    // UVs are sprite-local: 0 at the low corner, up to 16 repeats (the
    // fixed-point format's TERRAIN_UV_MAX_REPEAT) at the far corner.
    assert!(mesh
        .vertices
        .iter()
        .all(|v| v.uv[0] <= 16 * 4095 && v.uv[1] <= 16 * 4095));
    assert!(
        mesh.vertices.iter().any(|v| v.uv[0] == 0)
            && mesh.vertices.iter().any(|v| v.uv[0] == 16 * 4095)
    );
    // AABB spans the section.
    assert_eq!(mesh.aabb.0, [0.0, 0.0, 0.0]);
    assert_eq!(mesh.aabb.1, [16.0, 1.0, 16.0]);

    // A 16x16x4 grass slab: faces between grass blocks are culled. Visible: 256 top,
    // 256 bottom, 4 * 16 * 4 sides, plus the tinted overlay on each side face.
    use haste_core::view::BlockStateTable;
    let grass = (0..states.len() as u32)
        .map(BlockStateId)
        .find(|s| {
            states.block_of(*s).path() == "grass_block"
                && states.property_value(*s, "snowy") == Some("false")
        })
        .unwrap();
    let world = FlatWorld {
        ground: grass,
        layers: 4,
    };
    let snapshot = MeshSnapshot::capture(&world, SectionPos::new(0, 0, 0));
    let mesh = mesh_section(&snapshot, &baked, &atlas, SectionPos::new(0, 0, 0));
    let quads: usize = mesh.layers.iter().map(|(_, idx)| idx.len() / 6).sum();
    eprintln!(
        "grass slab: {quads} quads, {} vertices",
        mesh.vertices.len()
    );
    // Greedy merge: unmerged this scene would draw 256 top + 256 bottom +
    // 256 side + 256 overlay quads (1024). The uniform-light faces merge: top
    // and bottom to one quad each, and each edge's y=1..3 side and overlay
    // faces to one 16x3 rect (their face-adjacent cells are lit, so vanilla
    // LightCoordsUtil.smoothBlend's dark-neighbour substitution makes their
    // light uniform — the y=0 row below them is sky-0 in FlatWorld). Only the
    // y=0 side faces are light-graded — their face-adjacent cells sit at sky
    // 0 with lit cells one row up, so the corner blend differs per vertex —
    // and stay per block: 2 + 4 edges * 2 merged + 4 edges * 16 * 2 = 138.
    assert_eq!(
        quads, 138,
        "uniform-light faces merge, graded side faces stay per block"
    );
}

/// The face-box and union-coverage math of the cull rule
/// (`mesher/culling.rs`, vanilla `Block.shouldRenderFace`'s shape comparison).
mod culling_math {
    use super::culling::{face_box, union_covers};
    use haste_assets::Direction;

    /// A quad on `direction`'s plane from (u0, v0) to (u1, v1) in its tangent
    /// axes, in the winding order the bake produces (any order works: the
    /// box test is order-independent).
    fn quad(direction: Direction, u0: f32, v0: f32, u1: f32, v1: f32) -> [[f32; 3]; 4] {
        let n = direction.offset().iter().position(|&c| c != 0).unwrap();
        let (a, b) = match n {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let plane = if direction.offset()[n] > 0 { 1.0 } else { 0.0 };
        let corners = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)];
        let mut p = [[0.0; 3]; 4];
        for (i, (u, v)) in corners.iter().enumerate() {
            p[i][n] = plane;
            p[i][a] = *u;
            p[i][b] = *v;
        }
        p
    }

    #[test]
    fn flush_rectangle_quad_has_a_face_box() {
        // A snow layer's west side: full width, a quarter of the height.
        let q = quad(Direction::West, 0.0, 0.0, 1.0, 0.25);
        assert_eq!(face_box(Direction::West, &q), Some([0.0, 0.0, 1.0, 0.25]));
        // A full face.
        let q = quad(Direction::Up, 0.0, 0.0, 1.0, 1.0);
        assert_eq!(face_box(Direction::Up, &q), Some([0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn off_plane_or_tilted_quad_has_no_face_box() {
        // Off the plane: a slab's top face sits at half height, not on the
        // cell boundary the cullface refers to.
        let mut q = quad(Direction::Up, 0.0, 0.0, 1.0, 1.0);
        for p in &mut q {
            p[1] = 0.5;
        }
        assert_eq!(face_box(Direction::Up, &q), None);
        // On the plane but tilted: the corners leave the extent rectangle.
        let mut q = quad(Direction::West, 0.0, 0.0, 1.0, 0.25);
        q[2][2] = 0.5; // pull one corner into the middle: no longer a rectangle
        assert_eq!(face_box(Direction::West, &q), None);
        // Degenerate: two corners at the same extent corner.
        let q = quad(Direction::East, 0.25, 0.25, 0.25, 0.25);
        assert_eq!(face_box(Direction::East, &q), None);
    }

    #[test]
    fn identical_coverage_culls_like_vanilla_snow_layers() {
        // Snow against snow: the neighbour's side coverage is the same strip,
        // so the face's whole extent is covered (vanilla culls it).
        let region = [0.0, 0.0, 1.0, 0.25];
        let cover = [[0.0, 0.0, 1.0, 0.25]];
        assert!(union_covers(&cover, region));
        // Stone against snow: the full face sticks out above the strip.
        let region = [0.0, 0.0, 1.0, 1.0];
        assert!(!union_covers(&cover, region));
        // No coverage at all (air): never culled.
        assert!(!union_covers(&[], [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn union_coverage_handles_multiple_and_partial_boxes() {
        let region = [0.0, 0.0, 1.0, 1.0];
        // Two half-width strips cover a full-width face.
        let cover = [[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 1.0, 1.0]];
        assert!(union_covers(&cover, region));
        // A gap between the boxes leaves the face visible.
        let cover = [[0.0, 0.0, 0.4, 1.0], [0.6, 0.0, 1.0, 1.0]];
        assert!(!union_covers(&cover, region));
        // A 2x2 tiling of quarters covers the face.
        let cover = [
            [0.0, 0.0, 0.5, 0.5],
            [0.5, 0.0, 1.0, 0.5],
            [0.0, 0.5, 0.5, 1.0],
            [0.5, 0.5, 1.0, 1.0],
        ];
        assert!(union_covers(&cover, region));
        // ... but with one quarter missing the face shows through.
        let cover = [
            [0.0, 0.0, 0.5, 0.5],
            [0.5, 0.0, 1.0, 0.5],
            [0.0, 0.5, 0.5, 1.0],
        ];
        assert!(!union_covers(&cover, region));
        // Coverage larger than the face covers a smaller face.
        assert!(union_covers(&[[0.0, 0.0, 1.0, 1.0]], [0.0, 0.0, 1.0, 0.25]));
    }
}

/// Faces between partial blocks that cover each other exactly (snow layers
/// against snow layers, vanilla `Block.shouldRenderFace`'s shape comparison)
/// must be culled: the mesh of the tests/snow_seams.rs platform (without its
/// snow half) must have no quads on interior block boundaries — only the
/// field's outer sides remain.
#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn platform_interior_boundary_quads_culled() {
    let Some((states, baked, atlas)) = load_baked() else {
        return;
    };
    let find = |path: &str, prop: &str, value: &str| {
        use haste_core::view::BlockStateTable as _;
        (0..states.len() as u32)
            .map(BlockStateId)
            .find(|s| {
                states.block_of(*s).path() == path && states.property_value(*s, prop) == Some(value)
            })
            .unwrap_or_else(|| panic!("{path}[{prop}={value}] in blocks.json"))
    };
    // Dirt at y=0, snowy grass (west) and plain grass (east) at y=1, snow
    // layers on the west half at y=2: the user-bug platform.
    struct Patch {
        snowy_grass: BlockStateId,
        grass: BlockStateId,
        dirt: BlockStateId,
        snow: BlockStateId,
    }
    impl haste_core::view::BlockView for Patch {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            let in_area = (0..10).contains(&p.x) && (0..10).contains(&p.z);
            match p.y {
                0 if in_area => self.dirt,
                1 if in_area => {
                    if p.x < 7 {
                        self.snowy_grass
                    } else {
                        self.grass
                    }
                }
                2 if in_area && p.x < 5 => self.snow,
                _ => BlockStateId::AIR,
            }
        }
        fn is_chunk_loaded(&self, _: haste_core::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, p: BlockPos) -> u8 {
            if p.y <= 0 {
                0
            } else {
                15
            }
        }
        fn block_light(&self, _: BlockPos) -> u8 {
            0
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            384
        }
    }
    let world = Patch {
        snowy_grass: find("grass_block", "snowy", "true"),
        grass: find("grass_block", "snowy", "false"),
        dirt: BlockStateId(1),
        snow: find("snow", "layers", "2"),
    };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = MeshSnapshot::capture(&world, pos);
    let mesh = mesh_section(&snapshot, &baked, &atlas, pos);

    // Decode a vertex's section-local position back to floats.
    let local = |v: &PackedVertex| {
        [
            v.pos[0] as f32 / 65535.0 * 24.0 - 4.0,
            v.pos[1] as f32 / 65535.0 * 24.0 - 4.0,
            v.pos[2] as f32 / 65535.0 * 24.0 - 4.0,
        ]
    };
    let mut interior = 0usize;
    for (_layer, idx) in &mesh.layers {
        for quad in idx.chunks(6) {
            let vs: Vec<_> = quad.iter().map(|i| &mesh.vertices[*i as usize]).collect();
            let ps: Vec<[f32; 3]> = vs.iter().map(|v| local(v)).collect();
            let min = ps.iter().fold([f32::MAX; 3], |mut a, p| {
                for i in 0..3 {
                    a[i] = a[i].min(p[i]);
                }
                a
            });
            let max = ps.iter().fold([f32::MIN; 3], |mut a, p| {
                for i in 0..3 {
                    a[i] = a[i].max(p[i]);
                }
                a
            });
            let near_int = |t: f32| (t - t.round()).abs() < 0.01;
            // Interior for the level the quad is at: the grass/dirt platform
            // spans x/z 0..10, the snow half only x 0..5 (its east edge at x=5
            // is a real exposed face, like the field's outer sides).
            let snow_level = min[1] > 1.9;
            let (x_hi, z_hi) = if snow_level { (5.0, 10.0) } else { (10.0, 10.0) };
            let interior_plane = (min[0] > 0.01 && min[0] < x_hi - 0.01 && min[0] == max[0] && near_int(min[0]))
                || (min[2] > 0.01 && min[2] < z_hi - 0.01 && min[2] == max[2] && near_int(min[2]));
            if interior_plane {
                eprintln!(
                    "boundary quad x {}..{} y {}..{} z {}..{}",
                    min[0], max[0], min[1], max[1], min[2], max[2]
                );
                interior += 1;
            }
        }
    }
    assert_eq!(
        interior, 0,
        "faces on interior boundaries must be culled (vanilla shouldRenderFace)"
    );
}

/// The client jar's baked models and atlas (None without the cache: the caller
/// returns, so the test is skipped rather than failed).
fn load_baked() -> Option<(
    haste_assets::BlocksJsonStates,
    haste_assets::BakedBlockModels,
    haste_assets::AtlasBuilder,
)> {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return None;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
    Some((states, baked, atlas))
}

/// `minecraft:grass_block`'s non-snowy state.
fn grass_state(states: &haste_assets::BlocksJsonStates) -> BlockStateId {
    use haste_core::view::BlockStateTable;
    (0..states.len() as u32)
        .map(BlockStateId)
        .find(|s| {
            states.block_of(*s).path() == "grass_block"
                && states.property_value(*s, "snowy") == Some("false")
        })
        .expect("grass_block in blocks.json")
}

/// One vertex's tint (0xRRGGBB), unpacked from the packed bytes.
fn vertex_tint(v: &PackedVertex) -> u32 {
    ((v.light[2] as u32) << 16) | ((v.light[3] as u32) << 8) | v.tint_b[0] as u32
}

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn every_tinted_quad_of_a_block_takes_its_blended_colour() {
    let Some((states, baked, atlas)) = load_baked() else {
        return;
    };
    use haste_core::view::BlockStateTable as _;
    // A snowy biome's grass colour: explicit, and nothing like the default green.
    let mut snowy = crate::tint::BiomeClimate::plains();
    snowy.grass_color = Some(0x3A7D5C);
    let tables = MeshTables::new(
        Arc::new(crate::tint::TintTable::new(&states)),
        Arc::new(
            crate::tint::BiomeColors::new(&[snowy], &crate::tint::Colormaps::uniform(0x91BD59))
                .with_radius(0),
        ),
        Arc::new(crate::fluid::FluidTable::empty()),
        Arc::new(AoTable::all_solid(states.len())),
    );
    struct Snowy;
    impl crate::tint::BiomeSource for Snowy {
        fn biome(&self, _pos: BlockPos) -> haste_core::BiomeId {
            haste_core::BiomeId(0)
        }
    }
    let grass = grass_state(&states);
    let world = FlatWorld {
        ground: grass,
        layers: 1,
    };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = MeshSnapshot::capture_with_biomes(&world, Some(&Snowy), 0, pos);
    let mesh = mesh_section_with(&snapshot, &baked, &atlas, &tables, pos);

    let sprite = |texture: &str| atlas.region(texture).sprite as u16;
    let (top, side, overlay, bottom) = (
        sprite("minecraft:block/grass_block_top"),
        sprite("minecraft:block/grass_block_side"),
        sprite("minecraft:block/grass_block_side_overlay"),
        sprite("minecraft:block/dirt"),
    );
    // Every quad with a tint index (the top face and the side overlay, both
    // tintindex 0 in block/grass_block) takes the block's blended grass colour;
    // the untinted side and bottom quads stay white.
    let mut tinted = 0usize;
    let mut untinted = 0usize;
    for v in &mesh.vertices {
        let tint = vertex_tint(v);
        match v.sprite {
            s if s == top || s == overlay => {
                assert_eq!(tint, 0x3A7D5C, "tinted quad's colour");
                tinted += 1;
            }
            s if s == side || s == bottom => {
                assert_eq!(tint, TINT_WHITE, "untinted quad's colour");
                untinted += 1;
            }
            _ => {}
        }
    }
    assert!(tinted >= 4 && untinted >= 4, "both kinds must be meshed");
}

/// The grass block's side overlay is the model's second element: four quads
/// lying exactly on the side faces, one tintindex over the same plane the side
/// texture covers. The mesher must emit it unchanged (vanilla draws both
/// quads; the terrain depth test lets them share a depth), or the side loses
/// the biome tint the overlay carries.
#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn side_overlay_quad_is_meshed_on_the_faces_plane() {
    let Some((states, baked, atlas)) = load_baked() else {
        return;
    };
    let grass = grass_state(&states);
    let world = FlatWorld {
        ground: grass,
        layers: 1,
    };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = MeshSnapshot::capture(&world, pos);
    let mesh = mesh_section_with(&snapshot, &baked, &atlas, &MeshTables::default(), pos);

    // The packed position, back to section-local blocks (a quantization step is
    // far below any model feature, so this recovers the exact quad).
    let decode = |p: [u16; 4]| {
        std::array::from_fn(|i| {
            f32::from(p[i]) / 65535.0 * crate::vertex::POS_RANGE - crate::vertex::POS_BIAS
        })
    };
    let close = |a: [f32; 3], b: [f32; 3]| {
        a.iter()
            .zip(b)
            .all(|(x, y)| (x - y).abs() < 2.0 / 65535.0 * crate::vertex::POS_RANGE)
    };
    let sprite_of = |texture: &str| atlas.region(texture).sprite as u16;
    let overlay_sprite = sprite_of("minecraft:block/grass_block_side_overlay");
    let side_sprite = sprite_of("minecraft:block/grass_block_side");
    let at = |quad: &haste_assets::BakedQuad| quad.direction == haste_assets::Direction::North;
    let base = baked
        .quads(grass)
        .iter()
        .find(|q| at(q) && q.tint_index.is_none())
        .expect("the grass block's untinted side quad");
    let overlay = baked
        .quads(grass)
        .iter()
        .find(|q| at(q) && q.tint_index.is_some())
        .expect("the grass block's tinted side overlay quad");
    // The model's two elements share the face plane: same direction, same four
    // vertices. The mesh must keep the overlay on that plane, untouched.
    assert_eq!(overlay.positions, base.positions, "coplanar model elements");

    for want in base.positions {
        assert!(
            mesh.vertices
                .iter()
                .any(|v| v.sprite == side_sprite && close(decode(v.pos), want)),
            "the side quad on the face: {want:?} is not in the mesh"
        );
        assert!(
            mesh.vertices
                .iter()
                .any(|v| v.sprite == overlay_sprite && close(decode(v.pos), want)),
            "the overlay quad shares the face plane, but {want:?} is not in the mesh"
        );
    }
    assert!(
        mesh.vertices.iter().any(|v| v.sprite == overlay_sprite),
        "the grass block's side overlay must be meshed"
    );
}

#[test]
fn table_generations_are_unique() {
    // Every constructed set takes the next counter value: two sets never
    // share a generation, even if the allocator reuses a freed set's
    // addresses (the reconnect ABA the cache key must survive).
    let a = MeshTables::default();
    let b = MeshTables::default();
    assert_ne!(a.generation, b.generation);
    let c = MeshTables::new(
        Arc::new(crate::tint::TintTable::empty()),
        Arc::new(crate::tint::BiomeColors::default()),
        Arc::new(crate::fluid::FluidTable::empty()),
        Arc::new(AoTable::all_solid(0)),
    );
    assert_ne!(a.generation, c.generation);
    assert_ne!(b.generation, c.generation);
}

/// Vanilla `BlockModelLighter.prepareQuadShape`'s faceCubic: flat along the
/// normal AND (on the boundary OR full-collision state). A slab's top face
/// (y = 0.5, not on the boundary, not a full cube) is non-cubic; a full
/// block's face is cubic either way; a full-collision state's inset quad
/// (e.g. glass's model quirks) is cubic too.
#[test]
fn quad_cubic_follows_vanilla_prepare_quad_shape() {
    use haste_assets::Direction as D;
    let quad = |dir: D, from: [f32; 3], to: [f32; 3]| {
        haste_assets::BakedQuad::from_element(dir, from, to, None, None)
    };
    // Slab top: flat at 0.5, off the boundary, state not collision-full.
    let slab_top = quad(D::Up, [0.0, 0.5, 0.0], [1.0, 0.5, 1.0]);
    assert!(!quad_cubic(&slab_top, false), "slab top is not cubic");
    // Same quad on a full-collision state: cubic.
    assert!(quad_cubic(&slab_top, true), "full-collision state is cubic");
    // A boundary face (top of a full block): cubic regardless of state.
    let top = quad(D::Up, [0.0, 1.0, 0.0], [1.0, 1.0, 1.0]);
    assert!(quad_cubic(&top, false), "boundary face is cubic");
    // Bottom face at y = 0: boundary from below.
    let bottom = quad(D::Down, [0.0, 0.0, 0.0], [1.0, 0.0, 1.0]);
    assert!(quad_cubic(&bottom, false), "bottom boundary face is cubic");
    // A side face inset (x = 0.5) on a non-full state: non-cubic.
    let inset = quad(D::East, [0.5, 0.0, 0.0], [0.5, 1.0, 1.0]);
    assert!(!quad_cubic(&inset, false), "inset face is not cubic");
}

#[test]
fn ao_level_counts_occluders() {
    // Vanilla's brightness is 1.0 - 0.2 * count over the four samples
    // (two sides, diagonal, center).
    assert_eq!(vertex_ao_level(false, false, false, false), 0);
    assert_eq!(vertex_ao_level(true, false, false, false), 1);
    assert_eq!(vertex_ao_level(true, true, false, false), 2);
    assert_eq!(vertex_ao_level(true, true, true, false), 3);
    assert_eq!(vertex_ao_level(true, true, true, true), 4);
}

/// Hand-computed corner blend (vanilla BlockModelLighter's shade average
/// and `LightCoordsUtil.smoothBlend`), one up-face vertex.
#[test]
fn corner_blend_matches_vanilla_rules() {
    let stone = |sky: u8, block: u8| AoSample {
        occludes: true,
        light: pack_light(sky, block),
    };
    let air = |sky: u8, block: u8| AoSample {
        occludes: false,
        light: pack_light(sky, block),
    };
    // Center (the cell above the block): air, light (sky 12, block 0).
    // Sides: both stone, the west one's light fully dark (0, 0), the
    // south one's (15, 0); diagonal (the caller's substituted side-0
    // sample): air, (15, 0).
    // AO: stone + stone + air + air = 2 occluders (brightness 0.6).
    // Light: the lit center (sky 12 > 2) replaces the fully dark side
    // with its own light, so sky 12 + 12 + 15 + 15 = 54 levels, in 16th
    // units 54 * 4 = 216 (level 13.5); block 0.
    let (ao, [sky16, block16]) = ao_corner_blend(
        [stone(15, 0), stone(0, 0)],
        air(15, 0), // the substituted diagonal (side 0)
        air(12, 0), // center
    );
    assert_eq!(ao, 2);
    assert_eq!(sky16, 216, "the dark side takes the center's light");
    assert_eq!(block16, 0);

    let (_, [l2, _]) = ao_corner_blend([stone(15, 0), stone(9, 0)], air(15, 0), air(12, 0));
    assert_eq!(l2, 204, "lit neighbours average unchanged");
}

/// `smoothBlend`'s substitutions: a fully dark neighbour takes the whole
/// center light, a zero-sky one only the center's sky (keeping its own
/// block light), and nothing is substituted under a center at or below
/// level 2 on both channels.
#[test]
fn smooth_blend_substitutes_only_under_a_lit_center() {
    let center = pack_light(10, 4);
    // Dark (0,0) -> (10,4); zero-sky (0,7) -> (10,7); lit (14,1) as is.
    let (sky, block) = smooth_blend(
        [pack_light(0, 0), pack_light(0, 7), pack_light(14, 1)],
        center,
    );
    assert_eq!((sky, block), (10 + 10 + 14 + 10, 4 + 7 + 1 + 4));
    // A center at (2, 2) is not lit: the raw sums.
    let dim = pack_light(2, 2);
    let (sky, block) = smooth_blend([pack_light(0, 0), pack_light(0, 7), pack_light(14, 1)], dim);
    assert_eq!((sky, block), (2 + 14, 2 + 7 + 1));
    // Block light alone lights the center (block 3 > 2).
    let torch = pack_light(0, 3);
    let (sky, block) = smooth_blend([0, 0, 0], torch);
    assert_eq!((sky, block), (0, 12));
}

/// Pomme's `compute_face_ao` neighbour triples (vanilla AdjacencyInfo),
/// per face, in face_positions vertex order. side1/side2 order is
/// irrelevant to the AO level (symmetric), but the corner must match.
fn pomme_ao_rows(dir: haste_assets::Direction) -> [[[i32; 3]; 3]; 4] {
    use haste_assets::Direction as D;
    match dir {
        D::Up => [
            [[0, 1, -1], [-1, 1, 0], [-1, 1, -1]],
            [[0, 1, 1], [-1, 1, 0], [-1, 1, 1]],
            [[0, 1, 1], [1, 1, 0], [1, 1, 1]],
            [[0, 1, -1], [1, 1, 0], [1, 1, -1]],
        ],
        D::Down => [
            [[0, -1, 1], [-1, -1, 0], [-1, -1, 1]],
            [[0, -1, -1], [-1, -1, 0], [-1, -1, -1]],
            [[0, -1, -1], [1, -1, 0], [1, -1, -1]],
            [[0, -1, 1], [1, -1, 0], [1, -1, 1]],
        ],
        D::North => [
            [[1, 0, -1], [0, 1, -1], [1, 1, -1]],
            [[1, 0, -1], [0, -1, -1], [1, -1, -1]],
            [[-1, 0, -1], [0, -1, -1], [-1, -1, -1]],
            [[-1, 0, -1], [0, 1, -1], [-1, 1, -1]],
        ],
        D::South => [
            [[-1, 0, 1], [0, 1, 1], [-1, 1, 1]],
            [[-1, 0, 1], [0, -1, 1], [-1, -1, 1]],
            [[1, 0, 1], [0, -1, 1], [1, -1, 1]],
            [[1, 0, 1], [0, 1, 1], [1, 1, 1]],
        ],
        D::West => [
            [[-1, 0, -1], [-1, 1, 0], [-1, 1, -1]],
            [[-1, 0, -1], [-1, -1, 0], [-1, -1, -1]],
            [[-1, 0, 1], [-1, -1, 0], [-1, -1, 1]],
            [[-1, 0, 1], [-1, 1, 0], [-1, 1, 1]],
        ],
        D::East => [
            [[1, 0, 1], [1, 1, 0], [1, 1, 1]],
            [[1, 0, 1], [1, -1, 0], [1, -1, 1]],
            [[1, 0, -1], [1, -1, 0], [1, -1, -1]],
            [[1, 0, -1], [1, 1, 0], [1, 1, -1]],
        ],
    }
}

#[test]
fn face_corner_offsets_match_vanilla_adjacency() {
    use haste_assets::Direction as D;
    for dir in D::ALL {
        let (u, v) = face_axes(dir);
        let mine = face_corner_offsets(dir, u, v);
        let reference = pomme_ao_rows(dir);
        for vi in 0..4 {
            // Corner offset must match exactly (it decides the AO diagonal).
            assert_eq!(
                mine[vi][2], reference[vi][2],
                "{dir:?} vertex {vi} corner mismatch"
            );
            // Side offsets must be the same pair (order may differ: both
            // feed the symmetric vertex_ao_level).
            let mut mine_sides = [mine[vi][0], mine[vi][1]];
            let mut ref_sides = [reference[vi][0], reference[vi][1]];
            mine_sides.sort();
            ref_sides.sort();
            assert_eq!(mine_sides, ref_sides, "{dir:?} vertex {vi} side mismatch");
        }
    }
}

/// The side-0 cell per face is vanilla AdjacencyInfo.corners[0] plus the
/// face normal (the cell whose shade and light vanilla substitutes for an
/// unreachable diagonal).
#[test]
fn face_side0_matches_vanilla_adjacency() {
    use haste_assets::Direction as D;
    // (face, AdjacencyInfo.corners[0])
    let vanilla = [
        (D::Down, D::West),
        (D::Up, D::East),
        (D::North, D::Up),
        (D::South, D::West),
        (D::West, D::Up),
        (D::East, D::Down),
    ];
    for (dir, side0) in vanilla {
        let n = dir.offset();
        let s = side0.offset();
        assert_eq!(
            face_side0()[dir.index()],
            [n[0] + s[0], n[1] + s[1], n[2] + s[2]],
            "{dir:?} side-0 cell"
        );
    }
}

/// A world whose sky light is a function of y alone: 9 at y=1, 3 at y=2.
/// The smooth light of an up face must then be 9 (its four cells all sit at
/// y=1); sampling the neighbours one cell further out would average the y=2
/// cells in and give 5.
struct LitByHeight {
    ground: BlockStateId,
    layers: i32,
}

impl haste_core::view::BlockView for LitByHeight {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        if (0..self.layers).contains(&pos.y) && (0..16).contains(&pos.x) && (0..16).contains(&pos.z)
        {
            self.ground
        } else {
            BlockStateId::AIR
        }
    }
    fn is_chunk_loaded(&self, _pos: haste_core::ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, pos: BlockPos) -> u8 {
        match pos.y {
            1 => 9,
            2 => 3,
            _ => 15,
        }
    }
    fn block_light(&self, _pos: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        0
    }
    fn height(&self) -> i32 {
        512
    }
}

/// The vertex light's sky channel, in whole levels (smooth units / 16).
fn vertex_sky_levels(v: &PackedVertex) -> u8 {
    v.light[0] / crate::vertex::LIGHT16_PER_LEVEL as u8
}

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn smooth_light_reads_the_ao_neighbour_cells() {
    let Some((states, baked, atlas)) = load_baked() else {
        return;
    };
    let ground = grass_state(&states);
    // One layer of opaque blocks, so the top faces' four cells all sit at
    // y=1 (the plane above the blocks) and their light is 9.
    let world = LitByHeight { ground, layers: 1 };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = MeshSnapshot::capture(&world, pos);
    let mesh = mesh_section(&snapshot, &baked, &atlas, pos);
    // The up faces take the shade index 1 and are tinted; find them through
    // the tinted top sprite.
    let up = atlas.region("minecraft:block/grass_block_top").sprite as u16;
    let mut seen = 0;
    for v in &mesh.vertices {
        if v.sprite == up {
            assert_eq!(
                vertex_sky_levels(v),
                9,
                "up face light samples the y=1 cells"
            );
            seen += 1;
        }
    }
    assert!(seen > 0, "the section has top faces");
}

/// The same scene: the *side* faces of the layer must read the light of the
/// cells beside them. Their four cells per vertex are the face-adjacent
/// column's cell (light 15) and its row at y (15) and at y±1 (9 for the
/// upper corner cells), so the side faces carry 15 on the lower corners and
/// 12 on the upper ones. Sampling the neighbours one cell further out along
/// the face normal (as this code did before) would read the y=2 cells and
/// give 9 instead.
#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn smooth_light_of_side_faces_reads_the_adjacent_column() {
    let Some((states, baked, atlas)) = load_baked() else {
        return;
    };
    let ground = grass_state(&states);
    let world = LitByHeight { ground, layers: 1 };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = MeshSnapshot::capture(&world, pos);
    let mesh = mesh_section(&snapshot, &baked, &atlas, pos);
    let side = atlas.region("minecraft:block/grass_block_side").sprite as u16;
    let mut lights = std::collections::BTreeSet::new();
    for v in &mesh.vertices {
        if v.sprite == side {
            lights.insert(v.light[0]);
        }
    }
    assert_eq!(
        lights.into_iter().collect::<Vec<_>>(),
        vec![192, 240],
        "side face light: the corner cells beside the face, not further out"
    );
}
