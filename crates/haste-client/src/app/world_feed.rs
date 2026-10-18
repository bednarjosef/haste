//! The render thread's view of the world (vanilla's `LevelExtractor` side of the client):
//! which dirty sections are handed to the renderer's mesher, and the per-frame entity and
//! block-entity instances.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use haste_protocol::haste_core::model::EntityInstance;
use haste_protocol::haste_core::{ChunkPos, EntityId, SectionPos};
use haste_render::{
    BlockEntityInstance, FrameStats, MeshSnapshot, Renderer, SnapshotSource,
};

use super::biomes;
use super::block_entity_extract::{self, BlockEntityTickFrame};
use crate::game::{self, Shared};

/// Wall time the render thread may spend capturing sections for the mesher per frame.
/// Sections handed to the mesh workers per frame (they capture the snapshots themselves).
const SUBMIT_PER_FRAME: usize = 512;

/// Feeds dirty sections to the renderer within a per-frame time budget, nearest first, and
/// skips sections that are empty and were never meshed (nothing to draw, nothing to clear).
#[derive(Default)]
pub struct WorldFeed {
    /// Sections waiting, sorted so the nearest is last (`pop`).
    pub(crate) queue: Vec<SectionPos>,
    queued: HashSet<SectionPos>,
    /// Sections whose last submitted mesh was non-empty.
    meshed: HashSet<SectionPos>,
    sort_needed: bool,
    pub(crate) last_arrival: Option<Instant>,
    /// Entity instances for this frame (reused).
    pub(crate) instances: Vec<EntityInstance>,
    /// Block entity instances for this frame (reused): the game thread's last tick frame with
    /// the partial tick folded in (`block_entity_extract::fold_partial`).
    be_instances: Vec<BlockEntityInstance>,
    /// The game thread's last extracted block entity tick frame (cloned per frame, an `Arc`).
    be_frame: Arc<BlockEntityTickFrame>,
    /// The shared world as the mesh workers see it (created on the first feed).
    source: Option<Arc<dyn SnapshotSource>>,
    /// Sections submitted this frame (reused).
    batch: Vec<SectionPos>,
    /// The first-person state last sent to the renderer (None also before the first), and
    /// the skin it was drawn with.
    first_person: Option<game::FirstPersonView>,
    first_person_skin: Option<Arc<haste_entity::skin::PlayerSkin>>,
}

/// Lets the mesh worker threads capture section snapshots from the shared world under its
/// read lock, so the render thread never copies blocks.
struct SharedWorldSource(Arc<Shared>);

impl SnapshotSource for SharedWorldSource {
    fn capture(&self, pos: SectionPos, blend_radius: i32) -> Option<MeshSnapshot> {
        let world = self.0.world.read();
        world.chunk(pos.chunk())?;
        Some(MeshSnapshot::capture_with_biomes(
            &*world,
            Some(&biomes::WorldBiomes(&world)),
            blend_radius,
            pos,
        ))
    }
}

impl WorldFeed {
    /// Pulls new dirty sections from the game thread.
    fn collect(&mut self, shared: &Shared) {
        let mut dirty = shared.dirty.lock();
        if dirty.is_empty() {
            return;
        }
        for p in dirty.drain(..) {
            if self.queued.insert(p) {
                self.queue.push(p);
            }
        }
        self.sort_needed = true;
        self.last_arrival = Some(Instant::now());
    }

    /// Queues every non-empty section of the loaded world for meshing (a rebuilt renderer
    /// starts with empty arenas: vanilla's reload re-bakes every model and section too).
    /// Counts as an arrival so `settled` waits for the re-mesh to drain first.
    pub(crate) fn queue_all_sections(&mut self, shared: &Shared) {
        let world = shared.world.read();
        for chunk_pos in world.loaded_chunks() {
            let Some(chunk) = world.chunk(chunk_pos) else {
                continue;
            };
            for (i, section) in chunk.sections.iter().enumerate() {
                if section.is_empty() {
                    continue;
                }
                let pos = SectionPos::new(chunk_pos.x, chunk.min_section + i as i32, chunk_pos.z);
                if self.queued.insert(pos) {
                    self.queue.push(pos);
                }
            }
        }
        self.sort_needed = true;
        self.last_arrival = Some(Instant::now());
    }

    /// Hands queued sections (nearest first) to the mesh workers, which capture and mesh them
    /// off the render thread, and lets the renderer upload finished meshes.
    pub(crate) fn feed(
        &mut self,
        shared: &Arc<Shared>,
        renderer: &mut Renderer,
        camera: SectionPos,
        view_distance: i32,
    ) {
        self.collect(shared);
        if self.sort_needed {
            let d = |p: &SectionPos| {
                let (dx, dy, dz) = (p.x - camera.x, p.y - camera.y, p.z - camera.z);
                dx * dx + dy * dy + dz * dz
            };
            self.queue.sort_unstable_by_key(|p| std::cmp::Reverse(d(p)));
            self.sort_needed = false;
        }
        let camera_chunk = ChunkPos::new(camera.x, camera.z);
        let source = self
            .source
            .get_or_insert_with(|| Arc::new(SharedWorldSource(shared.clone())))
            .clone();
        let mut batch = std::mem::take(&mut self.batch);
        batch.clear();
        {
            let world = shared.world.read();
            while batch.len() < SUBMIT_PER_FRAME {
                let Some(pos) = self.queue.pop() else { break };
                self.queued.remove(&pos);
                let non_empty = world
                    .chunk(pos.chunk())
                    .and_then(|c| c.section(pos.y))
                    .is_some_and(|s| !s.is_empty());
                if !non_empty && !self.meshed.contains(&pos) {
                    continue;
                }
                if non_empty {
                    self.meshed.insert(pos);
                } else {
                    self.meshed.remove(&pos);
                }
                batch.push(pos);
            }
        }
        // Also uploads meshes that finished since the last frame.
        renderer.update_world_from(&source, &batch, camera_chunk, view_distance);
        self.batch = batch;
    }

    /// Copies the interpolated entity instances, gives players their default skin and samples
    /// each entity's light for the renderer. The local player's own model is always included
    /// (the server does not spawn us as an entity, so it is built here from the shared view):
    /// the GUI's inventory preview needs it in first person too, and the caller tells the
    /// renderer not to draw it in the world then (`FrameInput::camera_entity`). The frame's
    /// block entity instances come from the game thread's last tick frame (the partial tick
    /// folded in), so one call returns both slices the frame draws.
    pub(crate) fn entities(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        local: (EntityId, u128),
        partial: f32,
        view: &game::ViewState,
    ) -> (&[EntityInstance], &[BlockEntityInstance]) {
        // The sign text parser (on the game thread) needs the font for the wrap; handed over
        // once per connection.
        if shared.sign_text_metrics.get().is_none() {
            let _ = shared.sign_text_metrics.set(renderer.text_metrics());
        }
        // The game thread's last tick frame, with the partial tick folded into the
        // animatable fields (the lids, spins and anim pairs' lerp stays here, per frame).
        self.be_frame = shared.block_entity_tick.lock().clone();
        block_entity_extract::fold_partial(&self.be_frame, partial, &mut self.be_instances);
        self.instances.clear();
        // Finished skin downloads: registered as textures, then worn (players wear their
        // default skin until then).
        game::skins::pump(shared, renderer);
        let own_skin = {
            let store = shared.entities.read();
            store.render_instances(partial, &mut self.instances);
            store.player_skin(uuid::Uuid::from_u128(local.1))
        };
        game::push_local_instance(&mut self.instances, shared, local, view, partial, &own_skin);
        // The pickup particles (`take_item_entity`): one item/orb instance each at the eased
        // position, drawn through the dropped-item renderer (vanilla `ItemPickupParticleGroup`).
        game::pickup::instances(shared, partial, &mut self.instances);
        // Entity light rides on the instances themselves (sampled once, where they are built;
        // the renderer holds no world reference and no longer keeps a per-frame light map).
        {
            let world = shared.world.read();
            for inst in &mut self.instances {
                inst.sample_light(&*world);
            }
        }
        (&self.instances, &self.be_instances)
    }

    /// Hands the renderer the first-person hand (`Renderer::set_first_person`) when the view's
    /// per-tick part changed since the last call. The skin is fixed for the connection, so
    /// comparing the per-tick part is enough; the skin is cloned only when one happens.
    /// The hand only exists in first person (F5 hides it) and while the HUD is shown
    /// (`GameRenderer.renderItemInHand` skips it when F1 hid the HUD).
    pub(crate) fn set_first_person(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        perspective: game::Perspective,
        hud_hidden: bool,
        first_person: Option<game::FirstPersonView>,
    ) {
        let view = match perspective {
            game::Perspective::FirstPerson if !hud_hidden => first_person,
            _ => None,
        };
        // Our skin changes when its download lands: compare it by identity as well.
        let skin = game::skins::local_skin(shared);
        if view == self.first_person
            && self
                .first_person_skin
                .as_ref()
                .is_some_and(|s| Arc::ptr_eq(s, &skin))
        {
            return;
        }
        self.first_person = view;
        // `FirstPersonHandsAndItemsRenderer`: the sleeve follows the right-sleeve part.
        let sleeve = game::skins::LOCAL_MODEL_PARTS
            .shown(haste_entity::avatar::PlayerModelPart::RightSleeve);
        renderer.set_first_person(view.map(|v| v.with_skin(&skin, sleeve)));
        self.first_person_skin = Some(skin);
    }

    /// Whether everything the game thread produced has been meshed and uploaded, and nothing
    /// new arrived for `quiet`.
    ///
    /// `meshes_pending` (pending + queued results + in-flight) already guarantees every
    /// submitted section was meshed and uploaded; comparing the renderer's resident-section
    /// count against a client-side count cannot work, because a section with blocks can still
    /// mesh to zero vertices (fully enclosed) and the renderer then keeps no entry for it.
    pub(crate) fn settled(&self, stats: &FrameStats, quiet: Duration) -> bool {
        self.queue.is_empty()
            && stats.meshes_pending == 0
            && self.last_arrival.is_some_and(|t| t.elapsed() >= quiet)
    }
}
