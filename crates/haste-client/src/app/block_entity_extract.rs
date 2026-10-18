// The block entity render data, extracted once per tick on the game thread (vanilla's
// `BlockEntityRenderer`-per-tick `extractRenderState`) and handed to the render thread as a
// tick frame; the render thread only folds the partial tick back in (`fold_partial`). This
// keeps the per-frame cost to a copy plus a few lerps: no world lock, no block state or
// registry lookups, no per-frame cache rebuilds (the caches below live on the game thread,
// where the lids, spins and anims are produced anyway).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use haste_protocol::haste_core::{BlockPos, ChunkPos, Direction};
use haste_world::World;

use crate::game::Shared;

/// One tick's extracted block entity render data: the drawable instances (with the tick-end
/// values, as vanilla's render states after a tick) plus what the render thread needs to fold
/// the partial tick back in. Published by the game thread each tick, cloned (an `Arc`) by the
/// render thread each frame.
pub struct BlockEntityTickFrame {
    /// The instances to draw, in the world walk's order, with per-tick values (`open` and the
    /// spin `TickPair`s are raw tick ends; `anim`, `banner_phase`, `beam_time`, the gateway
    /// beam and the piston progress are at partial 0, to be folded by [`fold_partial`]).
    pub instances: Vec<haste_render::BlockEntityInstance>,
    /// The end gateways' (age, max build y) by position, for the beam's partial fixup (the
    /// beam scale is `sin(clamp((age + partial) / 200) * PI)`, not a plain add).
    pub gateways: HashMap<BlockPos, (i64, i32)>,
}

/// A tick frame with nothing in it (before the game thread's first tick).
impl Default for BlockEntityTickFrame {
    fn default() -> Self {
        Self {
            instances: Vec::new(),
            gateways: HashMap::new(),
        }
    }
}

/// Folds the partial tick into a tick frame's interpolatable values, producing the frame's
/// instances (vanilla's per-frame submit reading the render state): the object-model anims
/// (`BlockEntityAnim::at`), the banner's wave phase (`(floorMod(...) + partial) / 100`), the
/// beams' animation time (`floorMod(gameTime, 40) + partial`), the end gateway's beam scale
/// and the moving piston's progress. Everything else is per-tick data copied unchanged.
pub fn fold_partial(
    tick: &BlockEntityTickFrame,
    partial: f32,
    out: &mut Vec<haste_render::BlockEntityInstance>,
) {
    out.clear();
    out.extend(tick.instances.iter().cloned());
    for inst in out.iter_mut() {
        // The object-model anims (bell, pot, conduit, enchanting table): the raw tick pair
        // lerped by the partial tick, the same `Mth.lerp(partialTicks, oX, X)` the feed did.
        if let Some(anim) = inst.anim {
            inst.anim = Some(anim.at(partial));
        }
        match inst.kind {
            haste_render::BlockEntityKind::Banner { .. } => {
                // Vanilla `BannerRenderer.extractRenderState`'s phase: the position hash plus
                // the world age over 100, plus the partial tick.
                inst.banner_phase += partial / 100.0;
            }
            haste_render::BlockEntityKind::Beacon | haste_render::BlockEntityKind::EndGateway => {
                // Vanilla's beam animation time: `floorMod(gameTime, 40) + partial`.
                inst.beam_time += partial;
                if let (true, Some((age, max_y))) = (
                    matches!(inst.kind, haste_render::BlockEntityKind::EndGateway),
                    tick.gateways.get(&inst.pos).copied(),
                ) {
                    inst.gateway_beam = gateway_beam_at(age, partial, max_y);
                }
            }
            haste_render::BlockEntityKind::MovingPiston => {
                // Vanilla `PistonMovingBlockEntity.getProgress`: the anim pair's ends lerped
                // by the partial tick (a fixed pair, the NBT value without an anim entry,
                // lerps to itself). The frame's `Arc` is shared with the tick frame, so a
                // fresh one is built from the cloned data (pistons are few and live a tick).
                if let Some(p) = inst.piston.clone() {
                    let mut d = (*p).clone();
                    let progress = d.progress;
                    d.progress = haste_render::TickPair {
                        start: progress.start,
                        end: progress.at(partial),
                    };
                    inst.piston = Some(Arc::new(d));
                }
            }
            _ => {}
        }
    }
}

/// The end gateway's beam (vanilla `TheEndGatewayRenderer.extractRenderState`): spawning
/// under age 200 in magenta to the build limit; the cooldown phase arrives as a block event
/// (not in the NBT), so it is not drawn yet. `None` hides the beam.
fn gateway_beam_at(age: i64, partial: f32, max_y: i32) -> Option<haste_render::GatewayBeam> {
    if age >= 200 {
        return None;
    }
    // Vanilla `getSpawnPercent`: `clamp((age + partial) / 200, 0, 1)`, then `sin(scale * PI)`;
    // the beam distance is the build limit (`level.getMaxY()`), the height its floor.
    let scale = ((age as f32 + partial) / 200.0).clamp(0.0, 1.0);
    let scale = (scale * std::f32::consts::PI).sin();
    let height = (scale as f64 * max_y as f64).floor() as i32;
    Some(haste_render::GatewayBeam {
        scale,
        height,
        // Vanilla `DyeColor.MAGENTA.getTextureDiffuseColor()` while spawning.
        color: haste_render::blockentity::DYE_DIFFUSE_COLOR[2],
    })
}

/// The game thread's block entity extractor: everything the per-tick walk needs that lives
/// longer than a tick. The (pointer, length)-keyed caches are only read here now, so they
/// moved off the render thread's `WorldFeed` with the walk itself.
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
                let mut open = self.lid_map.get(&pos).copied().unwrap_or((0.0, 0.0));
                let mut light = (world.sky_light(pos), world.block_light(pos));
                if half != haste_render::ChestHalf::Single {
                    let connected = connected_direction(facing, half);
                    let neighbour = pos.offset(connected);
                    if is_double_chest_half(blocks, &world, neighbour, half, facing) {
                        if let Some(n) = self.lid_map.get(&neighbour) {
                            open = (open.0.max(n.0), open.1.max(n.1));
                        }
                        light = (
                            light.0.max(world.sky_light(neighbour)),
                            light.1.max(world.block_light(neighbour)),
                        );
                    }
                }
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
                        let (faces, beam) =
                            self.gateway_data(pos, &world, blocks, &shared.data, be, world_age);
                        // On every beam state change (and once at first sight): what the
                        // renderer gets (debugging the draw).
                        let beam_now = beam.is_some();
                        let seen = self.logged_gateways.insert(pos);
                        let changed = self
                            .gateway_beam_live
                            .insert(pos, beam_now)
                            .is_some_and(|was| was != beam_now);
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
                    let max_y = world.dimension().min_y + world.dimension().height - 1;
                    let age = self.gateway_age(pos, be, world_age);
                    frame.gateways.insert(pos, (age, max_y));
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

    /// The parsed banner patterns at `pos` (`BannerBlockEntity`'s `patterns` NBT list) with
    /// the base colour from the block id (vanilla `BannerBlock.getColor`). Cached like the
    /// sign text: re-parsed only when the payload changes.
    fn banner_data(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        block: &haste_core::Identifier,
    ) -> Option<Arc<haste_render::blockentity::BannerData>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, data)) = self.banner_texts.get(&pos) {
            if (*k, *len) == key {
                return Some(data.clone());
            }
        }
        let parsed = parse_banner_nbt(&be.nbt, block)?;
        self.banner_texts
            .insert(pos, (key.0, key.1, parsed.clone()));
        Some(parsed)
    }

    /// The item or mob payload of a block entity (campfire items, shelf items, a vault's
    /// displayed item, a brushable block's item, a spawner's display mob), parsed from the
    /// block entity's NBT only when the payload changes and cached per position (the same
    /// (pointer, length) scheme as the sign text). Kinds with no payload are handed back
    /// unchanged.
    #[allow(clippy::too_many_arguments)]
    fn item_block_kind(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        kind: haste_render::BlockEntityKind,
        type_name: &haste_core::Identifier,
        blocks: &haste_data::BlockStates,
        state: haste_core::BlockStateId,
        registries: &haste_data::Registries,
        spin: (f32, f32),
    ) -> haste_render::BlockEntityKind {
        use haste_render::blockentity::{BrushableItem, CampfireItems, ShelfItems, VaultDisplay};
        let payload = matches!(
            kind,
            haste_render::BlockEntityKind::Campfire(_)
                | haste_render::BlockEntityKind::Spawner(_)
                | haste_render::BlockEntityKind::Vault(_)
                | haste_render::BlockEntityKind::Shelf(_)
                | haste_render::BlockEntityKind::Brushable(_)
        );
        if !payload {
            return kind;
        }
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, cached)) = self.item_blocks.get(&pos) {
            if (*k, *len) == key {
                return cached.clone();
            }
        }
        let parsed = match kind {
            // The campfire's and the shelf's items, by slot (`ContainerHelper.loadAllItems`).
            haste_render::BlockEntityKind::Campfire(_) => {
                let mut items = [None; 4];
                parse_items_into(&be.nbt, registries, &mut items);
                haste_render::BlockEntityKind::Campfire(CampfireItems { items })
            }
            haste_render::BlockEntityKind::Shelf(_) => {
                let mut items = [None; 3];
                parse_items_into(&be.nbt, registries, &mut items);
                let align = read_nbt(&be.nbt)
                    .and_then(|doc| doc.root_tag.get_bool("align_items_to_bottom"))
                    .unwrap_or(false);
                haste_render::BlockEntityKind::Shelf(ShelfItems {
                    items,
                    align_to_bottom: align,
                })
            }
            // The vault's displayed item lives under `shared_data.display_item`; it draws only
            // when the shared data has one (vanilla `VaultBlockEntity.Client
            // .shouldDisplayActiveEffects` is `hasDisplayItem`).
            haste_render::BlockEntityKind::Vault(_) => {
                match parse_item_stack_path(&be.nbt, &["shared_data", "display_item"], registries) {
                    Some((item, count)) => haste_render::BlockEntityKind::Vault(VaultDisplay {
                        item,
                        count,
                        spin: haste_render::TickPair {
                            start: spin.0,
                            end: spin.1,
                        },
                    }),
                    None => kind,
                }
            }
            // The brushable block's item (its `item` tag) and the block state's `dusted`; the
            // face it is brushed from is not in the NBT until it has been brushed, so it stays
            // `None` and the item is drawn only while that face is known.
            haste_render::BlockEntityKind::Brushable(_) => {
                let dusted = blocks
                    .property_value(state, "dusted")
                    .and_then(|v| v.parse::<u8>().ok())
                    .unwrap_or(0);
                match (
                    parse_item_stack_path(&be.nbt, &["item"], registries),
                    // Vanilla stores `hit_direction` as `Direction.LEGACY_ID_CODEC`, a byte of
                    // the direction's 3D data value (`BrushableBlockEntity.getUpdateTag`); read
                    // it as any numeric, like vanilla's lenient `ValueInput`.
                    read_nbt(&be.nbt)
                        .and_then(|doc| doc.root_tag.get_numeric_short("hit_direction"))
                        .and_then(|v| haste_core::Direction::from_ordinal(v as u8)),
                ) {
                    (Some((item, _)), hit_direction) => {
                        haste_render::BlockEntityKind::Brushable(BrushableItem {
                            item,
                            hit_direction,
                            dusted,
                        })
                    }
                    (None, _) => kind,
                }
            }
            // The spawner's display mob: the model from `SpawnData`'s (a trial spawner's
            // `spawn_data`'s) entity id, and the spin the game tick keeps.
            haste_render::BlockEntityKind::Spawner(_) => {
                let trial =
                    type_name.namespace() == "minecraft" && type_name.path() == "trial_spawner";
                // A trial spawner has no display entity outside the states with a spinning mob
                // (vanilla `TrialSpawnerStateData.getOrCreateDisplayEntity`).
                let spinning = !trial
                    || matches!(
                        blocks
                            .property_value(state, "trial_spawner_state")
                            .unwrap_or(""),
                        "waiting_for_players" | "active"
                    );
                let entity = spinning
                    .then(|| spawner_entity_id(&be.nbt, trial))
                    .flatten()
                    .and_then(|id| registries.index("minecraft:entity_type", &id))
                    .and_then(haste_entity::registry::model_for);
                match entity {
                    Some(model) => haste_render::BlockEntityKind::Spawner(
                        haste_render::blockentity::SpawnerDisplay {
                            model: Some(model.clone()),
                            spin: haste_render::TickPair {
                                start: spin.0,
                                end: spin.1,
                            },
                            scale: haste_render::blockentity::spawner_scale(&model),
                        },
                    ),
                    None => kind,
                }
            }
            other => other,
        };
        self.item_blocks.insert(pos, (key.0, key.1, parsed.clone()));
        parsed
    }

    /// The parsed decorated pot sherds at `pos` (`DecoratedPotBlockEntity`'s `sherds` NBT).
    /// Cached like the banner patterns: re-parsed only when the payload changes.
    fn pot_sherds(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
    ) -> Option<Arc<haste_render::blockentity::PotSherds>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, data)) = self.pot_sherds_cache.get(&pos) {
            if (*k, *len) == key {
                return Some(data.clone());
            }
        }
        let parsed = Arc::new(parse_pot_sherds(&be.nbt));
        self.pot_sherds_cache
            .insert(pos, (key.0, key.1, parsed.clone()));
        Some(parsed)
    }

    /// The parsed sign text at `pos` (`SignBlockEntity`'s `front_text`/`back_text`), from the
    /// block entity's NBT. Cached: the parse (and the wrap to the sign's line width) runs only
    /// when the payload changes; the returned handle is shared with the tick's instances.
    /// The font comes from the renderer through `Shared::sign_text_metrics` (set on the
    /// first frame); until then signs draw without text, retried every tick.
    fn sign_text(
        &mut self,
        shared: &Shared,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        hanging: bool,
    ) -> Option<Arc<haste_render::blockentity::SignTextData>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, text)) = self.sign_texts.get(&pos) {
            if (*k, *len) == key {
                return Some(text.clone());
            }
        }
        let metrics = shared.sign_text_metrics.get().cloned()?;
        let (_line_height, max_width) = haste_render::blockentity::sign_line_metrics_pub(hanging);
        let parsed = parse_sign_nbt(&be.nbt, &*metrics, max_width)?;
        self.sign_texts.insert(pos, (key.0, key.1, parsed.clone()));
        Some(parsed)
    }

    /// The beacon's beam sections at `pos`: vanilla's own client-side walk
    /// (`BeaconBlockEntity.tick` over the stained glass above), done in one pass and re-walked
    /// whenever the column above the beacon changes (the key is a hash of the column's states;
    /// vanilla re-checks 10 blocks a tick, we need only the result).
    fn beacon_data(
        &mut self,
        pos: BlockPos,
        world: &World,
        blocks: &haste_data::BlockStates,
        data: &haste_data::GameData,
    ) -> Option<Arc<haste_render::BeamData>> {
        use haste_core::view::{BlockPhysics as _, BlockView as _};
        // The column's states from the beacon block itself (vanilla's walk starts at it: the
        // beacon is a white `BeaconBeamBlock`, so it seeds the first section) up to the
        // heightmap top, hashed as the cache key.
        let top = world
            .chunk(pos.chunk())
            .and_then(|c| c.height_at((pos.x & 15) as u8, (pos.z & 15) as u8))
            .unwrap_or(pos.y);
        let mut key = 0u64;
        let states: Vec<_> = (pos.y..=top)
            .map(|y| {
                let s = world.block_state(BlockPos::new(pos.x, y, pos.z));
                key = key.wrapping_mul(31).wrapping_add(s.0 as u64);
                s
            })
            .collect();
        if let Some((k, data)) = self.beacon_beams.get(&pos) {
            if *k == key {
                return Some(data.clone());
            }
        }
        // Vanilla's walk: the first one or two stained-glass blocks start sections (a colour
        // change beyond that starts one in the averaged colour), any other block with light
        // dampening 15 (except bedrock) ends the beam, and every further block extends the
        // last section.
        let mut sections: Vec<haste_render::BeamSection> = Vec::new();
        for &s in &states {
            let block = blocks.block_of(s);
            if let Some(color) = beacon_beam_color(block) {
                match sections.len() {
                    0 | 1 => sections.push(haste_render::BeamSection { color, height: 1 }),
                    _ => {
                        let last_color = sections.last().unwrap().color;
                        if last_color == color {
                            sections.last_mut().unwrap().height += 1;
                        } else {
                            sections.push(haste_render::BeamSection {
                                color: average_argb(last_color, color),
                                height: 1,
                            });
                        }
                    }
                }
            } else {
                let ends = match sections.last() {
                    None => true,
                    Some(_) => {
                        // Vanilla: `state.getLightDampening() >= 15 && !state.is(bedrock)` —
                        // the oracle's opaque flag is the same fully-light-blocking set.
                        data.is_opaque_full(s)
                            && block != &haste_core::Identifier::minecraft("bedrock")
                    }
                };
                if ends {
                    sections.clear();
                    break;
                }
                sections.last_mut().unwrap().height += 1;
            }
        }
        let data = Arc::new(haste_render::BeamData { sections });
        self.beacon_beams.insert(pos, (key, data.clone()));
        Some(data)
    }

    /// The end gateway's age at `pos`: its NBT `Age` plus the ticks since the payload was
    /// first seen (the age ticks on client-side, vanilla `beamAnimationTick`, since the
    /// server sends the NBT only once). A new payload restarts from its own age.
    fn gateway_age(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        world_age: i64,
    ) -> i64 {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        let entry = *self
            .gateway_ages
            .entry(pos)
            .or_insert_with(|| (key.0, key.1, world_age, parse_gateway_age(&be.nbt)));
        let (k, len, seen_at, age0) = entry;
        if (k, len) == key {
            age0 + (world_age - seen_at).max(0)
        } else {
            // A new payload: restart from its own age.
            let age = parse_gateway_age(&be.nbt);
            self.gateway_ages
                .insert(pos, (key.0, key.1, world_age, age));
            age
        }
    }

    /// The end gateway's visible faces and beam at `pos`: the faces from the neighbouring
    /// blocks (vanilla `Block.shouldRenderFace`: an opaque neighbour hides the face unless it
    /// is the gateway block itself), the beam from the block entity's NBT age and cooldown
    /// (vanilla `TheEndGatewayRenderer.extractRenderState`: spawning under age 200 in magenta
    /// to the build limit, cooling down for 40 ticks in purple over 50 blocks).
    #[allow(clippy::too_many_arguments)]
    fn gateway_data(
        &mut self,
        pos: BlockPos,
        world: &World,
        blocks: &haste_data::BlockStates,
        data: &haste_data::GameData,
        be: &haste_core::chunk::BlockEntityData,
        world_age: i64,
    ) -> (u8, Option<haste_render::GatewayBeam>) {
        use haste_core::view::{BlockPhysics as _, BlockView as _};
        let mut faces = 0u8;
        let state = world.block_state(pos);
        let own = blocks.block_of(state).clone();
        for d in haste_core::Direction::ALL {
            let neighbour = world.block_state(pos.offset(d));
            let same = *blocks.block_of(neighbour) == own;
            let opaque = data.is_opaque_full(neighbour);
            if !same && opaque {
                continue;
            }
            faces |= 1 << d as u8;
        }
        let max_y = world.dimension().min_y + world.dimension().height - 1;
        let age = self.gateway_age(pos, be, world_age);
        (faces, gateway_beam_at(age, 0.0, max_y))
    }

    /// The moving piston's data at `pos` from the block entity's NBT (vanilla
    /// `PistonMovingBlockEntity.loadAdditional`): the moved block state, facing, progress,
    /// extending and source flags. The progress is the game thread's tick (vanilla advances
    /// it 0.5 per tick to 1.0, then the block entity is gone); this stores the anim's raw
    /// tick pair for the render thread to fold by the partial tick (a fixed pair without an
    /// anim entry lerps to itself).
    fn piston_data(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        anim_map: &HashMap<BlockPos, haste_render::BlockEntityAnim>,
    ) -> Option<Arc<haste_render::PistonData>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, data)) = self.piston_cache.get(&pos) {
            if (*k, *len) == key {
                if let Some(data) = data {
                    // The progress ticks on: take the anim's raw tick pair (the render thread
                    // folds the partial tick into its ends).
                    let mut d = (**data).clone();
                    d.progress = anim_map
                        .get(&pos)
                        .map(|a| match a {
                            haste_render::BlockEntityAnim::Piston { progress } => *progress,
                            _ => d.progress,
                        })
                        .unwrap_or(d.progress);
                    return Some(Arc::new(d));
                }
                return None;
            }
        }
        let parsed = Arc::new(parse_piston_nbt(&be.nbt));
        self.piston_cache
            .insert(pos, (key.0, key.1, Some(parsed.clone())));
        Some(parsed)
    }
}

/// A block state `facing` property value as a `Direction`.
fn direction_of_property(v: &str) -> Option<Direction> {
    Direction::ALL.iter().find(|d| d.name() == v).copied()
}

/// Vanilla `Direction.toYRot`: south 0, west 90, north 180, east 270.
fn to_y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        Direction::Down | Direction::Up => 0.0,
    }
}

/// The yaw a sign/banner turns by: the block state's `rotation` segment
/// (`RotationSegment.convertToDegrees`: `segment * 22.5`, wrapped from 180 to -180), or the
/// wall `facing`'s `toYRot()` when there is no rotation property.
fn rotation_or_facing(
    blocks: &haste_data::BlockStates,
    state: haste_core::BlockStateId,
    facing: Direction,
) -> f32 {
    match blocks.property_value(state, "rotation") {
        Some(rot) => rot.parse::<i32>().map_or(0.0, |r| {
            let d = r.rem_euclid(16) as f32 * 22.5;
            if d >= 180.0 {
                d - 360.0
            } else {
                d
            }
        }),
        None => to_y_rot(facing),
    }
}

/// The pottery pattern a sherd item names (vanilla's item
/// `minecraft:provides_pottery_pattern` component against `DecoratedPotPattern`'s asset id):
/// `minecraft:angler_pottery_sherd` -> `minecraft:angler`. `minecraft:brick` (the blank sherd)
/// has none. Items from mods whose names do not follow the vanilla pattern draw the blank side.
fn sherd_pattern(item: &haste_core::Identifier) -> Option<haste_core::Identifier> {
    if item.namespace() != "minecraft" {
        return None;
    }
    let path = item.path().strip_suffix("_pottery_sherd")?;
    Some(haste_core::Identifier::minecraft(path))
}

/// A decorated pot's four sherds from its NBT (`DecoratedPotBlockEntity`'s `sherds`, a record
/// of optional item stack templates keyed `back`, `left`, `right`, `front`; the item is either
/// an `{id, count}` compound or a plain id string). Sides without a sherd are `None`.
fn parse_pot_sherds(nbt: &[u8]) -> haste_render::blockentity::PotSherds {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    use haste_protocol::nbt::NbtTag;
    let side = |c: &haste_protocol::nbt::NbtCompound, key: &str| match c.get(key) {
        // Vanilla writes each sherd as an item stack template; older payloads (and lenient
        // readers) allow the plain item id.
        Some(NbtTag::Compound(item)) => item.get_string("id").and_then(|s| {
            haste_core::Identifier::parse(s)
                .ok()
                .as_ref()
                .and_then(sherd_pattern)
        }),
        Some(NbtTag::String(s)) => haste_core::Identifier::parse(s)
            .ok()
            .as_ref()
            .and_then(sherd_pattern),
        _ => None,
    };
    let mut out = haste_render::blockentity::PotSherds::default();
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let Ok(doc) = haste_protocol::nbt::Nbt::read_unnamed(&mut helper) else {
        return out;
    };
    if let Some(c) = doc.root_tag.get_compound("sherds") {
        out.front = side(c, "front");
        out.back = side(c, "back");
        out.left = side(c, "left");
        out.right = side(c, "right");
    }
    out
}

/// A moving piston's data from its NBT (vanilla `PistonMovingBlockEntity.loadAdditional`:
/// `blockState`, `facing`, `progress`, `extending`, `source`). Unreadable or missing fields
/// draw nothing (vanilla's `movedState` defaults to air, which also draws nothing).
fn parse_piston_nbt(nbt: &[u8]) -> haste_render::PistonData {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let Ok(doc) = haste_protocol::nbt::Nbt::read_unnamed(&mut helper) else {
        return air_piston();
    };
    let root = &doc.root_tag;
    // The state string carries its properties: `block[facing=north,...]`; keep it whole (the
    // renderer parses it into the blockstate's variants).
    let block_state = root
        .get_string("blockState")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let facing = root
        .get_int("facing")
        .and_then(|i| u8::try_from(i).ok())
        .and_then(haste_core::Direction::from_ordinal)
        .unwrap_or(haste_core::Direction::Down);
    let progress = root.get_float("progress").unwrap_or(0.0);
    let extending = root.get_bool("extending").unwrap_or(false);
    let source = root.get_bool("source").unwrap_or(false);
    haste_render::PistonData {
        block_state,
        progress: haste_render::TickPair::fixed(progress),
        facing,
        extending,
        source,
    }
}

/// The piston data for a block entity without a moved block (draws nothing).
fn air_piston() -> haste_render::PistonData {
    haste_render::PistonData {
        block_state: String::new(),
        progress: haste_render::TickPair::fixed(0.0),
        facing: haste_core::Direction::Down,
        extending: false,
        source: false,
    }
}

/// A stained-glass block's (or the beacon's own) beam colour (vanilla `BeaconBeamBlock.getColor`
/// -> `DyeColor.getTextureDiffuseColor`), or `None` for other blocks.
fn beacon_beam_color(block: &haste_core::Identifier) -> Option<u32> {
    const GLASS: [(&str, u32); 16] = [
        ("white_stained_glass", 0),
        ("orange_stained_glass", 1),
        ("magenta_stained_glass", 2),
        ("light_blue_stained_glass", 3),
        ("yellow_stained_glass", 4),
        ("lime_stained_glass", 5),
        ("pink_stained_glass", 6),
        ("gray_stained_glass", 7),
        ("light_gray_stained_glass", 8),
        ("cyan_stained_glass", 9),
        ("purple_stained_glass", 10),
        ("blue_stained_glass", 11),
        ("brown_stained_glass", 12),
        ("green_stained_glass", 13),
        ("red_stained_glass", 14),
        ("black_stained_glass", 15),
    ];
    if block.namespace() != "minecraft" {
        return None;
    }
    // The beacon block itself is a white beam block (vanilla `BeaconBlock.getColor`).
    let color = if block.path() == "beacon" {
        0
    } else {
        GLASS
            .iter()
            .find(|(name, _)| *name == block.path())
            .map(|(_, dye)| *dye)?
    };
    Some(haste_render::blockentity::DYE_DIFFUSE_COLOR[color as usize])
}

/// Vanilla `ARGB.average`: the per-channel mean of two colours.
fn average_argb(a: u32, b: u32) -> u32 {
    let ch = |s: u32| (((a >> s) & 0xFF).wrapping_add((b >> s) & 0xFF)) / 2;
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// Reads a block entity's NBT payload (the network variant `read_unnamed` reads). `None` when
/// the payload does not parse, like vanilla's empty stack.
fn read_nbt(nbt: &[u8]) -> Option<haste_protocol::nbt::Nbt> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()
}

/// An item stack NBT's registry index and count: vanilla `ItemStack`'s `id` (a registry id in
/// the namespace's own form) and `count`. `None` for an id we cannot resolve.
fn parse_item_stack(
    c: &haste_protocol::nbt::NbtCompound,
    registries: &haste_data::Registries,
) -> Option<(haste_core::RegistryIndex, u32)> {
    let id = haste_core::Identifier::parse(c.get_string("id")?).ok()?;
    let item = registries.index("minecraft:item", &id)?;
    let count = c.get_int("count").unwrap_or(1).max(1) as u32;
    Some((item, count))
}

/// The item stack under a chain of NBT keys (`shared_data.display_item` for a vault, `item`
/// for a brushable block), as its registry index and count.
fn parse_item_stack_path(
    nbt: &[u8],
    path: &[&str],
    registries: &haste_data::Registries,
) -> Option<(haste_core::RegistryIndex, u32)> {
    let doc = read_nbt(nbt)?;
    let mut current = &doc.root_tag;
    for key in path {
        current = current.get_compound(key)?;
    }
    parse_item_stack(current, registries)
}

/// The items of a slot-addressed container NBT (`Items`, vanilla `ContainerHelper
/// .loadAllItems`), written into the caller's slot array; an item whose `Slot` is outside the
/// array is dropped rather than panicking.
fn parse_items_into(
    nbt: &[u8],
    registries: &haste_data::Registries,
    items: &mut [Option<haste_core::RegistryIndex>],
) {
    let Some(doc) = read_nbt(nbt) else {
        return;
    };
    let Some(list) = doc.root_tag.get_list("Items") else {
        return;
    };
    for entry in list {
        let haste_protocol::nbt::NbtTag::Compound(c) = entry else {
            continue;
        };
        let slot = c.get_byte("Slot").unwrap_or(-1);
        if slot < 0 || slot as usize >= items.len() {
            continue;
        }
        if let Some((item, _)) = parse_item_stack(c, registries) {
            items[slot as usize] = Some(item);
        }
    }
}

/// The entity id a spawner's NBT names (`BaseSpawner`'s `SpawnData.entity.id`; a trial
/// spawner's `spawn_data.entity.id`), as a registry id.
fn spawner_entity_id(nbt: &[u8], trial: bool) -> Option<haste_core::Identifier> {
    let doc = read_nbt(nbt)?;
    let data = if trial {
        doc.root_tag.get_compound("spawn_data")?
    } else {
        &doc.root_tag
    };
    let spawn = if trial {
        data
    } else {
        data.get_compound("SpawnData")?
    };
    let entity = spawn.get_compound("entity")?;
    haste_core::Identifier::parse(entity.get_string("id")?).ok()
}

/// A banner's patterns from its NBT (`BannerBlockEntity`'s `patterns`: a list of
/// `{pattern: <registry id>, color: <dye name>}`) plus the base colour from the block id
/// (`<colour>_banner`, vanilla `BannerBlock.getColor`). `None` when the NBT does not parse;
/// a banner without a `patterns` tag is the plain base colour.
fn parse_banner_nbt(
    nbt: &[u8],
    block: &haste_core::Identifier,
) -> Option<Arc<haste_render::blockentity::BannerData>> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    use haste_protocol::nbt::NbtTag;
    const DYES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let dye = |name: &str| {
        DYES.iter()
            .position(|d| *d == name)
            .map(|i| i as u8)
            .unwrap_or(15)
    };
    // The base colour: `<colour>_banner` / `<colour>_wall_banner` -> the dye id.
    let base = block
        .path()
        .strip_suffix("_wall_banner")
        .or_else(|| block.path().strip_suffix("_banner"))
        .and_then(|c| DYES.iter().position(|d| *d == c))
        .map_or(15, |i| i as u8);
    let mut data = haste_render::blockentity::BannerData {
        base,
        layers: Vec::new(),
    };
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let doc = haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()?;
    if let Some(patterns) = doc.root_tag.get_list("patterns") {
        for layer in patterns {
            let NbtTag::Compound(c) = layer else {
                continue;
            };
            // The pattern is the registry id (a string) or an inline `{asset_id: ...}` (a
            // compound under `pattern`, or at the layer's top level for leniency).
            let pattern = match c.get("pattern") {
                Some(NbtTag::String(s)) => haste_core::Identifier::parse(s).ok()?,
                Some(NbtTag::Compound(p)) => {
                    haste_core::Identifier::parse(p.get_string("asset_id")?).ok()?
                }
                _ => c
                    .get_string("asset_id")
                    .and_then(|s| haste_core::Identifier::parse(s).ok())?,
            };
            let color = c.get_string("color").map(dye).unwrap_or(15);
            data.layers
                .push(haste_render::blockentity::BannerLayer { pattern, color });
        }
    }
    Some(Arc::new(data))
}

/// One sign side from its NBT compound (vanilla `SignText.CODEC`): `messages` (4 components as
/// plain strings or `{"text": ...}` compounds), `color` (a `DyeColor` name) and
/// `has_glowing_text`. Each line is wrapped to `max_width` like vanilla's render messages
/// (`Font.split`, first wrapped line only). Missing fields keep vanilla's defaults (black,
/// not glowing, empty).
fn parse_sign_side(
    c: &haste_protocol::nbt::NbtCompound,
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> haste_render::blockentity::SignSideText {
    let mut lines: [String; 4] = Default::default();
    if let Some(messages) = c.get_list("messages") {
        for (i, m) in messages.iter().take(4).enumerate() {
            let text = match m {
                haste_protocol::nbt::NbtTag::String(s) => s.to_string(),
                haste_protocol::nbt::NbtTag::Compound(cc) => {
                    cc.get_string("text").unwrap_or_default().to_string()
                }
                _ => String::new(),
            };
            lines[i] = first_wrapped_line(&text, metrics, max_width).to_string();
        }
    }
    const DYES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let color = c
        .get_string("color")
        .and_then(|n| DYES.iter().position(|d| *d == n))
        .map_or(15, |i| i as u8);
    let glowing = c.get_bool("has_glowing_text").unwrap_or(false);
    haste_render::blockentity::SignSideText {
        lines,
        color,
        glowing,
    }
}

/// The first wrapped line of `text` at `max_width`, vanilla `StringSplitter.splitLines` for a
/// plain string: characters accumulate their widths; past the limit the line breaks at the
/// last space (dropping it), or right before the character that overflowed when there is
/// none; a first character wider than the limit still yields one character (the
/// `hadNonZeroWidthChar` guard). Newlines break there.
fn first_wrapped_line<'a>(
    text: &'a str,
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> &'a str {
    let max_width = max_width.max(1);
    let mut width = 0i32;
    let mut had_non_zero = false;
    let mut last_space: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        match ch {
            '\n' => return &text[..i],
            ' ' => last_space = Some(i),
            _ => {}
        }
        let char_width = metrics.width(ch.encode_utf8(&mut [0; 4]));
        width += char_width;
        if !had_non_zero || width <= max_width {
            had_non_zero |= char_width != 0;
        } else {
            return match last_space {
                // Break at the last space, dropping it.
                Some(s) => &text[..s],
                // No space: break before this character (at least one char: the guard above
                // always accepts the first non-zero-width one).
                None => &text[..i],
            };
        }
    }
    text
}

/// The whole sign's text (`front_text` / `back_text`), or `None` when the NBT does not parse.
fn parse_sign_nbt(
    nbt: &[u8],
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> Option<Arc<haste_render::blockentity::SignTextData>> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let doc = haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()?;
    let root = &doc.root_tag;
    let side = |name: &str| {
        root.get_compound(name)
            .map(|c| parse_sign_side(c, metrics, max_width))
            .unwrap_or_else(empty_sign_side)
    };
    Some(Arc::new(haste_render::blockentity::SignTextData {
        front: side("front_text"),
        back: side("back_text"),
    }))
}

/// Vanilla `SignText.EMPTY`: four empty lines, black, not glowing.
fn empty_sign_side() -> haste_render::blockentity::SignSideText {
    haste_render::blockentity::SignSideText {
        lines: std::array::from_fn(|_| String::new()),
        color: 15,
        glowing: false,
    }
}

/// Where a double chest's other half is (vanilla `ChestBlock.getConnectedDirection`:
/// the LEFT half's neighbour is clockwise of the facing, the RIGHT half's counter-clockwise).
fn connected_direction(facing: Direction, half: haste_render::ChestHalf) -> Direction {
    let clockwise = |d: Direction| match d {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        Direction::Down | Direction::Up => d,
    };
    match half {
        haste_render::ChestHalf::Left => clockwise(facing),
        _ => clockwise(facing).opposite(),
    }
}

/// Whether `pos` holds the matching other half of our double chest: a chest/trapped chest of
/// the opposite half, the same facing, whose connected direction points back at us (vanilla's
/// `DoubleBlockCombiner` accepts exactly that neighbour).
fn is_double_chest_half(
    blocks: &haste_data::BlockStates,
    world: &World,
    pos: BlockPos,
    our_half: haste_render::ChestHalf,
    facing: Direction,
) -> bool {
    use haste_core::view::BlockView as _;
    let state = world.block_state(pos);
    let block = blocks.block_of(state);
    if block.namespace() != "minecraft" || !matches!(block.path(), "chest" | "trapped_chest") {
        return false;
    }
    let neighbour_half = match blocks.property_value(state, "type") {
        Some("left") => haste_render::ChestHalf::Left,
        Some("right") => haste_render::ChestHalf::Right,
        _ => return false,
    };
    let Some(neighbour_facing) = blocks
        .property_value(state, "facing")
        .and_then(direction_of_property)
    else {
        return false;
    };
    neighbour_facing == facing
        && neighbour_half == our_half.other()
        && connected_direction(neighbour_facing, neighbour_half).opposite()
            == connected_direction(facing, our_half)
}

/// The end gateway's `Age` from its NBT (vanilla `EndGatewayBlockEntity.loadAdditional`,
/// default 0).
fn parse_gateway_age(nbt: &[u8]) -> i64 {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper)
        .ok()
        .and_then(|doc| doc.root_tag.get_long("Age"))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
