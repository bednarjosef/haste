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
//!   `MultiPlayerGameMode.releaseUsingItem`'s `release_use_item`. An item with a use duration
//!   (`Item.getUseDuration` from the `minecraft:consumable` component or the item class) starts
//!   `LivingEntity.useItem`/`useItemRemaining`, whose animation is `Item.getUseAnimation`.
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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::DVec3;
use haste_core::entity::ItemStackData;
use haste_core::input::InteractInput;
use haste_core::view::BlockView;
use haste_core::{Aabb, BlockPos, BlockStateId, Direction, EntityId, Identifier, RegistryIndex};
use haste_physics::mining::{BlockTags, ToolTable};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::LocalPlayer;
use haste_core::game::GameMode;

mod attack;
mod destroy;
mod effects;
mod held_item;
mod packets;
mod pick;
mod use_item;
pub use pick::Target;
pub use use_item::UsingItem;
pub(crate) use effects::{LightmapEffectIds, MovementEffectIds};
// Re-exported for `player/tests.rs` (`interact::swap_key`); the parent itself no longer
// calls it since `refresh_components` moved to `held_item`.
#[allow(unused_imports)]
pub(crate) use held_item::swap_key;
use effects::EffectIds;
use held_item::HeldItem;

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
    /// Vanilla `MultiPlayerGameMode.destroyTicks`: how many `continueDestroyBlock` calls have
    /// added progress to the current break (the press tick's `continueAttack` counts, so a
    /// finished break reports the ticks from press to finish inclusive).
    destroy_ticks: u32,
    destroy_delay: u8,
    use_delay: u8,
    /// Left/right clicks queued since the last tick (vanilla's `KeyMapping.consumeClick`): each
    /// press counts, even several in one tick. The driver adds one per mouse press; a driver
    /// that only reports the held boolean gets one click on the press edge instead.
    attack_clicks: u8,
    use_clicks: u8,
    /// `missTime` ticks left: 10 after a miss outside creative.
    miss_time: u8,
    /// True while the player is using an item (`LivingEntity.isUsingItem`): set when a use with
    /// a duration starts, cleared on release, an item change or death. `using` carries the
    /// component-driven duration and animation.
    is_using_item: bool,
    /// The item currently being used (`LivingEntity.useItem`/`useItemRemaining`): the
    /// `minecraft:consumable` component's `use_duration`/`use_animation` (or the item class's
    /// own), ticked down each game tick. Set together with `is_using_item`.
    using: Option<UsingItem>,
    /// The hand of a use the server's synced flags started that we have not started
    /// locally yet (`LivingEntity.onSyncedDataUpdated` reads the held item then; we do it on
    /// the next tick, when the game data is at hand).
    pending_use_hand: Option<i32>,
    /// Last sequence number used for a prediction.
    sequence: i32,
    /// Predicted blocks: (sequence, pos, the server's state: before the prediction, then the
    /// latest block update held back).
    predictions: Vec<(i32, BlockPos, BlockStateId)>,
    world_edits: Vec<(BlockPos, BlockStateId)>,
    /// Hotbar 0..=8 and off hand.
    pub hotbar: [ItemStackData; 9],
    pub offhand: ItemStackData,
    /// Armor, indexed helmet 0, chest 1, legs 2, boots 3 (player inventory menu 5..=8 and
    /// `set_player_inventory` 36..=39). The chest slot answers the glider check; the helmet is
    /// the worn item whose camera overlay `Hud.extractCameraOverlays` reads.
    pub armor: [ItemStackData; 4],
    /// Item registry indices whose default components carry `minecraft:glider`
    /// (`LivingEntity.canGlideUsing`), resolved from the item component reports in `prepare`.
    pub(crate) glider_items: Option<HashSet<RegistryIndex>>,
    /// Item registry indices that steer a ridden animal while held (vanilla
    /// `Pig.getControllingPassenger`'s carrot on a stick, `Strider`'s warped fungus on a
    /// stick), resolved from the item registry in `prepare`.
    pub(crate) steer_items: Option<HashMap<RegistryIndex, crate::item_based_steering::SteerStick>>,
    /// Active effects on us: `minecraft:mob_effect` index -> amplifier, remaining ticks and the
    /// vanilla `MobEffectInstance.BlendState` the screen effects read.
    pub effects: HashMap<i32, EffectData>,
    tools: Option<Arc<ToolTable>>,
    tags: Option<BlockTags>,
    effect_ids: Option<EffectIds>,
    /// `minecraft:data_component_type` names by protocol id, for decoding the held stack's
    /// component patch (`haste_protocol::wire::components::read_patch`).
    component_tables: Option<Arc<haste_protocol::packets::ProtocolTables>>,
    /// The item default components (`minecraft:attribute_modifiers`, `minecraft:consumable`)
    /// from the data generator.
    item_components: Option<Arc<haste_data::ItemComponents>>,
    /// The held main-hand stack's decoded data and the stack it was decoded from (re-decoded
    /// when the slot's stack changes; a decode failure leaves it `None`).
    components: Option<HeldItem>,
    /// Night-vision and darkness effect indices for the lightmap (`prepare`).
    pub(crate) lightmap_effect_ids: LightmapEffectIds,
    /// Levitation, slow-falling and jump-boost effect indices for the movement tick (`prepare`).
    pub(crate) movement_effect_ids: MovementEffectIds,
    /// Last blocks we sent actions for (debug logging of their server updates).
    recent: Vec<BlockPos>,
    /// The screen the last use opened (see [`UsedScreen`]), until the game thread takes it.
    used_screen: Option<UsedScreen>,
}

/// A screen a use opened (vanilla opens these in `MultiPlayerGameMode.performUseItemOn` /
/// `useItem` on the client; the server sends no packet for them). The game thread hands it to
/// the UI the same tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsedScreen {
    /// `CommandBlock.useWithoutItem` -> `LocalPlayer.openCommandBlock`: the command block being
    /// used (game-master blocks need creative + permission level >= 2, like vanilla's
    /// `Player.canUseGameMasterBlocks`).
    CommandBlock(BlockPos),
    /// `WritableBookItem.use` -> `LocalPlayer.openItemGui`: the hand holding the writable book
    /// (0 main, 1 off, `InteractionHand.values()` numbering).
    ItemGui(i32),
}

/// One active effect on us: the vanilla `MobEffectInstance` the screen effects and the lightmap
/// read (amplifier, remaining ticks and the `BlendState`).
pub use crate::screen_effects::Effect as EffectData;

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
        if self.steer_items.is_none() {
            // The sticks that steer a ridden animal while held (their item registry names).
            self.steer_items = Some(
                [
                    crate::item_based_steering::SteerStick::CarrotOnAStick,
                    crate::item_based_steering::SteerStick::WarpedFungusOnAStick,
                ]
                .into_iter()
                .filter_map(|stick| {
                    data.registries
                        .index("minecraft:item", &Identifier::minecraft(stick.item_path()))
                        .map(|i| (i, stick))
                })
                .collect(),
            );
        }
        if self.effect_ids.is_none() {
            let id = |n: &str| {
                data.registries
                    .index("minecraft:mob_effect", &Identifier::minecraft(n))
                    .map(|i| i.0 as i32)
            };
            if let (
                Some(haste),
                Some(conduit_power),
                Some(mining_fatigue),
                Some(nausea),
                Some(darkness),
            ) = (
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
                blindness: id("blindness"),
                fire_resistance: id("fire_resistance"),
            };
            self.movement_effect_ids = MovementEffectIds {
                levitation: id("levitation"),
                slow_falling: id("slow_falling"),
                jump_boost: id("jump_boost"),
            };
        }
        if self.component_tables.is_none() {
            self.component_tables = Some(Arc::new(
                haste_protocol::packets::ProtocolTables::with_data_components(
                    data.registries.data_component_names(),
                ),
            ));
        }
        if self.item_components.is_none() {
            let c = haste_data::ItemComponents::load_default().unwrap_or_else(|e| {
                tracing::warn!("no item attribute modifiers (attack speed from the server): {e:#}");
                haste_data::ItemComponents::default()
            });
            self.item_components = Some(Arc::new(c));
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

    /// Replaces the item default components (tests, without the data generator cache).
    pub fn set_item_components(&mut self, item_components: Arc<haste_data::ItemComponents>) {
        self.item_components = Some(item_components);
    }

    pub(super) fn respawned(&mut self) {
        self.destroying = None;
        self.progress = 0.0;
        self.destroy_ticks = 0;
        self.target = None;
        self.effects.clear();
        self.miss_time = 0;
        self.is_using_item = false;
        self.using = None;
        self.pending_use_hand = None;
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

    /// Whether we are scoping with a spyglass this tick: vanilla's `Player.isScoping` (using an
    /// item, and the item being used is a spyglass). The use state is the held "use" button
    /// (`use_item` is re-sent every 4 ticks while it stays held, so a held button is a use in
    /// progress) and the item is the main hand's.
    pub fn scoping(&self, data: &haste_data::GameData) -> bool {
        if !self.int.last.use_item {
            return false;
        }
        let name = self
            .main_hand()
            .item
            .and_then(|i| data.registries.name("minecraft:item", i));
        crate::screen_effects::is_spyglass(name)
    }

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
    }

    /// The screen the last use opened (taken by the game thread, which hands it to the UI).
    pub fn take_used_screen(&mut self) -> Option<UsedScreen> {
        self.int.used_screen.take()
    }

    /// The held main-hand stack.
    pub fn main_hand(&self) -> &ItemStackData {
        &self.int.hotbar[self.held_slot.clamp(0, 8) as usize]
    }

    /// Whether either hand holds an item that steers a ridden animal (vanilla
    /// `Pig`/`Strider.getControllingPassenger` -> `LivingEntity.isHolding`, which looks at the
    /// main hand and the off hand: the carrot on a stick, the warped fungus on a stick). An
    /// unknown registry (no `prepare` yet) steers nothing.
    pub fn holds_steer_stick(&self, stick: crate::item_based_steering::SteerStick) -> bool {
        let steers = |stack: &haste_core::entity::ItemStackData| {
            stack.item.is_some_and(|i| {
                self.int
                    .steer_items
                    .as_ref()
                    .is_some_and(|s| s.get(&i) == Some(&stick))
            })
        };
        steers(self.main_hand()) || steers(&self.int.offhand)
    }

    /// `Player.getAttackStrengthScale`: how charged the attack is (0..1) after
    /// `attackStrengthTicker` ticks with the held item's delay.
    pub fn attack_strength_scale(&self, partial: f32) -> f32 {
        self.hand.attack_strength_scale(partial)
    }

    /// `Player.resetAttackStrengthTicker` (abort breaking, item change).
    pub fn reset_attack_strength_ticker(&mut self) {
        self.hand.reset_attack_strength_ticker();
    }

    /// `Player.resetOnlyAttackStrengthTicker` (`Player.onAttack`).
    pub fn reset_only_attack_strength_ticker(&mut self) {
        self.hand.reset_only_attack_strength_ticker();
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

        // 2. The crosshair target (Minecraft.pick). The held stack's components are decoded once
        // per stack change; nothing in the pick reads them, but the decode belongs with the
        // per-tick held-item upkeep.
        let held = self.main_hand().clone();
        self.int.refresh_components(held, env.data);
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
        // A use the server's synced flags started (`LivingEntity.onSyncedDataUpdated`): the
        // held item of that hand and its duration.
        if let Some(hand) = self.int.pending_use_hand.take() {
            let stack = self.hand(hand).clone();
            if let Some((duration, animation)) = self.int.use_data(&stack, env.data) {
                self.int.using = Some(UsingItem {
                    item: stack.item,
                    duration,
                    remaining: duration,
                    animation,
                });
            }
        }
        if self.int.is_using_item {
            // While an item is in use the clicks are consumed and do nothing; releasing the
            // button releases the item (MultiPlayerGameMode.releaseUsingItem), and a different
            // item in the hand stops the use without a packet
            // (`LivingEntity.updatingUsingItem`).
            if !input.use_item {
                self.release_using_item(out);
            } else if self
                .int
                .using
                .as_ref()
                .is_some_and(|u| self.main_hand().item != u.item)
            {
                self.int.using = None;
                self.int.is_using_item = false;
            }
        } else {
            // while (keyAttack.consumeClick()) instantAttack |= startAttack();
            for _ in 0..attack_clicks {
                instant_attack |= self.start_attack(world, env, out);
            }
            // while (keyUse.consumeClick()) startUseItem();
            for _ in 0..use_clicks {
                self.start_use_item(look, world, env, out);
            }
        }

        // if (keyUse.isDown() && rightClickDelay == 0 && !player.isUsingItem()) startUseItem();
        if input.use_item && self.int.use_delay == 0 && !self.int.is_using_item {
            self.start_use_item(look, world, env, out);
        }

        // `LivingEntity.tick` -> `updateUsingItem`: the remaining time ticks down each game tick,
        // including the tick the use started on.
        if let Some(mut u) = self.int.using {
            u.remaining -= 1;
            self.int.using = Some(u);
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
}

#[cfg(test)]
mod tests;
