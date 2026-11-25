//! A forgotten chunk frees its resident section meshes: the mesh workers' `None`
//! capture (the chunk is gone) must remove the section from the renderer — its arena
//! memory, its visibility entry — instead of leaving a ghost that is drawn until the
//! arenas fill (vanilla drops the render data with the chunk, `ClientChunkCache.drop`).
//! Needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh).

use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, ChunkPos, SectionPos};
use haste_render::Renderer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A flat stone world whose chunk (0, 0) can be forgotten mid-test.
struct Flat {
    stone: BlockStateId,
    loaded: AtomicBool,
}

impl BlockView for Flat {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y == 0
            && (0..16).contains(&p.x)
            && (0..16).contains(&p.z)
            && self.loaded.load(Ordering::Relaxed)
        {
            self.stone
        } else {
            BlockStateId::AIR
        }
    }
    fn is_chunk_loaded(&self, pos: ChunkPos) -> bool {
        pos == ChunkPos::new(0, 0) && self.loaded.load(Ordering::Relaxed)
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

/// The client's capture path: `None` when the section's chunk is gone.
struct Source(Arc<Flat>);

impl haste_render::SnapshotSource for Source {
    fn capture(&self, pos: SectionPos, blend_radius: i32) -> Option<haste_render::MeshSnapshot> {
        if !self.0.is_chunk_loaded(pos.chunk()) {
            return None;
        }
        Some(haste_render::MeshSnapshot::capture_with_biomes(
            &*self.0,
            None,
            blend_radius,
            pos,
        ))
    }
}

/// Drains finished meshes until `pred` holds or the frame budget is exhausted: an empty
/// `update_world_from` call pumps the mesher's result channel like a frame would.
fn drain_until(
    renderer: &mut Renderer,
    source: &Arc<dyn haste_render::SnapshotSource>,
    pred: impl Fn(&Renderer) -> bool,
) {
    for _ in 0..400 {
        if pred(renderer) {
            return;
        }
        renderer.update_world_from(source, &[], ChunkPos::new(0, 0), 4);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn forgotten_chunk_frees_resident_sections() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let data = haste_data::GameData::load().unwrap();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let mut renderer = Renderer::new_offscreen(320, 240, &mut assets, &data.blocks).unwrap();
    let world = Arc::new(Flat {
        stone: data
            .blocks
            .default_state(&haste_core::Identifier::minecraft("stone"))
            .unwrap(),
        loaded: AtomicBool::new(true),
    });
    let source: Arc<dyn haste_render::SnapshotSource> = Arc::new(Source(world.clone()));

    // Load and mesh the section, then forget the chunk and submit it again: the
    // resident count and arena bytes must return to the empty state.
    let pos = SectionPos::new(0, 0, 0);
    renderer.update_world_from(&source, &[pos], ChunkPos::new(0, 0), 4);
    drain_until(&mut renderer, &source, |r| r.arena_memory().3 > 0);
    let (_, _, used, sections) = renderer.arena_memory();
    assert!(sections > 0, "section did not upload");
    assert!(used > 0, "uploaded section has no arena bytes");

    world.loaded.store(false, Ordering::Relaxed);
    renderer.update_world_from(&source, &[pos], ChunkPos::new(0, 0), 4);
    drain_until(&mut renderer, &source, |r| r.arena_memory().3 == 0);
    let (_, _, after_used, after_sections) = renderer.arena_memory();
    assert_eq!(after_sections, 0, "resident sections after the forget");
    assert_eq!(after_used, 0, "arena bytes in use after the forget");
}
