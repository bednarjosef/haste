//! The render thread's view of the world (vanilla's `LevelExtractor` side of the client):
//! which dirty sections are handed to the renderer's mesher, and the per-frame entity and
//! block-entity instances.

mod promotion;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use haste_protocol::haste_core::model::EntityInstance;
use haste_protocol::haste_core::view::BlockView as _;
use haste_protocol::haste_core::{ChunkPos, EntityId, SectionPos};
use haste_render::{
    BlockEntityInstance, FrameStats, MeshSnapshot, Renderer, SectionLoadState, SnapshotSource,
};
use haste_world::World;

use super::biomes;
use super::block_entity_extract::{self, BlockEntityTickFrame};
use crate::game::{self, Shared};

/// Wall time the render thread may spend capturing sections for the mesher per frame.
/// Sections handed to the mesh workers per frame (they capture the snapshots themselves).
const SUBMIT_PER_FRAME: usize = 512;
/// Queue entries classified per frame (world probes plus the renderer's section-state
/// note): classifying a whole render distance 32 queue (~28k sections) in one frame
/// would be a visible hitch, so the rest is classified over the next frames.
const SCAN_PER_FRAME: usize = 4096;
/// Held-back entries tested against the pre-mesh cone per frame (only while a turn is
/// predicted): the scan rotates through `waiting`, so the whole list is covered over a
/// few frames without a hitch.
const PREMESH_SCAN_PER_FRAME: usize = 2048;

/// What the client knows about a section right now: gone (its chunk is not loaded),
/// loaded and empty, or loaded with blocks.
fn section_state(world: &World, pos: SectionPos) -> SectionLoadState {
    match world.chunk(pos.chunk()) {
        None => SectionLoadState::Unloaded,
        Some(c) => match c.section(pos.y) {
            None => SectionLoadState::Unloaded,
            Some(s) if s.is_empty() => SectionLoadState::Empty,
            Some(_) => SectionLoadState::NonEmpty,
        },
    }
}

/// The section the camera is in (the feed and the settle test ask for it every frame).
pub(crate) fn camera_section(c: &haste_render::Camera) -> SectionPos {
    let p = c.pos.floor().as_ivec3();
    SectionPos::new(p.x >> 4, p.y >> 4, p.z >> 4)
}

/// Vanilla `SectionUpdateTracker.hasAllNeighbors`: the eight horizontal neighbour
/// chunks of the section are loaded (vanilla also requires their light, which Haste
/// applies with the chunk). A chunk outside the server's reach around the camera never
/// loads, so it does not count as missing — otherwise the outermost ring of the view
/// would never have its first build and settle would never finish.
fn has_all_neighbours(
    world: &World,
    pos: SectionPos,
    camera: SectionPos,
    server_reach: i32,
) -> bool {
    (-1..=1).all(|dx| {
        (-1..=1).all(|dz| {
            let (nx, nz) = (pos.x + dx, pos.z + dz);
            (dx == 0 && dz == 0)
                || world.chunk(ChunkPos::new(nx, nz)).is_some()
                // Vanilla's chunk-send shape (ChunkTrackingView.isWithinDistance without
                // neighbours): a chunk outside the rounded square never loads, so it does
                // not count as missing.
                || {
                    let ex = ((nx - camera.x).abs() - 1).max(0) as i64;
                    let ez = ((nz - camera.z).abs() - 1).max(0) as i64;
                    ex * ex + ez * ez >= server_reach as i64 * server_reach as i64
                }
        })
    })
}

/// Feeds dirty sections to the renderer within a per-frame time budget, nearest first,
/// skipping sections that are empty and were never meshed (nothing to draw, nothing to
/// clear) and holding back sections the renderer does not want yet — vanilla's
/// `LevelExtractor` compiles a dirty section only when it is in the frame's visible
/// sections, so invisible terrain keeps its dirty state and meshes when it becomes
/// visible.
#[derive(Default)]
pub struct WorldFeed {
    /// Sections waiting, sorted so the nearest is last (`pop`).
    pub(crate) queue: Vec<SectionPos>,
    queued: HashSet<SectionPos>,
    /// Sections whose last submitted mesh was non-empty.
    meshed: HashSet<SectionPos>,
    /// Dirty sections the renderer does not want yet (not visible): re-checked whenever
    /// the visible set or the look changes. Not sorted; promotion re-sorts the queue.
    waiting: Vec<SectionPos>,
    /// The visible-set version `waiting` was last checked against.
    last_want_version: u64,
    /// Whether the last promotion moved sections back into the queue (settled must
    /// re-scan `waiting` then, and after every visible-set change).
    promoted: bool,
    /// The wanted-waiting scan's cached answer, with the version it was computed for
    /// (`None`: the first call always scans).
    wanted_waiting: bool,
    wanted_waiting_version: Option<u64>,
    /// Chunks whose columns the renderer's empty-section map has been told about.
    noted_columns: HashSet<ChunkPos>,
    /// Columns a first-build hold is waiting on (vanilla re-checks its dirty set every
    /// frame; Haste only re-checks `waiting` on visible-set changes, so a section whose
    /// neighbours all arrived under a still camera would stay unmeshed until the next
    /// turn). When one of these columns loads, the next feed re-runs the promotion
    /// whatever the visible-set version.
    first_build_waits: HashSet<ChunkPos>,
    /// A column with waiting holds just loaded: promote `waiting` this frame.
    rescan_waiting: bool,
    /// The rotating cursor of the waiting re-check sweep (see [`SWEEP_FRACTION`]).
    sweep_cursor: usize,
    /// The rotating cursor of the pre-mesh scan through `waiting` (see
    /// [`PREMESH_SCAN_PER_FRAME`]).
    premesh_cursor: usize,
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
    /// The work-budget mode this feed's renderer runs (`--work-budget`; the feed's
    /// uploads are the budgeted work, so the feed carries the flag).
    work_budget: haste_render::WorkBudgetMode,
    /// The last feed call's wall time in ms (the work budget's CPU cost input).
    last_feed_ms: f32,
    /// The first-person state last sent to the renderer (None also before the first), and
    /// the skin it was drawn with.
    first_person: Option<game::FirstPersonView>,
    first_person_skin: Option<Arc<haste_entity::skin::PlayerSkin>>,
    /// Each framed item stack's map id (`ItemFrame.getFramedMapId`), decoded from the
    /// stack's components when its bytes last changed (a per-frame compare, a decode only
    /// on change; the entity store keeps no component tables).
    frame_map_ids: HashMap<EntityId, (haste_core::entity::ItemStackData, Option<i32>)>,
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
    /// A feed running the given work-budget mode (`--work-budget`; `Default` is
    /// `fixed`, the historical behaviour).
    pub(crate) fn with_work_budget(mode: haste_render::WorkBudgetMode) -> Self {
        Self {
            work_budget: mode,
            ..Default::default()
        }
    }

    /// The last feed call's wall time in ms (the work budget's CPU cost input).
    pub(crate) fn last_feed_ms(&self) -> f32 {
        self.last_feed_ms
    }
    /// Pulls new dirty sections from the game thread.
    fn collect(&mut self, shared: &Shared) {
        let mut dirty = shared.dirty.lock();
        if dirty.is_empty() {
            return;
        }
        for p in dirty.drain(..) {
            if self.queued.insert(p) {
                self.queue.push(p);
            } else if let Some(i) = self.waiting.iter().position(|&q| q == p) {
                // A re-dirtied held-back section: put it back in the queue so its new
                // state is examined (a held chunk-unload must still free the mesh).
                self.waiting.swap_remove(i);
                self.queue.push(p);
                self.sort_needed = true;
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
        let started = Instant::now();
        renderer.set_work_budget(self.work_budget);
        self.collect(shared);
        self.promote_wanted(renderer);
        self.sweep_wanted(renderer);
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
            // Scan the queue nearest-first (it is sorted so the nearest is last) up to
            // both budgets: sections are submitted, dropped (never-meshed empty) or held
            // back as not visible; classifying the whole view distance in one frame
            // would be a hitch, so the rest waits for the next.
            let mut scanned = 0;
            while batch.len() < SUBMIT_PER_FRAME && scanned < SCAN_PER_FRAME {
                let Some(pos) = self.queue.last().copied() else {
                    break;
                };
                scanned += 1;
                // First encounter of a loaded chunk: classify its whole column. Empty
                // sections never ride the dirty stream (the world skips them, like
                // vanilla's chunk cache drops `hasOnlyAir` sections), so without this
                // their see-through bits would never reach the occlusion walk.
                if !self.noted_columns.contains(&pos.chunk()) {
                    if let Some(chunk) = world.chunk(pos.chunk()) {
                        self.noted_columns.insert(pos.chunk());
                        // A chunk just loaded: the walk treated this column as
                        // see-through-unloaded, so the sightline data changed — re-run
                        // the walk, or the new chunk's sections stay out of the visible
                        // mask until the camera crosses a section boundary.
                        renderer.note_chunk_loaded();
                        // If any first-build hold waited on this column, re-run the
                        // promotion this frame whatever the visible-set version.
                        if self.first_build_waits.remove(&pos.chunk()) {
                            self.rescan_waiting = true;
                        }
                        for (i, section) in chunk.sections.iter().enumerate() {
                            let p = SectionPos::new(pos.x, chunk.min_section + i as i32, pos.z);
                            renderer.note_section_state(
                                p,
                                if section.is_empty() {
                                    SectionLoadState::Empty
                                } else {
                                    SectionLoadState::NonEmpty
                                },
                            );
                        }
                    }
                }
                let state = section_state(&world, pos);
                renderer.note_section_state(pos, state);
                let was_meshed = self.meshed.contains(&pos);
                match state {
                    SectionLoadState::Unloaded => {
                        // The chunk is gone: submit the removal so the renderer frees the
                        // section (vanilla drops a chunk's render data with the chunk),
                        // visible or not.
                        self.queue.pop();
                        self.queued.remove(&pos);
                        self.meshed.remove(&pos);
                        // A reloaded chunk classifies its column again.
                        self.noted_columns.remove(&pos.chunk());
                        if was_meshed {
                            batch.push(pos);
                        }
                    }
                    SectionLoadState::Empty if !was_meshed => {
                        // Never meshed and nothing in it: nothing to draw, nothing to
                        // clear.
                        self.queue.pop();
                        self.queued.remove(&pos);
                    }
                    _ => {
                        // Vanilla `LevelExtractor`: a dirty section compiles only when it
                        // is in the frame's visible sections; a first build also waits for
                        // every neighbour chunk (`SectionUpdateTracker.hasAllNeighbors`).
                        // Held-back sections keep their dirty state — they stay in
                        // `waiting` — and mesh when they become visible.
                        let first_build = !was_meshed
                            && state == SectionLoadState::NonEmpty
                            && !has_all_neighbours(&world, pos, camera, view_distance);
                        if first_build || !renderer.wants_section(pos) {
                            self.queue.pop();
                            self.waiting.push(pos);
                            if first_build {
                                self.note_first_build_hold(pos);
                            }
                            continue;
                        }
                        self.queue.pop();
                        self.queued.remove(&pos);
                        if state == SectionLoadState::NonEmpty {
                            self.meshed.insert(pos);
                        } else {
                            self.meshed.remove(&pos);
                        }
                        batch.push(pos);
                    }
                }
            }
            // Pre-mesh (turn prediction): with submit budget left over after the
            // visible sections, pick held-back sections the renderer's widened cone
            // wants. Visible sections always go first — this only fills the remainder —
            // and the same backlog cap and upload budget apply (the sections join the
            // same batch). A wrong guess only meshes a section early.
            if renderer.premesh_enabled()
                && renderer.premesh_active()
                && renderer.premesh_slack()
                && batch.len() < SUBMIT_PER_FRAME
                && !self.waiting.is_empty()
            {
                let mut scanned = 0;
                while batch.len() < SUBMIT_PER_FRAME && scanned < PREMESH_SCAN_PER_FRAME {
                    if self.premesh_cursor >= self.waiting.len() {
                        self.premesh_cursor = 0;
                    }
                    let pos = self.waiting[self.premesh_cursor];
                    scanned += 1;
                    // The cone test runs before any world probe (it reads only the
                    // mask and the widened frustum): the scan covers thousands of
                    // held-back entries per frame at high render distance and the cone
                    // holds a small fraction of them, so the world lock and the
                    // neighbour probes only run for sections actually inside it.
                    if !renderer.wants_premesh(pos) {
                        self.premesh_cursor += 1;
                        continue;
                    }
                    let state = section_state(&world, pos);
                    renderer.note_section_state(pos, state);
                    let was_meshed = self.meshed.contains(&pos);
                    // The same rules as a visible submission (a first build waits for
                    // every neighbour chunk; an empty section never meshed has nothing
                    // to draw).
                    let first_build = !was_meshed
                        && state == SectionLoadState::NonEmpty
                        && !has_all_neighbours(&world, pos, camera, view_distance);
                    let submittable = !first_build
                        && state != SectionLoadState::Unloaded
                        && !(state == SectionLoadState::Empty && !was_meshed);
                    if submittable {
                        self.waiting.swap_remove(self.premesh_cursor);
                        self.queued.remove(&pos);
                        if state == SectionLoadState::NonEmpty {
                            self.meshed.insert(pos);
                        } else {
                            self.meshed.remove(&pos);
                        }
                        renderer.note_premeshed(pos);
                        batch.push(pos);
                    } else {
                        self.premesh_cursor += 1;
                    }
                }
            }
        }
        // Also uploads meshes that finished since the last frame.
        renderer.update_world_from(&source, &batch, camera_chunk, view_distance);
        self.batch = batch;
        self.last_feed_ms = started.elapsed().as_secs_f32() * 1000.0;
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
        // The maps' snapshot for this frame: one `Arc` clone (the renderer re-uploads only
        // the maps whose data changed). Every path that draws goes through here, so the
        // window, script, screenshot and bench modes all get it.
        renderer.set_maps(shared.maps.lock().clone());
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
        // The item frames' map ids, from each framed stack's component patch.
        self.patch_frame_maps(shared);
        // Entity light rides on the instances themselves (sampled once, where they are built;
        // the renderer holds no world reference and no longer keeps a per-frame light map).
        {
            let world = shared.world.read();
            for inst in &mut self.instances {
                // Falling blocks carry their own light and landing rules: vanilla
                // `FallingBlockRenderer.extractRenderState` samples the light at the block
                // containing `(x, boundingBox.maxY, z)` (not the generic half-block-up
                // probe), and `shouldRender` hides the entity once the world's block at
                // its position is the falling state (the block has landed).
                if let Some(haste_core::model::Decoration::FallingBlock(f)) = &mut inst.decoration {
                    let p = haste_core::BlockPos::new(
                        inst.pos.x.floor() as i32,
                        (inst.pos.y + f64::from(inst.height)).floor() as i32,
                        inst.pos.z.floor() as i32,
                    );
                    inst.light = (world.sky_light(p), world.block_light(p));
                    f.landed = world.block_state(haste_core::BlockPos::new(
                        inst.pos.x.floor() as i32,
                        inst.pos.y.floor() as i32,
                        inst.pos.z.floor() as i32,
                    )) == f.block_state;
                    continue;
                }
                inst.sample_light(&*world);
            }
            // The minecarts' rail snapping reads the level (`OldMinecartBehavior.getPos`),
            // so it happens here too, per frame.
            haste_entity::minecart::snap_rails(&mut self.instances, &*world, &shared.data.blocks);
        }
        (&self.instances, &self.be_instances)
    }

    /// Each framed item stack's map id (`ItemFrame.getFramedMapId` reads the stack's
    /// `minecraft:map_id` component): filled onto the item frame instances so the renderer
    /// can draw the map instead of the item. The stack's bytes are compared per frame and
    /// the component patch decoded only when they changed (the entity store keeps no
    /// component tables, so the decode lives here on the render thread).
    fn patch_frame_maps(&mut self, shared: &Shared) {
        let tables = &shared.component_tables;
        let store = shared.entities.read();
        for inst in self.instances.iter_mut() {
            let Some(haste_core::model::Decoration::ItemFrame(f)) = inst.decoration.as_mut() else {
                continue;
            };
            if f.item.is_none() {
                f.map_id = None;
                continue;
            }
            let Some(entity) = store.get(inst.id) else {
                f.map_id = None;
                continue;
            };
            let stack = entity
                .metadata
                .get(haste_entity::decoration::ITEM_FRAME_ITEM)
                .and_then(|e| match &e.value {
                    haste_core::entity::MetadataValue::ItemStack(s) if !s.is_empty() => Some(s),
                    _ => None,
                });
            let Some(stack) = stack else {
                f.map_id = None;
                continue;
            };
            if self
                .frame_map_ids
                .get(&inst.id)
                .is_none_or(|(s, _)| *s != *stack)
            {
                let map_id = crate::net::map_id_of(stack, tables);
                self.frame_map_ids.insert(inst.id, (stack.clone(), map_id));
            }
            f.map_id = self.frame_map_ids.get(&inst.id).unwrap().1;
        }
    }

    /// Hands the renderer the first-person hand (`Renderer::set_first_person`) when the view's
    /// per-tick part changed since the last call. The skin is fixed for the connection, so
    /// comparing the per-tick part is enough; the skin is cloned only when one happens.
    /// The hand only exists in first person (F5 hides it) and while the HUD is shown
    /// (`GameRenderer.renderItemInHand` skips it when F1 hid the HUD). The totem pop's item
    /// goes along in every perspective (`ScreenEffectRenderer.submit`: only F1 hides it).
    pub(crate) fn set_first_person(
        &mut self,
        shared: &Shared,
        renderer: &mut Renderer,
        perspective: game::Perspective,
        hud_hidden: bool,
        state: &game::ViewState,
    ) {
        renderer.set_item_activation(state.screen.item_activation.filter(|_| !hud_hidden));
        let view = match perspective {
            game::Perspective::FirstPerson if !hud_hidden => state.first_person,
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

    /// Whether everything visible has been meshed and uploaded, and nothing new arrived for
    /// `quiet`. Waits for the sections the renderer wants, not the whole view distance:
    /// held-back sections stay unsettled only while one of them is wanted (or could still
    /// become wanted — the visible set not being final).
    ///
    /// `meshes_pending` (pending + queued results + in-flight) already guarantees every
    /// submitted section was meshed and uploaded; comparing the renderer's resident-section
    /// count against a client-side count cannot work, because a section with blocks can still
    /// mesh to zero vertices (fully enclosed) and the renderer then keeps no entry for it.
    pub(crate) fn settled(
        &mut self,
        stats: &FrameStats,
        quiet: Duration,
        renderer: &Renderer,
        camera: SectionPos,
        view_distance: i32,
    ) -> bool {
        // The wanted-waiting scan walks the whole held-back list, so it is cached and
        // only re-run when the answer can have changed: a new visible set, or a
        // promotion having just moved sections (the next scan must not see them as
        // still-held-back). Settle is queried every frame, including the windowed
        // bench's measured ones.
        let version = renderer.want_version();
        if self.wanted_waiting_version != Some(version) || self.promoted {
            self.promoted = false;
            self.wanted_waiting_version = Some(version);
            self.wanted_waiting = self.waiting.iter().any(|&p| renderer.wants_section(p));
        }
        self.queue.is_empty()
            && stats.meshes_pending == 0
            && self.last_arrival.is_some_and(|t| t.elapsed() >= quiet)
            && renderer.occlusion_settled(camera, view_distance)
            && !self.wanted_waiting
    }
}
