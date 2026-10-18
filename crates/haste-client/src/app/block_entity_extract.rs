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
use haste_world::World;

use banner::parse_banner_nbt;
use beacon::{average_argb, beacon_beam_color};
use chest::{connected_direction, is_double_chest_half};
use facing::{direction_of_property, rotation_or_facing};
use gateway::{gateway_beam_at, parse_gateway_age};
use items::{parse_item_stack_path, parse_items_into, read_nbt, spawner_entity_id};
use piston::parse_piston_nbt;
use pot::parse_pot_sherds;
use sign::parse_sign_nbt;

use crate::game::Shared;

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

#[cfg(test)]
mod tests;
