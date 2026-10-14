//! Interaction (player workstream): hotbar, crosshair target, breaking, placing/using, attacking,
//! picking, and the client-side block predictions that go with them.
//!
//! Ported from vanilla 26.3 (the decompiled client), method by method, in vanilla's order per
//! tick (`Minecraft.tick`, then `Minecraft.handleKeybinds`):
//! - `Minecraft.tick`: `rightClickDelay` counts down before the key handling, `missTime` after;
//! - `Minecraft.pick` / `LocalPlayer.pick` / `ProjectileUtil.getEntityHitResult`: the crosshair
//!   target — block outlines within `block_interaction_range`, entities within
//!   `entity_interaction_range` (clamped to the block distance when a block is hit), and an
//!   entity wins only when it is strictly closer; only `Entity.isPickable` entities are
//!   searched (EntitySelector.CAN_BE_PICKED), never ourselves — an item entity, an experience
//!   orb, an arrow and the player itself are never attack or interact targets;
//! - `Minecraft.startAttack`: one click — entity: `attack` + `punch`; block: start breaking +
//!   `punch`; nothing: a miss (`missTime` 10 outside creative) + `punch`; a creative or
//!   otherwise instant break returns `endAttack`, which skips `continueAttack` this tick;
//! - `Minecraft.continueAttack` while the button stays down: a block target keeps breaking
//!   (with a `punch` every tick), anything else stops the break, and after a miss nothing
//!   happens for `missTime` ticks. `MultiPlayerGameMode.startDestroyBlock` /
//!   `continueDestroyBlock`: `destroyDelay` 5 between blocks, the creative instant break, the
//!   5-tick delay after a break, CHANGE_DESTROY_DIRECTION and the 151-tick stone break;
//! - `Minecraft.startUseItem`: `rightClickDelay` 4, then per hand (main first, then off hand):
//!   entity interact, block use, item use; nothing at all while a block is being destroyed;
//!   while the button is held, one use every 4 ticks (`handleKeybinds`); the item in progress
//!   blocks both attack and use, and releasing the button sends
//!   `MultiPlayerGameMode.releaseUsingItem`'s `release_use_item`.
//!
//! Per tick (vanilla's order): hotbar, target, then the key handling (clicks, held use,
//! continueAttack, `missTime`--), then pick block, then drop / off-hand swap.
//!
//! Predictions: a block we break is set to air locally at once (`take_world_edits`). Until the
//! server acknowledges our sequence number (`block_changed_ack`) its block updates for that
//! position are only recorded; on the ack the last state the server told us (or, if it said
//! nothing, the state from before our prediction) is put back. Observed on the vanilla server:
//! a break it rejects gets an ack and no block update, so without the revert our world would keep
//! a block the server still has.
// MERGE(effects side):
// MERGE(main side):
use std::collections::{HashMap, HashSet};
// MERGE end
use std::sync::Arc;

use glam::DVec3;
use haste_core::entity::ItemStackData;
use haste_core::input::InteractInput;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, Direction, EntityId, Identifier, RegistryIndex};
use haste_physics::mining::{BlockTags, MiningParams, ToolTable};
use haste_physics::raycast::{self, BlockHit};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::{ClientboundPlay, ServerboundPlay};

use super::LocalPlayer;
use haste_core::game::GameMode;

/// Ticks between two broken blocks while attack stays held (`destroyDelay`).
pub const DESTROY_DELAY: u8 = 5;
/// Ticks between repeated uses while use stays held (`rightClickDelay`).
pub const USE_DELAY: u8 = 4;
/// Ticks the held attack freezes after a miss outside creative (`missTime`).
pub const MISS_TIME: u8 = 10;

/// `player_action` statuses. 26.3 inserted `CHANGE_DESTROY_DIRECTION` at 1, moving abort to 2
/// and finish to 3 (enum order read from the server jar's `ServerboundPlayerActionPacket$Action`
/// and confirmed on the vanilla server: status 2 is handled as an abort). The wiki's older order
/// (0 start, 1 abort, 2 finish) makes every survival break an abort.
pub mod action {
    pub const START_DESTROY: i32 = 0;
    pub const CHANGE_DESTROY_DIRECTION: i32 = 1;
    pub const ABORT_DESTROY: i32 = 2;
    pub const STOP_DESTROY: i32 = 3;
    pub const DROP_ALL_ITEMS: i32 = 4;
    pub const DROP_ITEM: i32 = 5;
    pub const RELEASE_USE_ITEM: i32 = 6;
    pub const SWAP_ITEM_WITH_OFFHAND: i32 = 7;
    pub const STAB: i32 = 8;
}

/// Interaction-related attributes (server values; vanilla defaults until it sends them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractAttributes {
    pub block_break_speed: f64,
    pub mining_efficiency: f64,
    pub submerged_mining_speed: f64,
    pub block_reach: f64,
    pub entity_reach: f64,
    /// `attack_speed`: the equip progress returns over 20 / this many ticks.
    pub attack_speed: f64,
}

impl Default for InteractAttributes {
    fn default() -> Self {
        // wiki "Attribute" defaults.
        Self {
            block_break_speed: 1.0,
            mining_efficiency: 0.0,
            submerged_mining_speed: 0.2,
            block_reach: 4.5,
            entity_reach: 3.0,
            attack_speed: crate::hand::DEFAULT_ATTACK_SPEED,
        }
    }
}

/// What the interaction step reads besides the world: game data and nearby entity boxes.
pub struct InteractEnv<'a> {
    pub data: &'a haste_data::GameData,
    /// Hit boxes of the entities that can be targeted (not ourselves).
    pub entities: &'a [(EntityId, Aabb)],
    /// `minecraft:entity_type` registry index per entry of `entities` (same order). When it is
    /// short or absent the type is unknown — a mod type — and unknown types are pickable, like
    /// vanilla's living-entity default. Filling this in is the driver's side of the interface
    /// (`docs/INTERFACES.md`: `entity_box` carries the entity_type name); the picker only reads
    /// it.
    pub entity_types: &'a [RegistryIndex],
}

/// What the crosshair is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Block(BlockHit),
    Entity { id: EntityId, location: DVec3 },
}

/// Interaction state of the local player.
#[derive(Default)]
pub struct Interaction {
    pub input: InteractInput,
    last: InteractInput,
    pub target: Option<Target>,
    pub attrs: InteractAttributes,
    /// Block being broken, its face and the held item it started with, progress 0..1.
    destroying: Option<(BlockPos, Direction, ItemStackData)>,
    progress: f32,
    destroy_delay: u8,
    use_delay: u8,
    /// Left/right clicks queued since the last tick (vanilla's `KeyMapping.consumeClick`): each
    /// press counts, even several in one tick. The driver adds one per mouse press; a driver
    /// that only reports the held boolean gets one click on the press edge instead.
    attack_clicks: u8,
    use_clicks: u8,
    /// `missTime` ticks left: 10 after a miss outside creative.
    miss_time: u8,
    /// True while the player is using an item (vanilla sets it when a consumable's use starts;
    /// haste has no consumable-item data yet, so nothing sets it — see `set_using_item`).
    is_using_item: bool,
    /// Last sequence number used for a prediction.
    sequence: i32,
    /// Predicted blocks: (sequence, pos, the server's state: before the prediction, then the
    /// latest block update held back).
    predictions: Vec<(i32, BlockPos, BlockStateId)>,
    world_edits: Vec<(BlockPos, BlockStateId)>,
    /// Hotbar 0..=8 and off hand.
    pub hotbar: [ItemStackData; 9],
    pub offhand: ItemStackData,
// MERGE(effects side):
    /// The item in the head armor slot (player inventory menu slot 5), for the worn item's
    /// camera overlay (`Hud.extractCameraOverlays` reads `getItemBySlot(HEAD)`). The server
    /// sends our own armor through `container_set_slot`/`container_set_content`, not
    /// `set_equipment` (that goes to other players).
    pub head_item: Option<haste_core::RegistryIndex>,
    /// Active effects on us (`minecraft:mob_effect` index), with the vanilla blend state the
    /// screen effects need (`screen_effects`).
    pub effects: crate::screen_effects::Effects,
// MERGE(main side):
    /// Armor, indexed helmet 0, chest 1, legs 2, boots 3 (player inventory menu 5..=8 and
    /// `set_player_inventory` 36..=39). The chest slot answers the glider check.
    pub armor: [ItemStackData; 4],
    /// Item registry indices whose default components carry `minecraft:glider`
    /// (`LivingEntity.canGlideUsing`), resolved from the item component reports in `prepare`.
    pub(crate) glider_items: Option<HashSet<RegistryIndex>>,
    /// Active effects on us: `minecraft:mob_effect` index -> amplifier and remaining ticks.
    pub effects: HashMap<i32, EffectData>,
// MERGE end
    tools: Option<Arc<ToolTable>>,
    tags: Option<BlockTags>,
    effect_ids: Option<EffectIds>,
    /// Night-vision and darkness effect indices for the lightmap (`prepare`).
    pub(crate) lightmap_effect_ids: LightmapEffectIds,
    /// Last blocks we sent actions for (debug logging of their server updates).
    recent: Vec<BlockPos>,
}

#[derive(Clone, Copy, Debug)]
struct EffectIds {
    haste: i32,
    conduit_power: i32,
    mining_fatigue: i32,
    /// The two effects that blend, as vanilla's `MobEffects` builds them: nausea has a 150-tick
    /// fade in and a 20-tick fade out that starts 60 ticks early, darkness a 22-tick one.
    /// Their blend factors are what the screen effects read.
    nausea: i32,
    darkness: i32,
}

/// The `minecraft:mob_effect` indices the lightmap needs (night vision and darkness), resolved
/// from the game data. `None` until [`Interaction::prepare`] finds them (mod registries, or a
/// report without them, leave the lightmap without those effects).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LightmapEffectIds {
    pub(crate) night_vision: Option<i32>,
    pub(crate) darkness: Option<i32>,
}

/// One active effect on us (vanilla `MobEffectInstance`): its amplifier and remaining ticks
/// (`-1` infinite). The lightmap reads the night-vision and darkness durations from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectData {
    pub amplifier: i32,
    pub duration: i32,
}

impl Interaction {
    /// Loads the tool table (once; reads the data generator's item component reports) and the
    /// effect ids.
    pub fn prepare(&mut self, data: &haste_data::GameData) {
        if self.tools.is_none() {
            let t = ToolTable::load_default().unwrap_or_else(|e| {
                tracing::warn!("no tool components (mining speed with bare hands only): {e:#}");
                ToolTable::default()
            });
            self.tools = Some(Arc::new(t));
        }
        if self.glider_items.is_none() {
            // The glider items the reports name, as `minecraft:item` registry indices (the
            // chest stack only carries its index).
            self.glider_items = Some(
                self.tools
                    .as_ref()
                    .map(|t| {
                        t.gliders()
                            .iter()
                            .filter_map(|n| data.registries.index("minecraft:item", n))
                            .collect()
                    })
                    .unwrap_or_default(),
            );
        }
        if self.effect_ids.is_none() {
            let id = |n: &str| {
                data.registries
                    .index("minecraft:mob_effect", &Identifier::minecraft(n))
                    .map(|i| i.0 as i32)
            };
            if let (Some(haste), Some(conduit_power), Some(mining_fatigue), Some(nausea), Some(darkness)) = (
                id("haste"),
                id("conduit_power"),
                id("mining_fatigue"),
                id("nausea"),
                id("darkness"),
            ) {
                self.effect_ids = Some(EffectIds {
                    haste,
                    conduit_power,
                    mining_fatigue,
                    nausea,
                    darkness,
                });
            }
            self.lightmap_effect_ids = LightmapEffectIds {
                night_vision: id("night_vision"),
                darkness: id("darkness"),
            };
        }
    }

    pub fn has_block_tags(&self) -> bool {
        self.tags.is_some()
    }

    /// The block tags the server synced (`update_tags`, registry `minecraft:block`).
    pub fn block_tags(&self) -> Option<&BlockTags> {
        self.tags.as_ref()
    }

    pub fn set_block_tags(&mut self, tags: BlockTags) {
        self.tags = Some(tags);
    }

    pub fn set_tools(&mut self, tools: Arc<ToolTable>) {
        self.tools = Some(tools);
    }

// MERGE(effects side):
    /// The `minecraft:mob_effect` index of nausea, the effect whose blend factor drives the
    /// nausea overlay and the screen warp (`MobEffects.NAUSEA`).
    fn nausea_id(&self) -> Option<i32> {
        self.effect_ids.map(|e| e.nausea)
    }

    /// Whether `effect` is `MobEffects.DARKNESS`, the other effect with a blend duration.
    fn darkness_id(&self, effect: i32) -> bool {
        self.effect_ids.is_some_and(|e| e.darkness == effect)
    }

    /// The nausea blend factor between two frames, 0 without the effect
    /// (`LivingEntity.getEffectBlendFactor`).
    pub fn nausea_factor(&self, partial_tick: f32) -> f32 {
        self.nausea_id()
            .map_or(0.0, |id| self.effects.blend_factor(id, partial_tick))
    }

    /// The nausea blend factor pair (the previous frame's and this tick's), for the frame's
    /// lerp (`BlendState.factorPreviousFrame`, `.factor`).
    pub fn nausea_blend_pair(&self) -> (f32, f32) {
        self.nausea_id()
            .map_or((0.0, 0.0), |id| self.effects.blend_pair(id))
    }

    /// One tick of the active effects (`LivingEntity.tickEffects`: each instance ticks its
    /// duration and its vanilla blend state on the client).
    pub(super) fn tick_effects(&mut self) {
        self.effects.tick();
// MERGE(main side):
    /// Test/script hook for the item-in-use state (vanilla: `player.isUsingItem()`; eating blocks
    /// attack and use and releasing the button then sends `release_use_item`).
    pub fn set_using_item(&mut self, using: bool) {
        self.is_using_item = using;
// MERGE end
    }

    pub(super) fn respawned(&mut self) {
        self.destroying = None;
        self.progress = 0.0;
        self.target = None;
        self.effects.clear();
        self.miss_time = 0;
        self.is_using_item = false;
        self.attack_clicks = 0;
        self.use_clicks = 0;
    }

    fn next_sequence(&mut self) -> i32 {
        self.sequence += 1;
        self.sequence
    }

    /// Predicts `pos` becomes `state` (`None`: left to the server); `old` is our current state.
    fn predict(&mut self, seq: i32, pos: BlockPos, old: BlockStateId, state: Option<BlockStateId>) {
        // An earlier prediction for the same block already holds the server's state.
        let server = self
            .predictions
            .iter()
            .rev()
            .find(|p| p.1 == pos)
            .map_or(old, |p| p.2);
        self.predictions.push((seq, pos, server));
        if let Some(s) = state {
            self.world_edits.push((pos, s));
        }
    }
}

impl LocalPlayer {
    /// This tick's mouse buttons and hotbar selection (applied inside the next tick).
    pub fn set_interact(&mut self, input: InteractInput) {
        self.int.input = input;
    }

// MERGE(effects side):
    /// Whether we are scoping with a spyglass this tick: vanilla's `Player.isScoping` (using an
    /// item, and the item being used is a spyglass). The use state is the held "use" button
    /// (`use_item` is re-sent every 4 ticks while it stays held, so a held button is a use in
    /// progress) and the item is the main hand's.
    pub fn scoping(&self, data: &haste_data::GameData) -> bool {
        if !self.int.last.use_item {
            return false;
        }
        let name = self.main_hand()
            .item
            .and_then(|i| data.registries.name("minecraft:item", i));
        crate::screen_effects::is_spyglass(name)
// MERGE(main side):
    /// Queues one left click for the next tick (vanilla's `keyAttack.clickCount++` /
    /// `KeyMapping.consumeClick`): every press counts, even several within one tick. The input
    /// layer queues one per mouse press on the game thread next to [`LocalPlayer::set_interact`];
    /// a driver that only reports the held boolean still gets one click on the press edge.
    pub fn queue_attack_click(&mut self) {
        self.int.attack_clicks = self.int.attack_clicks.saturating_add(1);
    }

    /// Queues one right click for the next tick (`keyUse.clickCount++`).
    pub fn queue_use_click(&mut self) {
        self.int.use_clicks = self.int.use_clicks.saturating_add(1);
// MERGE end
    }

    /// Block under the crosshair and the face hit.
    pub fn target(&self) -> Option<(BlockPos, Direction)> {
        match self.int.target {
            Some(Target::Block(h)) => Some((h.pos, h.face)),
            _ => None,
        }
    }

    /// Entity under the crosshair.
    pub fn target_entity(&self) -> Option<EntityId> {
        match self.int.target {
            Some(Target::Entity { id, .. }) => Some(id),
            _ => None,
        }
    }

    /// Our own breaking: block and crack stage 0..=9.
    pub fn break_progress(&self) -> Option<(BlockPos, u8)> {
        let (pos, _, _) = self.int.destroying.as_ref()?;
        if self.int.progress <= 0.0 {
            return None;
        }
        Some((*pos, ((self.int.progress * 10.0) as u8).min(9)))
    }

    /// The held main-hand stack.
    pub fn main_hand(&self) -> &ItemStackData {
        &self.int.hotbar[self.held_slot.clamp(0, 8) as usize]
    }

    /// The stack of hand `i` (0 main, 1 off), as `InteractionHand.values()` numbers them.
    fn hand(&self, hand: i32) -> &ItemStackData {
        if hand == 1 {
            &self.int.offhand
        } else {
            self.main_hand()
        }
    }

    /// Predicted block changes to apply to the world now (breaking), and server states released
    /// by `block_changed_ack`.
    pub fn take_world_edits(&mut self) -> std::vec::Drain<'_, (BlockPos, BlockStateId)> {
        self.int.world_edits.drain(..)
    }

    /// A server block update: `true` if it was held back because we predicted that block and the
    /// server has not acknowledged the prediction yet (it is applied on the ack).
    pub fn defer_block_update(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        if self.int.recent.contains(&pos) {
            tracing::debug!(
                "block update {pos:?} -> {} (predicted: {})",
                state.0,
                self.int.predictions.iter().any(|p| p.1 == pos)
            );
        }
        let mut held = false;
        for p in self.int.predictions.iter_mut().filter(|p| p.1 == pos) {
            p.2 = state;
            held = true;
        }
        held
    }

    pub(super) fn handle_interact_packet(&mut self, p: &ClientboundPlay) -> bool {
        use ClientboundPlay as P;
        let me = self.entity_id.0;
        match p {
            P::BlockChangedAck(a) => {
                tracing::debug!("block_changed_ack {}", a.sequence);
                let (done, keep): (Vec<_>, Vec<_>) = self
                    .int
                    .predictions
                    .drain(..)
                    .partition(|p| p.0 <= a.sequence);
                self.int.predictions = keep;
                for (_, pos, server) in done {
                    // The server's state wins, unless a newer prediction still covers the block.
                    if !self.int.predictions.iter().any(|p| p.1 == pos) {
                        self.int.world_edits.push((pos, server));
                    }
                }
            }
            P::ContainerSetContent(c) if c.window_id == 0 => {
                for (i, s) in c.slots.iter().enumerate() {
                    self.set_inventory_menu_slot(i as i32, s);
                }
            }
            P::ContainerSetSlot(c) if c.window_id == 0 => {
                self.set_inventory_menu_slot(c.slot as i32, &c.item)
            }
            P::SetPlayerInventory(s) => match s.slot {
                0..=8 => self.int.hotbar[s.slot as usize] = s.item.clone(),
                // `Inventory` indices: 36 feet, 37 legs, 38 chest, 39 head, 40 off hand.
                36..=39 => self.int.armor[(39 - s.slot) as usize] = s.item.clone(),
                40 => self.int.offhand = s.item.clone(),
                _ => {}
            },
            P::UpdateMobEffect(e) if e.entity_id == me => {
// MERGE(effects side):
                // Vanilla's blend durations per effect (`MobEffects`): only nausea and
                // darkness set them, everything else blends at once.
                let (blend_in, blend_out, blend_out_advance) =
                    if Some(e.effect) == self.int.nausea_id() {
                    use crate::screen_effects as se;
                    (
                        se::NAUSEA_BLEND_IN,
                        se::NAUSEA_BLEND_OUT,
                        se::NAUSEA_BLEND_OUT_ADVANCE,
                    )
                } else if self.int.darkness_id(e.effect) {
                    use crate::screen_effects as se;
                    (se::DARKNESS_BLEND, se::DARKNESS_BLEND, se::DARKNESS_BLEND)
                } else {
                    (0, 0, 0)
                };
                let effect = crate::screen_effects::Effect::new(
                    e.amplifier,
                    e.duration,
                    e.flags & 0x08 != 0,
                    blend_in,
                    blend_out,
                    blend_out_advance,
                );
                self.int.effects.insert(e.effect, effect);
// MERGE(main side):
                self.int.effects.insert(
                    e.effect,
                    EffectData {
                        amplifier: e.amplifier,
                        duration: e.duration,
                    },
                );
// MERGE end
            }
            P::RemoveMobEffect(e) if e.entity_id == me => {
                self.int.effects.remove(e.effect);
            }
            _ => return false,
        }
        true
    }

    /// Slot of the player inventory menu (window 0): 5..=8 armor (helmet, chest, legs, boots),
    /// 36..=44 hotbar, 45 off hand (wiki "Inventory" slot numbering).
    fn set_inventory_menu_slot(&mut self, slot: i32, item: &ItemStackData) {
        match slot {
// MERGE(effects side):
            // Player inventory menu: 5..=8 armor (5 head), 36..=44 hotbar, 45 off hand.
            5 => self.int.head_item = item.item,
// MERGE(main side):
            5..=8 => self.int.armor[(slot - 5) as usize] = item.clone(),
// MERGE end
            36..=44 => self.int.hotbar[(slot - 36) as usize] = item.clone(),
            45 => self.int.offhand = item.clone(),
            _ => {}
        }
    }

    /// Whether the chest armor is a usable glider: vanilla `LivingEntity.canGlide` ->
    /// `canGlideUsing` (a `minecraft:glider` component in the matching equippable slot, not about
    /// to break). The `nextDamageWillBreak` half needs the stack's damage component, which the
    /// client does not decode yet (see NOTES), so a nearly-broken elytra still reads as usable.
    pub fn has_glider(&self) -> bool {
        let chest = &self.int.armor[1];
        let Some(index) = chest.item else {
            return false;
        };
        chest.count > 0
            && self
                .int
                .glider_items
                .as_ref()
                .is_some_and(|s| s.contains(&index))
    }

    fn eye(&self) -> DVec3 {
        self.state.pos + DVec3::new(0.0, self.attrs.eye_height_for(self.state.pose), 0.0)
    }

    /// The crosshair target for a look direction: `Minecraft.pick` ->
    /// `LocalPlayer.raycastHitResult` -> `LocalPlayer.pick`, with
    /// `ProjectileUtil.getEntityHitResult`'s `EntitySelector.CAN_BE_PICKED` filter.
    ///
    /// Block outlines are searched out to the larger of the two interaction ranges and then
    /// filtered back to `block_interaction_range` (`filterHitResult`); the entity segment reaches
    /// the block's distance when a block is hit, else the larger range, and an entity only wins
    /// when it is strictly closer than the block. The winner is filtered to
    /// `entity_interaction_range`, so an entity just out of reach makes the pick a miss rather
    /// than showing the block behind it.
    pub(super) fn pick(
        &self,
        look: (f32, f32),
        world: &dyn BlockView,
        shapes: &dyn CollisionShapes,
        data: &haste_data::GameData,
        entities: &[(EntityId, Aabb)],
        entity_types: &[RegistryIndex],
    ) -> Option<Target> {
        let eye = self.eye();
        let dir = raycast::look_vector(look.0, look.1);
        let a = self.int.attrs;
        let max_dist = a.block_reach.max(a.entity_reach);
        // LocalPlayer.pick: the block raycast reaches the larger range; `filterHitResult`
        // afterwards drops a block beyond `block_interaction_range`.
        let block = raycast::raycast_blocks(eye, dir, max_dist, world, shapes)
            .filter(|b| b.distance <= a.block_reach);
        let block_dist = block.as_ref().map_or(f64::INFINITY, |b| b.distance);
        // ProjectileUtil.getEntityHitResult: the nearest pickable box the segment touches. The
        // segment reaches the block's distance when a block was hit, else the larger range
        // (LocalPlayer.pick clamps maxDistance to the block distance); the winner is filtered
        // back to `entity_interaction_range` (filterHitResult).
        let search_dist = block_dist.min(max_dist);
        let delta = dir * search_dist;
        let mut best: Option<(usize, f64)> = None;
        for (i, (id, b)) in entities.iter().enumerate() {
            if *id == self.entity_id {
                continue; // getEntities(except = the camera entity)
            }
            let name = self.entity_type_name(data, entity_types, i);
            // EntitySelector.CAN_BE_PICKED = Entity::isPickable per type; unknown (mod)
            // types are pickable, like vanilla's default.
            if !name.is_none_or(is_pickable) {
                continue;
            }
            // Entity.getPickRadius: Projectile searches with 1.0 for pickable projectiles.
            let radius = name.and_then(entity_pick_radius).unwrap_or(0.0);
            let bb = b.expand(DVec3::splat(radius));
            // ProjectileUtil: an entity whose box holds the eye is picked at distance 0
            // (canBePickedFromInside).
            if bb.min.x <= eye.x
                && eye.x <= bb.max.x
                && bb.min.y <= eye.y
                && eye.y <= bb.max.y
                && bb.min.z <= eye.z
                && eye.z <= bb.max.z
            {
                if best.is_none_or(|(_, bd)| 0.0 < bd) {
                    best = Some((i, 0.0));
                }
                continue;
            }
            if let Some((t, _, _)) = raycast::clip_box(&bb, eye, delta) {
                let d = search_dist * t;
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
        }
        if let Some((i, d)) = best {
            if d < block_dist {
                // filterHitResult: an entity beyond entity_interaction_range makes the pick a
                // miss (the block behind it is not shown).
                if d <= a.entity_reach {
                    return Some(Target::Entity {
                        id: entities[i].0,
                        location: eye + dir * d,
                    });
                }
                return None;
            }
        }
        block.map(Target::Block)
    }

    /// `minecraft:entity_type` name of `entities[i]`, when the caller has type data.
    fn entity_type_name<'d>(
        &self,
        data: &'d haste_data::GameData,
        entity_types: &[RegistryIndex],
        i: usize,
    ) -> Option<&'d Identifier> {
        let kind = *entity_types.get(i)?;
        data.registries.name("minecraft:entity_type", kind)
    }

    pub(super) fn interact_step(
        &mut self,
        look: (f32, f32),
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let input = self.int.input;
        let last = std::mem::replace(&mut self.int.last, input);
        if self.int.tags.is_none() {
            self.int.tags = Some(BlockTags::default());
        }

        // Minecraft.tick(): rightClickDelay-- happens before the key handling.
        if self.int.use_delay > 0 {
            self.int.use_delay -= 1;
        }

        // 1. Hotbar (handleKeybinds: the keyHotbarSlots loop).
        if let Some(slot) = input.hotbar {
            if slot <= 8 && slot as i32 != self.held_slot {
                self.held_slot = slot as i32;
                out.push(ServerboundPlay::SetCarriedItem(sb::SetCarriedItem {
                    slot: slot as i16,
                }));
                // A new tool restarts breaking (startDestroyBlock compares the held item).
                if let Some((_, face, _)) = self.int.destroying {
                    self.abort_destroy(face, out);
                }
            }
        }

        // 2. The crosshair target (Minecraft.pick).
        self.int.target = if self.game_mode == GameMode::Spectator {
            None
        } else {
            self.pick(
                look,
                world,
                &env.data.shapes,
                env.data,
                env.entities,
                env.entity_types,
            )
        };
        if self.game_mode == GameMode::Spectator {
            self.int.input = InteractInput::default();
            return;
        }

        // 3. Minecraft.handleKeybinds: the click loops, the held use, then continueAttack.
        // `KeyMapping.consumeClick` returns once per queued press, so several clicks in one tick
        // each run. A driver that only reports the held button (no click queue) still gets one
        // click on the press edge, as consumeClick would on a fresh press.
        let mut attack_clicks = std::mem::take(&mut self.int.attack_clicks);
        let mut use_clicks = std::mem::take(&mut self.int.use_clicks);
        if attack_clicks == 0 && input.attack && !last.attack {
            attack_clicks = 1;
        }
        if use_clicks == 0 && input.use_item && !last.use_item {
            use_clicks = 1;
        }
        let mut instant_attack = false;
        if self.int.is_using_item {
            // While an item is in use the clicks are consumed and do nothing; releasing the
            // button releases the item (MultiPlayerGameMode.releaseUsingItem).
            if !input.use_item {
                self.release_using_item(out);
            }
        } else {
            // while (keyAttack.consumeClick()) instantAttack |= startAttack();
            for _ in 0..attack_clicks {
                instant_attack |= self.start_attack(world, env, out);
            }
            // while (keyUse.consumeClick()) startUseItem();
            for _ in 0..use_clicks {
                self.start_use_item(look, out);
            }
        }

        // if (keyUse.isDown() && rightClickDelay == 0 && !player.isUsingItem()) startUseItem();
        if input.use_item && self.int.use_delay == 0 && !self.int.is_using_item {
            self.start_use_item(look, out);
        }

        // continueAttack(screen == null && !instantAttack && keyAttack.isDown() && grabbed).
        if !instant_attack {
            self.continue_attack(input.attack, world, env, out);
        }

        // Minecraft.tick(): missTime-- after the key handling.
        if self.int.miss_time > 0 {
            self.int.miss_time -= 1;
        }

        // 4. Pick block (handleKeybinds: the keyPickItem loop).
        if input.pick_block && !last.pick_block {
            let include_data = false;
            match self.int.target {
                Some(Target::Block(h)) => {
                    out.push(ServerboundPlay::PickItemFromBlock(sb::PickItemFromBlock {
                        pos: h.pos,
                        include_data,
                    }))
                }
                Some(Target::Entity { id, .. }) => out.push(ServerboundPlay::PickItemFromEntity(
                    sb::PickItemFromEntity {
                        entity_id: id.0,
                        include_data,
                    },
                )),
                None => {}
            }
        }
        // 5. Drop (Q / Ctrl+Q) and off-hand swap (F): handleKeybinds' keyDrop / keySwapOffhand
        // loops; no target, pos and face zeroed as vanilla sends it; the server answers with
        // the slot updates, nothing is predicted here.
        if self.health > 0.0 {
            let status = if input.drop_stack && !last.drop_stack {
                Some(action::DROP_ALL_ITEMS)
            } else if input.drop_item && !last.drop_item {
                Some(action::DROP_ITEM)
            } else if input.swap_offhand && !last.swap_offhand {
                Some(action::SWAP_ITEM_WITH_OFFHAND)
            } else {
                None
            };
            if let Some(status) = status {
                out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                    status,
                    pos: BlockPos::new(0, 0, 0),
                    face: 0,
                    sequence: 0,
                }));
            }
        }

        // The press-only fields are consumed; held buttons stay until the caller changes them.
        self.int.input.pick_block = false;
        self.int.input.drop_item = false;
        self.int.input.drop_stack = false;
        self.int.input.swap_offhand = false;
        self.int.input.hotbar = None;
    }

    /// `Minecraft.startAttack`: one left click. Returns vanilla's `endAttack` — the block broke
    /// at once (creative, or a survival block with progress >= 1 in one tick), so
    /// `continueAttack` is skipped for this tick.
    fn start_attack(
        &mut self,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) -> bool {
        if self.int.miss_time > 0 {
            return false;
        }
        // player.isHandsBusy() (an item in use, or dying) blocks the swing; spectator mode
        // zeroed the input in interact_step.
        if self.int.is_using_item {
            return false;
        }
        // `Minecraft.startAttack` swings on every attack, whatever the crosshair is on.
        self.hand.swing();
        match self.int.target {
            Some(Target::Entity { id, .. }) => {
                // gameMode.attack, then the swing (below). No missTime, no attack-strength
                // reset.
                out.push(ServerboundPlay::Attack(sb::Attack { entity_id: id.0 }));
            }
            Some(Target::Block(h)) if !env.data.is_air(world.block_state(h.pos)) => {
                // startDestroyBlock, then endAttack when the block is air afterwards.
                let gone = self.start_destroy(h, world, env, out);
                out.push(ServerboundPlay::Punch(sb::Punch {}));
                return gone;
            }
            // A miss, or a block hit on an already-air block: vanilla's switch has no `break`
            // on the air branch, so it falls through to the MISS case — missTime 10 outside
            // creative (gameMode.hasMissTime) and an attack-strength reset.
            _ => {
                if self.game_mode != GameMode::Creative {
                    self.int.miss_time = MISS_TIME;
                }
            }
        }
        out.push(ServerboundPlay::Punch(sb::Punch {}));
        false
    }

    /// `Minecraft.continueAttack` with `down = keyAttack.isDown()` (this tick's `instantAttack`
    /// aside): a held block target keeps breaking (swinging every tick), anything else stops
    /// the break; releasing the button clears `missTime`; after a miss nothing happens for
    /// `missTime` ticks.
    fn continue_attack(
        &mut self,
        down: bool,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        if !down {
            self.int.miss_time = 0;
        }
        if self.int.miss_time > 0 || self.int.is_using_item {
            return;
        }
        if !down {
            self.stop_destroy(out);
            return;
        }
        match self.int.target {
            Some(Target::Block(h)) if !env.data.is_air(world.block_state(h.pos)) => {
                if self.continue_destroy(h, world, env, out) {
                    // `Minecraft.continueAttack` swings each tick the breaking continues.
                    self.hand.swing();
                    out.push(ServerboundPlay::Punch(sb::Punch {}));
                    // `Minecraft.startUseItem` calls `player.itemUsed(hand)` for a use that
                    // changed the held stack, which puts the hand down for the item switch.
                    self.hand.item_used();
                }
                // The place sound, played when we place against a block while holding a
                // block item (vanilla: BlockItem.place runs on the client too and plays
                // it; the server's copy excludes us). We cannot know the placed state
                // yet, so the held block item's block decides; unknown items (not a
                // block, a mod) play nothing.
                self.own_place_sound(h.pos, env);
            }
            // An entity, nothing, or an already-air block: stop breaking.
            _ => self.stop_destroy(out),
        }
    }

    /// `Minecraft.startUseItem`: `rightClickDelay` 4, then per hand (main first, then off
    /// hand, `InteractionHand.values()`): entity interact, block use, item use. Nothing at all
    /// while a block is being destroyed (`!gameMode.isDestroying()`).
    fn start_use_item(&mut self, look: (f32, f32), out: &mut Vec<ServerboundPlay>) {
        if self.int.destroying.is_none() {
            self.int.use_delay = USE_DELAY;
            if !self.int.is_using_item {
                for hand in 0..=1 {
                    let holding = !self.hand(hand).is_empty();
                    match self.int.target {
                        Some(Target::Entity { id, location }) => {
                            // gameMode.interact: the interact packet goes out (the mod-driven
                            // result that decides the swing comes back with the server's answer).
                            out.push(ServerboundPlay::Interact(sb::Interact {
                                entity_id: id.0,
                                hand,
                                target_offset: location - self.eye(),
                                sneaking: self.state.sneaking,
                            }));
                            // `Minecraft.startUseItem` swings on a successful use (vanilla's
                            // swing source PREDICTED).
                            self.hand.swing();
                            return;
                        }
                        Some(Target::Block(h)) => {
                            // gameMode.useItemOn: cursor position inside the block, sequence.
                            let seq = self.int.next_sequence();
                            let c = h.location - h.pos.as_dvec3();
                            out.push(ServerboundPlay::UseItemOn(sb::UseItemOn {
                                hand,
                                pos: h.pos,
                                face: h.face as i32,
                                cursor_x: c.x as f32,
                                cursor_y: c.y as f32,
                                cursor_z: c.z as f32,
                                inside_block: h.inside,
                                world_border_hit: false,
                                sequence: seq,
                            }));
                            if holding {
                                out.push(ServerboundPlay::Punch(sb::Punch {}));
                            }
                            self.hand.swing();
                            return;
                        }
                        None => {}
                    }
                    // gameMode.useItem with nothing targeted.
                    if holding {
                        let seq = self.int.next_sequence();
                        out.push(ServerboundPlay::UseItem(sb::UseItem {
                            hand,
                            sequence: seq,
                            yaw: look.0,
                            pitch: look.1,
                        }));
                        self.hand.swing();
                        return;
                    }
                }
            }
        }
    }

    /// `MultiPlayerGameMode.releaseUsingItem`: `player_action` release_use_item with a zeroed
    /// pos, then the player stops using the item.
    fn release_using_item(&mut self, out: &mut Vec<ServerboundPlay>) {
        out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
            status: action::RELEASE_USE_ITEM,
            pos: BlockPos::new(0, 0, 0),
            face: Direction::Down as u8,
            sequence: 0,
        }));
        self.int.is_using_item = false;
    }

    /// `MultiPlayerGameMode.startDestroyBlock`: creative breaks at once (with `destroyDelay` 5,
    /// and nothing with a tool that cannot break blocks in creative); survival starts the
    /// break and finishes it in the same call when one tick of progress is enough. Returns
    /// vanilla's `endAttack`: the block is air now (so `Minecraft.startAttack` skips
    /// `continueAttack` this tick). `Minecraft.startAttack` reads the block state after the
    /// call; the `startDestroyBlock` return value itself means only "handled".
    fn start_destroy(
        &mut self,
        h: BlockHit,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) -> bool {
        let state = world.block_state(h.pos);
        // A block we broke this tick (or whose break the server has not acknowledged yet) is
        // gone for us even if the world has not applied the edit yet.
        let predicted_gone = self.int.predictions.iter().any(|p| p.1 == h.pos);
        if env.data.is_air(state) || predicted_gone {
            return false;
        }
        if self.game_mode == GameMode::Adventure {
            // Breaking in adventure depends on the held item's can_break predicate (not decoded).
            return false;
        }
        if self.game_mode == GameMode::Creative {
            if self.int.destroy_delay > 0 {
                return false;
            }
            self.int.destroy_delay = DESTROY_DELAY;
            // Item.canDestroyBlock: a tool with can_destroy_blocks_in_creative false cannot
            // break in creative. The packet still goes out; only the block change is skipped.
            let can_break = self.can_break_in_creative(env.data);
            // The start tick's hit sound (vanilla: addBreakingBlockEffects with playSound=true;
            // the follow-up hits every 4 ticks are the server's 2020 level events).
            self.own_mining_hit(h.pos, state, env);
            self.send_destroy(action::START_DESTROY, h, state, can_break, env, out);
            return can_break;
        }
        if !self.same_destroy_target(h.pos) {
            if self.int.destroying.is_some() {
                // The old break is aborted with the new direction, as startDestroyBlock sends.
                self.abort_destroy(h.face, out);
            }
            let params = self.mining_params(state, env);
            let d = params.progress_per_tick();
            tracing::debug!("start breaking {:?}: {params:?} -> {d} per tick", h.pos);
            let instant = d >= 1.0;
            // The start tick's hit sound (vanilla: addBreakingBlockEffects with playSound=true;
            // the follow-up hits every 4 ticks are the server's 2020 level events).
            self.own_mining_hit(h.pos, state, env);
            self.send_destroy(action::START_DESTROY, h, state, instant, env, out);
            if instant {
                return true;
            }
            self.int.destroying = Some((h.pos, h.face, self.main_hand().clone()));
            self.int.progress = 0.0;
        }
        false
    }

    /// `MultiPlayerGameMode.continueDestroyBlock`: the `destroyDelay` countdown, the creative
    /// instant break, then one tick of progress for the block under the crosshair — stop and
    /// the 5-tick delay at 1, CHANGE_DESTROY_DIRECTION when the same block is hit from another
    /// face, and a restart when the target changed. `true` when the arm swings this tick.
    fn continue_destroy(
        &mut self,
        h: BlockHit,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) -> bool {
        if self.int.destroy_delay > 0 {
            self.int.destroy_delay -= 1;
            return true;
        }
        if self.game_mode == GameMode::Creative {
            self.int.destroy_delay = DESTROY_DELAY;
            // continueDestroyBlock does not re-check the tool: the packet goes out every 5
            // ticks, but only a breakable block actually changes (Item.canDestroyBlock).
            let can_break = self.can_break_in_creative(env.data);
            self.send_destroy(
                action::START_DESTROY,
                h,
                world.block_state(h.pos),
                can_break,
                env,
                out,
            );
            return true;
        }
        if self.game_mode == GameMode::Adventure {
            return false;
        }
        if !self.same_destroy_target(h.pos) {
            return self.start_destroy(h, world, env, out);
        }
        let state = world.block_state(h.pos);
        if env.data.is_air(state) {
            self.int.destroying = None;
            return false;
        }
        self.int.progress += self.mining_params(state, env).progress_per_tick();
        if self.int.progress >= 1.0 {
            self.send_destroy(action::STOP_DESTROY, h, state, true, env, out);
            self.int.destroying = None;
            self.int.progress = 0.0;
            self.int.destroy_delay = DESTROY_DELAY;
        } else if self
            .int
            .destroying
            .as_ref()
            .is_some_and(|(_, face, _)| *face != h.face)
        {
            // Same block, another face under the crosshair (new in 26.3); no sequence, and
            // only when the block did not finish this tick.
            out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                status: action::CHANGE_DESTROY_DIRECTION,
                pos: h.pos,
                face: h.face as u8,
                sequence: 0,
            }));
            self.int.destroying = Some((h.pos, h.face, self.main_hand().clone()));
        }
        true
    }

    /// `MultiPlayerGameMode.stopDestroyBlock`: abort the break (Direction.DOWN, no sequence)
    /// and drop the progress, as vanilla does when the button is released or the crosshair
    /// leaves the block.
    fn stop_destroy(&mut self, out: &mut Vec<ServerboundPlay>) {
        if let Some((_, _, _)) = self.int.destroying {
            self.abort_destroy(Direction::Down, out);
        }
    }

    /// `MultiPlayerGameMode.sameDestroyTarget`: same block and same held item.
    fn same_destroy_target(&self, pos: BlockPos) -> bool {
        self.int
            .destroying
            .as_ref()
            .is_some_and(|(p, _, item)| *p == pos && *item == *self.main_hand())
    }

    fn abort_destroy(&mut self, face: Direction, out: &mut Vec<ServerboundPlay>) {
        if let Some((pos, _, _)) = self.int.destroying.take() {
            // Abort carries no prediction (sequence 0).
            out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                status: action::ABORT_DESTROY,
                pos,
                face: face as u8,
                sequence: 0,
            }));
            self.int.progress = 0.0;
        }
    }

    /// The held item's block, if it is a block item (item id = block id in vanilla,
    /// `BlockItemId.create(name)`; the 14 crop pairs with differing names are missing
    /// here — placing crops is rare and the server confirms anyway).
    fn held_block<'d>(&self, data: &'d haste_data::GameData) -> Option<&'d haste_core::Identifier> {
        let item = self.main_hand();
        let index = item.item?;
        data.registries
            .name("minecraft:item", index)
            .and_then(|id| data.blocks.default_state(id).map(|_| id))
    }

    /// The place sound for a `use_item_on` with a block item held (see `use_item`).
    fn own_place_sound(&mut self, pos: BlockPos, env: &InteractEnv<'_>) {
        let Some(block) = self.held_block(env.data) else {
            return;
        };
        let state = env.data.blocks.default_state(block);
        if let Some(state) = state {
            if let Some(t) = env.data.block_sound_type(state) {
                self.own_sounds.block_placed(
                    pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5),
                    t,
                    &mut self.sound_out,
                );
            }
        }
    }

    /// Mining parameters for a block with the held item.
    pub fn mining_params(&self, state: BlockStateId, env: &InteractEnv<'_>) -> MiningParams {
        let data = env.data;
        let block = data.blocks.block_of(state);
        let index = data.blocks.block_index(block).map_or(u32::MAX, |i| i.0);
        let tags = self.int.tags.as_ref();
        let (tool_speed, tool_correct) = self
            .held_tool(data)
            .map(|t| {
                t.speed_for(|set| {
                    tags.is_some_and(|tg| tg.matches(set, block, index))
                        || matches!(set, haste_physics::mining::BlockSet::Block(b) if b == block)
                        || matches!(set, haste_physics::mining::BlockSet::List(l) if l.contains(block))
                })
            })
            .unwrap_or((1.0, None));
        let level = |id: Option<i32>| {
// MERGE(effects side):
            id.and_then(|i| self.int.effects.amplifier(i))
                .map_or(0, |a| (a + 1).max(0) as u32)
// MERGE(main side):
            id.and_then(|i| self.int.effects.get(&i))
                .map_or(0, |e| (e.amplifier + 1).max(0) as u32)
// MERGE end
        };
        let e = self.int.effect_ids;
        let a = self.int.attrs;
        MiningParams {
            hardness: data.destroy_time(state),
            requires_correct_tool: data.requires_correct_tool(state),
            tool_speed,
            tool_correct,
            mining_efficiency: a.mining_efficiency,
            haste_level: level(e.map(|e| e.haste)).max(level(e.map(|e| e.conduit_power))),
            fatigue_level: level(e.map(|e| e.mining_fatigue)),
            block_break_speed: a.block_break_speed,
            submerged_mining_speed: a.submerged_mining_speed,
            eyes_in_water: self.state.eyes_in_water,
            on_ground: self.state.on_ground,
        }
    }

    fn held_tool<'t>(
        &'t self,
        data: &haste_data::GameData,
    ) -> Option<&'t haste_physics::mining::Tool> {
        let item = self.main_hand();
        if item.is_empty() {
            return None;
        }
        let name = data
            .registries
            .name("minecraft:item", item.item.unwrap_or(RegistryIndex(0)))?;
        self.int.tools.as_ref()?.get(name)
    }

    /// `Item.canDestroyBlock` for the main hand in creative: a tool whose
    /// `can_destroy_blocks_in_creative` is false cannot break blocks; anything else can.
    fn can_break_in_creative(&self, data: &haste_data::GameData) -> bool {
        !self
            .held_tool(data)
            .is_some_and(|t| !t.can_destroy_blocks_in_creative)
    }

    /// The mining hit sound of a block we start breaking (vanilla plays it
    /// client-side on the start tick; the block's type comes from the
    /// `block_sounds.json` dump). Unknown blocks play nothing.
    fn own_mining_hit(&mut self, pos: BlockPos, state: BlockStateId, env: &InteractEnv<'_>) {
        if let Some(t) = env.data.block_sound_type(state) {
            self.own_sounds.mining_hit(
                pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5),
                t,
                &mut self.sound_out,
            );
        }
    }

    fn send_destroy(
        &mut self,
        status: i32,
        h: BlockHit,
        state: BlockStateId,
        breaks: bool,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let seq = self.int.next_sequence();
        if !self.int.recent.contains(&h.pos) {
            if self.int.recent.len() >= 8 {
                self.int.recent.remove(0);
            }
            self.int.recent.push(h.pos);
        }
        tracing::debug!(
            "player_action {status} {:?} seq {seq} on_ground {} progress {}",
            h.pos,
            self.state.on_ground,
            self.int.progress
        );
        out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
            status,
            pos: h.pos,
            face: h.face as u8,
            sequence: seq,
        }));
        if breaks {
            // Predict the block gone; a waterlogged block leaves water, which we leave to the
            // server's update.
            let to = env.data.fluid(state).is_none().then_some(BlockStateId::AIR);
            self.int.predict(seq, h.pos, state, to);
            // The break sound, played the moment the block goes for us (vanilla's client
            // gets it as the server's level_event 2001, which we do not decode yet).
            if let Some(t) = env.data.block_sound_type(state) {
                self.own_sounds.block_broken(
                    h.pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5),
                    t,
                    &mut self.sound_out,
                );
            }
        }
    }
}

/// Vanilla's per-class `Entity.isPickable` (what `EntitySelector.CAN_BE_PICKED` tests in
/// `ProjectileUtil.getEntityHitResult`), as a rule over `minecraft:entity_type` names. The type
/// names are the registry ids from `EntityTypeIds` (ItemEntity's type is `item`, FishingHook's is
/// `fishing_bobber`, the thrown potions are `splash_potion`/`lingering_potion`).
/// Classes returning false: ItemEntity, ExperienceOrb, Marker, AreaEffectCloud, LightningBolt,
/// EyeOfEnder, EvokerFangs and OminousItemSpawner (Entity's default `isPickable`), the Display
/// entities (block/item/text_display, also Entity's default), EnderDragon (explicit false),
/// AbstractArrow's arrow, spectral_arrow and trident (`!isInGround` aside), and the Projectile
/// types outside the redirectable_projectile tag (small_fireball, dragon_fireball, wither_skull,
/// egg, snowball, ender_pearl, splash_potion, lingering_potion, experience_bottle,
/// firework_rocket, llama_spit, fishing_bobber — the tag holds fireball, wind_charge and
/// breeze_wind_charge). Everything else — LivingEntity's `!isRemoved`, Player's `!isSpectator`,
/// the boats, minecarts, item frames, paintings, leash knot, end crystal, TNT, falling blocks,
/// the interaction entity, shulker bullets — is pickable, as is any unknown (mod) type. Arrows
/// stuck in the ground (`AbstractArrow.isInGround`) stay pickable here: the box alone cannot tell.
pub fn is_pickable(t: &Identifier) -> bool {
    if t.namespace() != "minecraft" {
        return true;
    }
    let p = t.path();
    !matches!(
        p,
        "item"
            | "experience_orb"
            | "marker"
            | "area_effect_cloud"
            | "lightning_bolt"
            | "eye_of_ender"
            | "evoker_fangs"
            | "ominous_item_spawner"
            | "block_display"
            | "item_display"
            | "text_display"
            | "ender_dragon"
            | "trident"
            | "small_fireball"
            | "dragon_fireball"
            | "wither_skull"
            | "egg"
            | "snowball"
            | "ender_pearl"
            | "splash_potion"
            | "lingering_potion"
            | "experience_bottle"
            | "firework_rocket"
            | "llama_spit"
            | "fishing_bobber"
    ) && p != "arrow"
        && !p.ends_with("_arrow")
}

/// `Entity.getPickRadius` (Projectile: 1.0 when the type is pickable, else 0.0; every other
/// class 0.0) — the box inflation `ProjectileUtil.getEntityHitResult` searches with.
fn entity_pick_radius(t: &Identifier) -> Option<f64> {
    if t.namespace() == "minecraft"
        && matches!(
            t.path(),
            "fireball" | "wind_charge" | "breeze_wind_charge" | "shulker_bullet"
        )
    {
        Some(1.0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
