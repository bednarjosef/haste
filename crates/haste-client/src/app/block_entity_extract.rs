// The block entity render data, extracted once per tick on the game thread (vanilla's
// `BlockEntityRenderer`-per-tick `extractRenderState`) and handed to the render thread as a
// tick frame; the render thread only folds the partial tick back in (`fold_partial`). This
// keeps the per-frame cost to a copy plus a few lerps: no world lock, no block state or
// registry lookups, no per-frame cache rebuilds (the caches below live on the game thread,
// where the lids, spins and anims are produced anyway).

mod banner;
mod beacon;
mod chest;
mod facing;
mod frame;
mod gateway;
mod items;
mod piston;
mod pot;
mod sign;

pub use frame::{fold_partial, BlockEntityTickFrame};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use haste_protocol::haste_core::{BlockPos, ChunkPos, Direction};

use facing::{direction_of_property, rotation_or_facing};

use crate::game::Shared;

/// The game thread's block entity extractor: everything the per-tick walk needs that lives
/// longer than a tick. The (pointer, length)-keyed caches are only read here now, so they
/// moved off the render thread's `WorldFeed` with the walk itself. The per-kind work lives
/// next to its helpers, in the concept modules under `block_entity_extract/`.
pub struct BlockEntityExtractor {
    /// Block entity types already logged as undrawable (log once per type).
    unknown_be_types: HashSet<String>,
    /// Parsed sign text by position, rebuilt only when the block entity's NBT changes (the
    /// value is shared with the tick's instances, so no per-tick allocation). The key is the
    /// NBT payload's (pointer, length): the server sends a fresh `Bytes` per update, so a
    /// different payload always re-parses.
    sign_texts: HashMap<BlockPos, (usize, usize, Arc<haste_render::blockentity::SignTextData>)>,
    /// Parsed banner patterns by position, rebuilt only when the NBT changes (see
    /// `sign_texts` for the caching scheme).
    banner_texts: HashMap<BlockPos, (usize, usize, Arc<haste_render::blockentity::BannerData>)>,
    /// Parsed decorated pot sherds by position, rebuilt only when the NBT changes (the same
    /// (pointer, length) caching scheme as `sign_texts`).
    pot_sherds_cache: HashMap<BlockPos, (usize, usize, Arc<haste_render::blockentity::PotSherds>)>,
    /// Parsed item and mob payloads by position, rebuilt only when the block entity's NBT
    /// changes (the (pointer, length) key scheme of `sign_texts`): campfire items, shelf
    /// items, a vault's displayed item, a brushable block's item and a spawner's display mob.
    item_blocks: HashMap<BlockPos, (usize, usize, haste_render::BlockEntityKind)>,
    /// Beacon beam sections by position, rebuilt only when the blocks above change (the key
    /// is the column's state above the beacon: any change re-walks it).
    beacon_beams: HashMap<BlockPos, (u64, Arc<haste_render::BeamData>)>,
    /// End gateway ages by position: the NBT payload key, the world age it was first seen at
    /// and its NBT age then (the age ticks on client-side, vanilla `beamAnimationTick`, since
    /// the server sends the NBT only once).
    gateway_ages: HashMap<BlockPos, (usize, usize, i64, i64)>,
    /// Gateways already logged (debugging the draw, once per position).
    logged_gateways: HashSet<BlockPos>,
    /// Gateways whose beam was live on the last tick (log on change).
    gateway_beam_live: HashMap<BlockPos, bool>,
    /// Moving piston data by position, rebuilt only when the block entity's NBT changes.
    piston_cache: HashMap<BlockPos, (usize, usize, Option<Arc<haste_render::PistonData>>)>,
    /// The per-tick scratch lookups, refilled from the shared anim vecs each tick: the lids'
    /// and skulls' and spins' (previous, current) tick values by position, and the
    /// object-model anims by position.
    lid_map: HashMap<BlockPos, (f32, f32)>,
    skull_anims: HashMap<BlockPos, (f32, f32)>,
    spin_map: HashMap<BlockPos, (f32, f32)>,
}

impl BlockEntityExtractor {
    pub fn new() -> Self {
        Self {
            unknown_be_types: HashSet::new(),
            sign_texts: HashMap::new(),
            banner_texts: HashMap::new(),
            pot_sherds_cache: HashMap::new(),
            item_blocks: HashMap::new(),
            beacon_beams: HashMap::new(),
            gateway_ages: HashMap::new(),
            logged_gateways: HashSet::new(),
            gateway_beam_live: HashMap::new(),
            piston_cache: HashMap::new(),
            lid_map: HashMap::new(),
            skull_anims: HashMap::new(),
            spin_map: HashMap::new(),
        }
    }

    /// Builds this tick's block entity instances (chests, ender chests, trapped chests,
    /// shulker boxes, signs, banners, ... all the kinds the renderer draws) from the world's
    /// stored block entities: resolves the type name to what to draw, reads facing and
    /// double-chest half from the block state, combines the lid animation over a double
    /// chest's halves (vanilla `ChestBlock.opennessCombiner`'s max, `BrightnessCombiner`'s max
    /// light) and samples the block light. Types Haste cannot draw are logged once and
    /// skipped. The result replaces `shared.block_entity_tick`'s frame.
    ///
    /// `feet`/`view_distance` are the camera's chunk and view radius (only the chunks the
    /// camera can see carry drawable block entities; skipping the rest keeps the walk short
    /// on big worlds with nothing in them), `world_age` the tick's world age (the banner wave
    /// phase and the beams' animation time).
    pub fn extract(
        &mut self,
        shared: &Shared,
        feet: glam::DVec3,
        view_distance: i32,
        world_age: i64,
    ) {
        use haste_core::view::BlockView as _;
        let world = shared.world.read();
        self.lid_map.clear();
        {
            let lids = shared.block_entity_lids.lock();
            self.lid_map.extend(lids.iter().copied());
        }
        {
            let anims = shared.skull_anims.lock();
            self.skull_anims.clear();
            self.skull_anims.extend(anims.iter().copied());
        }
        {
            let spins = shared.block_entity_spins.lock();
            self.spin_map.clear();
            self.spin_map.extend(spins.iter().copied());
        }
        // The object-model anims are a HashMap already: looked up under the lock (the game
        // thread is the only reader now, so the lock is only held against the tick's writers).
        let anim_map = shared.block_entity_anims.lock();
        let names = &shared.block_entity_names;
        let blocks = &shared.data.blocks;
        let xmas = haste_render::xmas_textures();
        let cam_chunk = ChunkPos::new(feet.x.floor() as i32 >> 4, feet.z.floor() as i32 >> 4);
        let r = view_distance.max(1);
        let mut frame = BlockEntityTickFrame::default();
        for chunk in world.loaded_chunks() {
            if chunk.x < cam_chunk.x - r
                || chunk.x > cam_chunk.x + r
                || chunk.z < cam_chunk.z - r
                || chunk.z > cam_chunk.z + r
            {
                continue;
            }
            for (pos, be) in world.block_entities_in(chunk) {
                let Some(type_name) = names.get(be.kind as usize) else {
                    if self.unknown_be_types.insert(format!("#{}", be.kind)) {
                        tracing::warn!(
                            "block entity type id {} is outside the registry; not drawn",
                            be.kind
                        );
                    }
                    continue;
                };
                let state = world.block_state(pos);
                let block = blocks.block_of(state);
                let Some(kind) = haste_render::blockentity::kind_of(type_name, block, xmas) else {
                    if self.unknown_be_types.insert(type_name.to_string()) {
                        tracing::warn!("no renderer for block entity type {type_name}; not drawn");
                    }
                    continue;
                };
                // The item and mob payloads (campfire items, spawner display mobs, vault and
                // shelf items, brushable item) come from the block entity's NBT, parsed only
                // when the payload changes. The vault's and spawner's spins come from the
                // game tick (`VaultClientData.updateDisplayItemSpin`, `BaseSpawner.clientTick`).
                let registries = &shared.data.registries;
                let spin = self.spin_map.get(&pos).copied().unwrap_or((0.0, 0.0));
                let kind =
                    self.item_block_kind(pos, be, kind, type_name, blocks, state, registries, spin);
                // Vanilla `LecternRenderer`: the book only while the state's `has_book` is true.
                if kind == haste_render::BlockEntityKind::Lectern
                    && blocks.property_value(state, "has_book") != Some("true")
                {
                    continue;
                }
                let facing = blocks
                    .property_value(state, "facing")
                    .and_then(direction_of_property)
                    .unwrap_or(Direction::South);
                let half = match blocks.property_value(state, "type") {
                    Some("left") => haste_render::ChestHalf::Left,
                    Some("right") => haste_render::ChestHalf::Right,
                    _ => haste_render::ChestHalf::Single,
                };
                // Lid and light: a double chest uses the max over both halves (vanilla's
                // combiners); the neighbour is the block the connected direction points at
                // (`ChestBlock.getConnectedDirection`).
                let (open, light) = self.chest_open_and_light(&world, blocks, pos, facing, half);
                // Signs, banners and skulls: the yaw angle the transform turns by (vanilla
                // `StandingSignRenderer`/`BannerRenderer`/`SkullBlockRenderer` picking the
                // block state's `rotation` segment or the wall `facing`), the sign's cached
                // text, the banner's wave phase and cached patterns, the skull's animation.
                let (mut sign_angle, text, banner_phase, banner, skull_anim, anim, sherds) =
                    match kind {
                        haste_render::BlockEntityKind::Sign { hanging } => {
                            let angle = rotation_or_facing(blocks, state, facing);
                            (
                                angle,
                                self.sign_text(shared, pos, be, hanging),
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::Banner { .. } => {
                            let angle = rotation_or_facing(blocks, state, facing);
                            // Vanilla `BannerRenderer.extractRenderState`'s phase: the position
                            // hash plus the world age, over 100 (the partial tick is folded
                            // back in by the render thread).
                            let phase =
                                haste_render::blockentity::banner_wave_phase(pos, world_age, 0.0);
                            (
                                angle,
                                None,
                                phase,
                                self.banner_data(pos, be, block),
                                haste_render::TickPair::fixed(0.0),
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::Skull { .. } => {
                            // The ground rotation angle (wall skulls read `facing` instead) and
                            // the powered animation's tick ends.
                            let angle = rotation_or_facing(blocks, state, facing);
                            let anim = self.skull_anims.get(&pos).copied().unwrap_or((0.0, 0.0));
                            (
                                angle,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair {
                                    start: anim.0,
                                    end: anim.1,
                                },
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::DecoratedPot => {
                            // The wobble from the game thread's tick, the sherds from the block
                            // entity's NBT (cached like the banner's patterns).
                            let anim = anim_map.get(&pos).copied();
                            (
                                0.0,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                anim,
                                self.pot_sherds(pos, be),
                            )
                        }
                        haste_render::BlockEntityKind::Lectern => {
                            // A lectern's book is at rest (`BOOK_STATE` is constant) and its yaw
                            // comes from the block's `facing`.
                            (
                                0.0,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                None,
                                None,
                            )
                        }
                        haste_render::BlockEntityKind::EnchantTable
                        | haste_render::BlockEntityKind::Bell
                        | haste_render::BlockEntityKind::Conduit => {
                            // Ticked by the game thread (vanilla's block entity tickers); the
                            // renderer's counters are lerped by the partial tick, vanilla's
                            // `Mth.lerp(partialTicks, oX, X)`.
                            (
                                0.0,
                                None,
                                0.0,
                                None,
                                haste_render::TickPair::fixed(0.0),
                                anim_map.get(&pos).copied(),
                                None,
                            )
                        }
                        _ => (
                            0.0,
                            None,
                            0.0,
                            None,
                            haste_render::TickPair::fixed(0.0),
                            None,
                            None,
                        ),
                    };
                // Wall skulls turn by the wall `facing` (their transform reads it); the
                // rotation segment is only for the ground ones.
                if matches!(
                    kind,
                    haste_render::BlockEntityKind::Skull {
                        wall: true,
                        kind: _
                    }
                ) && blocks.property_value(state, "rotation").is_none()
                {
                    sign_angle = 0.0;
                }
                // The special renderers' data: the beacon's beam sections (the client's own
                // glass walk, vanilla `BeaconBlockEntity.tick`), the gateway's beam and the
                // portal's faces, the piston's moved-block state.
                let (beam, gateway_beam, portal_faces, piston) = match kind {
                    haste_render::BlockEntityKind::Beacon => {
                        let beam = self.beacon_data(pos, &world, blocks, &shared.data);
                        (beam, None, 0, None)
                    }
                    haste_render::BlockEntityKind::EndPortal => (
                        None,
                        None,
                        haste_render::blockentity::end_portal_faces(),
                        None,
                    ),
                    haste_render::BlockEntityKind::EndGateway => {
                        let (faces, beam) = self.gateway_render_data(
                            pos,
                            &world,
                            blocks,
                            &shared.data,
                            be,
                            world_age,
                        );
                        // On every beam state change (and once at first sight): what the
                        // renderer gets (debugging the draw).
                        let (seen, changed) = self.gateway_log_state(pos, beam.is_some());
                        if seen || changed {
                            tracing::info!(
                                "end gateway at {pos:?}: faces {faces:#08b}, beam {:?}",
                                beam.map(|b| (b.scale, b.height))
                            );
                        }
                        (None, beam, faces, None)
                    }
                    haste_render::BlockEntityKind::MovingPiston => {
                        (None, None, 0, self.piston_data(pos, be, &anim_map))
                    }
                    _ => (None, None, 0, None),
                };
                if matches!(kind, haste_render::BlockEntityKind::EndGateway) {
                    frame
                        .gateways
                        .insert(pos, self.gateway_frame_entry(pos, be, &world, world_age));
                }
                frame.instances.push(haste_render::BlockEntityInstance {
                    pos,
                    facing,
                    half,
                    kind,
                    open: haste_render::TickPair {
                        start: open.0,
                        end: open.1,
                    },
                    light,
                    sign_angle,
                    text,
                    banner_phase,
                    banner,
                    skull_anim,
                    anim,
                    sherds,
                    beam,
                    // Vanilla's beam animation time, `floorMod(gameTime, 40) + partial` (the
                    // partial tick is folded back in by the render thread).
                    beam_time: haste_render::blockentity::beam_animation_time(world_age, 0.0),
                    gateway_beam,
                    portal_faces,
                    piston,
                });
            }
        }
        *shared.block_entity_tick.lock() = Arc::new(frame);
    }
}

#[cfg(test)]
mod tests;
