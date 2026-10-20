//! Still water surfaces greedy-merge like block faces (`mesher/fluid_block.rs`): a pool's
//! flat top becomes a few large quads instead of one per block, at the exact surface
//! height, with its backward copy filed in the down-facing group.

use super::*;

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn a_still_pool_merges_into_few_quads_at_the_surface_height() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let data = haste_data::GameData::load().unwrap();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let baked = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
    let state = |name: &str, props: &[(&str, &str)]| -> BlockStateId {
        (0..states.len() as u32)
            .map(BlockStateId)
            .find(|s| {
                states.block_of(*s).path() == name
                    && props
                        .iter()
                        .all(|(k, v)| states.property_value(*s, k) == Some(*v))
            })
            .unwrap()
    };
    let sprites = FluidSprites::from_atlas(&atlas);
    let tables = crate::mesher::MeshTables::new(
        Arc::new(crate::tint::TintTable::new(&states)),
        Arc::new(crate::tint::BiomeColors::default()),
        Arc::new(FluidTable::new(
            &states,
            &Fluids::vanilla(&sprites),
            &FluidRules::vanilla(),
            sprites,
        )),
        Arc::new(crate::mesher::AoTable::all_solid(states.len())),
        Arc::new(FluidOccluders::from_shapes(&states, &data.shapes, &baked)),
        Arc::new(crate::mesher::FaceOccluders::absent()),
        Arc::new(crate::mesher::SkipTable::absent()),
    );
    // A 12x12 pool of sources at y = 1, walled and floored with stone, open sky above.
    struct Pool {
        stone: BlockStateId,
        water: BlockStateId,
    }
    impl BlockView for Pool {
        fn block_state(&self, p: BlockPos) -> BlockStateId {
            let inside = (2..14).contains(&p.x) && (2..14).contains(&p.z);
            match p.y {
                0 => self.stone,
                1 if inside => self.water,
                1 => self.stone,
                _ => BlockStateId::AIR,
            }
        }
        fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, _: BlockPos) -> u8 {
            15
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
    let world = Pool {
        stone: state("stone", &[]),
        water: state("water", &[("level", "0")]),
    };
    let pos = SectionPos::new(0, 0, 0);
    let snapshot = crate::mesher::MeshSnapshot::capture(&world, pos);
    let mesh = crate::mesher::mesh_section_with(&snapshot, &baked, &atlas, &tables, pos);
    let water = mesh
        .layers
        .iter()
        .find(|l| l.layer == haste_assets::RenderLayer::Translucent)
        .expect("the pool's surface is translucent");
    let group = |dir: Direction| {
        let (start, count) = water.dirs[dir.index()];
        &water.indices[start as usize..(start + count) as usize]
    };
    // One quad per block would be 144 quads (864 indices) facing up and as many facing
    // down; merged, each is one rectangle split only where the walls' corners meet it.
    for dir in [Direction::Up, Direction::Down] {
        let n = group(dir).len();
        assert!(n > 0, "{dir:?}: the surface is drawn");
        assert!(
            n < 864 / 4,
            "{dir:?}: {n} indices, the surface did not merge"
        );
    }
    // Every vertex of both groups sits on the source's surface (8/9 minus vanilla's
    // inset), up to the u16 position quantization.
    let surface = 1.0 + 8.0 / 9.0 - INSET;
    for dir in [Direction::Up, Direction::Down] {
        for &i in group(dir) {
            let v = &mesh.vertices[i as usize];
            let y =
                f32::from(v.pos[1]) / 65535.0 * crate::vertex::POS_RANGE - crate::vertex::POS_BIAS;
            assert!((y - surface).abs() < 5e-4, "{dir:?}: vertex at y {y}");
        }
    }
}
