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
use haste_core::view::{BlockPhysics, BlockView};
use haste_core::{Aabb, BlockPos, BlockStateId, Direction, EntityId, Identifier, RegistryIndex};
use haste_physics::mining::{BlockTags, ToolTable};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::{ClientboundPlay, ServerboundPlay};

use super::LocalPlayer;
use haste_core::game::GameMode;

mod destroy;
mod pick;
mod use_item;
pub use pick::Target;
pub use use_item::UsingItem;

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

/// The held main-hand stack's decoded components the game logic reads: what it mines
/// (`minecraft:tool`), the Efficiency enchantment's `mining_efficiency` contribution
/// (`Enchantments.EFFICIENCY`'s `LevelsSquared(1.0)` effect = level^2 + 1), and the
/// `minecraft:attack_speed` its attribute modifiers grant.
#[derive(Clone, Debug)]
struct HeldItem {
    /// The stack this was decoded from (re-decode when it changes).
    stack: ItemStackData,
    /// The stack's own `minecraft:tool` component, if it overrides the item default.
    tool: Option<haste_physics::mining::Tool>,
    /// `minecraft:mining_efficiency` from the stack's Efficiency enchantment (0 if none).
    efficiency: f64,
    /// The player's `minecraft:attack_speed` with this item held
    /// (`Player.getCurrentItemAttackStrengthDelay`).
    attack_speed: f64,
    /// [`swap_key`] of this stack, for the first-person equip.
    swap_key: u64,
}

/// A key over what `ItemStack.matchesIgnoringComponents(a, b, DataComponentType::
/// ignoreSwapAnimation)` compares beyond the item type: the count, which component types the
/// stack's patch adds and removes, and the values of all but the ignored types (`damage`,
/// `max_damage`, `max_stack_size`: those count only by presence). Equal keys mean the hand
/// swaps the shown stack without the dip.
pub(crate) fn swap_key(
    stack: &ItemStackData,
    patch: &haste_protocol::wire::components::ComponentPatch,
) -> u64 {
    use haste_protocol::wire::components::ItemComponent;
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    stack.count.hash(&mut h);
    for c in &patch.added {
        match c {
            ItemComponent::Damage(_) => "damage".hash(&mut h),
            ItemComponent::MaxDamage(_) => "max_damage".hash(&mut h),
            ItemComponent::MaxStackSize(_) => "max_stack_size".hash(&mut h),
            other => format!("{other:?}").hash(&mut h),
        }
    }
    patch.removed.hash(&mut h);
    h.finish()
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

/// The `minecraft:mob_effect` indices the movement tick needs (levitation, slow falling and jump
/// boost), resolved from the game data. `None` until [`Interaction::prepare`] finds them, like
/// [`LightmapEffectIds`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MovementEffectIds {
    pub(crate) levitation: Option<i32>,
    pub(crate) slow_falling: Option<i32>,
    pub(crate) jump_boost: Option<i32>,
}

/// The `minecraft:mob_effect` indices the lightmap needs (night vision and darkness), resolved
/// from the game data. `None` until [`Interaction::prepare`] finds them (mod registries, or a
/// report without them, leave the lightmap without those effects).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LightmapEffectIds {
    pub(crate) night_vision: Option<i32>,
    pub(crate) darkness: Option<i32>,
    /// The blindness effect (fog environment `BlindnessFogEnvironment`).
    pub(crate) blindness: Option<i32>,
    /// The fire-resistance effect (lava fog distance `LavaFogEnvironment`).
    pub(crate) fire_resistance: Option<i32>,
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

    /// Decodes the held main-hand stack's component patch once, when the slot's stack changes
    /// (`ItemStack.get(DataComponents.TOOL)` / the Efficiency enchantment's attribute effect).
    fn refresh_components(&mut self, stack: ItemStackData, data: &haste_data::GameData) {
        if self.components.as_ref().is_some_and(|h| h.stack == stack) {
            return;
        }
        let Some(tables) = self.component_tables.clone() else {
            return;
        };
        let patch = if stack.is_empty() || stack.components_raw.is_empty() {
            haste_protocol::wire::components::ComponentPatch::default()
        } else {
            match haste_protocol::wire::components::read_patch(&stack.components_raw, &tables) {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!("held stack component patch did not decode: {e}");
                    self.components = None;
                    return;
                }
            }
        };
        let tool = patch.tool().map(|t| tool_from_component(t, data));
        let efficiency = efficiency_from_enchantments(&patch, data);
        let attack_speed = self.item_attack_speed(&stack, data);
        let swap_key = swap_key(&stack, &patch);
        tracing::debug!(
            "held item components: attack_speed {attack_speed} ({} delay ticks), efficiency {efficiency}",
            if attack_speed > 0.0 { 20.0 / attack_speed } else { 20.0 / 4.0 }
        );
        self.components = Some(HeldItem {
            stack,
            tool,
            efficiency,
            attack_speed,
            swap_key,
        });
    }

    /// The player's `minecraft:attack_speed` with `stack` held: the base 4.0 plus the item's
    /// main-hand `attack_speed` attribute modifiers, as `ItemAttributeModifiers.compute` does.
    /// Falls back to the server-synced attribute for an item without any such modifier.
    fn item_attack_speed(&self, stack: &ItemStackData, data: &haste_data::GameData) -> f64 {
        let fallback = self.attrs.attack_speed;
        let Some(item) = stack
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))
        else {
            return fallback;
        };
        let attr = Identifier::minecraft("attack_speed");
        let Some(ic) = &self.item_components else {
            return fallback;
        };
        if ic
            .attribute_modifiers(item)
            .iter()
            .any(|m| m.attribute == attr && m.applies_to_main_hand())
        {
            ic.main_hand_attribute(item, &attr, crate::hand::DEFAULT_ATTACK_SPEED)
        } else {
            fallback
        }
    }

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
            .and_then(|id| self.effects.get(&id))
            .map_or(0.0, |e| e.blend_factor(partial_tick))
    }

    /// The nausea blend factor pair (the previous frame's and this tick's), for the frame's
    /// lerp (`BlendState.factorPreviousFrame`, `.factor`).
    pub fn nausea_blend_pair(&self) -> (f32, f32) {
        self.nausea_id()
            .and_then(|id| self.effects.get(&id))
            .map_or((0.0, 0.0), |e| e.blend_pair())
    }

    /// The movement effects the physics tick reads, from our active effects
    /// (`LivingEntity.hasEffect` / `getEffect(...).getAmplifier()`): the levitation and jump-boost
    /// amplifiers (`None` without the effect) and whether slow falling is on.
    pub fn movement_effects(&self) -> (Option<i32>, bool, Option<i32>) {
        let level = |id: Option<i32>| id.and_then(|i| self.effects.get(&i)).map(|e| e.amplifier);
        let ids = self.movement_effect_ids;
        (
            level(ids.levitation),
            ids.slow_falling
                .is_some_and(|i| self.effects.contains_key(&i)),
            level(ids.jump_boost),
        )
    }

    /// One tick of the active effects (`LivingEntity.tickEffects`: each instance ticks its
    /// duration and its vanilla blend state on the client).
    pub(super) fn tick_effects(&mut self) {
        for e in self.effects.values_mut() {
            e.tick();
        }
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

/// Converts a stack's `minecraft:tool` component (protocol `IdSet`s) into the physics tool the
/// mining code matches against block tags and names.
fn tool_from_component(
    t: &haste_protocol::wire::components::Tool,
    data: &haste_data::GameData,
) -> haste_physics::mining::Tool {
    use haste_physics::mining::{BlockSet, ToolRule};
    use haste_protocol::wire::IdSet;
    let rules = t
        .rules
        .iter()
        .map(|r| {
            let blocks = match &r.blocks {
                IdSet::Tag(tag) => BlockSet::Tag(tag.clone()),
                IdSet::Ids(ids) => BlockSet::List(
                    ids.iter()
                        .filter_map(|i| {
                            data.registries
                                .name("minecraft:block", RegistryIndex(*i as u32))
                                .cloned()
                        })
                        .collect(),
                ),
            };
            ToolRule {
                blocks,
                speed: r.speed,
                correct_for_drops: r.correct_for_drops,
            }
        })
        .collect();
    haste_physics::mining::Tool {
        rules,
        default_mining_speed: t.default_mining_speed,
        damage_per_block: t.damage_per_block,
        can_destroy_blocks_in_creative: t.can_destroy_blocks_in_creative,
    }
}

/// The `minecraft:mining_efficiency` a stack's Efficiency enchantment grants
/// (`Enchantments.EFFICIENCY`, `LevelsSquared(1.0)` = level^2 + 1; ids from
/// `minecraft:enchantment`).
fn efficiency_from_enchantments(
    patch: &haste_protocol::wire::components::ComponentPatch,
    data: &haste_data::GameData,
) -> f64 {
    let Some(ench) = patch.enchantments() else {
        return 0.0;
    };
    ench.iter()
        .filter_map(|e| {
            let name = data
                .registries
                .name("minecraft:enchantment", RegistryIndex(e.id as u32))?;
            (name.namespace() == "minecraft" && name.path() == "efficiency").then(|| {
                let l = f64::from(e.level);
                l * l + 1.0
            })
        })
        .sum()
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

    /// The main-hand stack as the first-person equip compares it
    /// (`FirstPersonHandsAndItems.shouldInstantlyReplaceVisibleItem`): its type and the swap key
    /// of its decoded components, or of the raw patch when it has not been decoded.
    pub fn held_stack(&self) -> crate::hand::HeldStack {
        let stack = self.main_hand();
        let swap_key = match self.int.components.as_ref() {
            Some(h) if h.stack == *stack => h.swap_key,
            _ => {
                use std::hash::{Hash, Hasher};
                let mut h = std::hash::DefaultHasher::new();
                stack.count.hash(&mut h);
                stack.components_raw.hash(&mut h);
                h.finish()
            }
        };
        crate::hand::HeldStack {
            item: stack.item,
            swap_key,
        }
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

    /// The player's `minecraft:attack_speed` with the held item (base 4.0 plus the item's
    /// attribute modifiers; the server-synced attribute when the item has none). Used by
    /// `Player.getCurrentItemAttackStrengthDelay` for the equip and the attack indicator.
    pub fn held_attack_speed(&self) -> f64 {
        let stack = self.main_hand();
        self.int
            .components
            .as_ref()
            .filter(|h| h.stack == *stack)
            .map_or(self.int.attrs.attack_speed, |h| h.attack_speed)
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
            P::SetPlayerInventory(s) => self.set_inventory_slot(s.slot, &s.item),
            P::UpdateMobEffect(e) if e.entity_id == me => {
                // Vanilla's blend durations per effect (`MobEffects`): only nausea and
                // darkness set them, everything else blends at once.
                use crate::screen_effects as se;
                let (blend_in, blend_out, blend_out_advance) =
                    if Some(e.effect) == self.int.nausea_id() {
                        (
                            se::NAUSEA_BLEND_IN,
                            se::NAUSEA_BLEND_OUT,
                            se::NAUSEA_BLEND_OUT_ADVANCE,
                        )
                    } else if self.int.darkness_id(e.effect) {
                        (se::DARKNESS_BLEND, se::DARKNESS_BLEND, se::DARKNESS_BLEND)
                    } else {
                        (0, 0, 0)
                    };
                let mut effect = EffectData::new(
                    e.amplifier,
                    e.duration,
                    e.flags & 0x08 != 0,
                    blend_in,
                    blend_out,
                    blend_out_advance,
                );
                // `LivingEntity.forceAddEffect` copies the previous instance's blend state.
                if let Some(prev) = self.int.effects.get(&e.effect) {
                    effect.copy_blend_from(prev);
                }
                self.int.effects.insert(e.effect, effect);
            }
            P::RemoveMobEffect(e) if e.entity_id == me => {
                self.int.effects.remove(&e.effect);
            }
            _ => return false,
        }
        true
    }

    /// Slot of the player inventory menu (window 0): 5..=8 armor (helmet, chest, legs, boots),
    /// 36..=44 hotbar, 45 off hand (wiki "Inventory" slot numbering).
    fn set_inventory_menu_slot(&mut self, slot: i32, item: &ItemStackData) {
        match slot {
            5..=8 => self.int.armor[(slot - 5) as usize] = item.clone(),
            36..=44 => self.int.hotbar[(slot - 36) as usize] = item.clone(),
            45 => self.int.offhand = item.clone(),
            _ => {}
        }
    }

    /// A slot of the player's own inventory in vanilla `Inventory` numbering (0-8 hotbar,
    /// 9-35 main, 36-39 armor feet..head, 40 off hand), as `set_player_inventory` carries
    /// and Java's `InventorySync` mirrors. One path for the server's packets and the
    /// client-side edits vanilla Java code predicts.
    pub fn set_inventory_slot(&mut self, slot: i32, item: &ItemStackData) {
        match slot {
            0..=8 => self.int.hotbar[slot as usize] = item.clone(),
            // `Inventory` indices: 36 feet, 37 legs, 38 chest, 39 head, 40 off hand.
            36..=39 => self.int.armor[(39 - slot) as usize] = item.clone(),
            40 => self.int.offhand = item.clone(),
            // 9..=35 main inventory: not held anywhere yet (no use reads it).
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
                // `MultiPlayerGameMode.attack`: the packet, `Player.attack` (`onAttack` resets
                // the attack ticker), `resetAttackStrengthTicker` (both tickers) and
                // `destroyDelay` 5 in creative. No missTime.
                out.push(ServerboundPlay::Attack(sb::Attack { entity_id: id.0 }));
                self.hand.reset_attack_strength_ticker();
                if self.game_mode == GameMode::Creative {
                    self.int.destroy_delay = DESTROY_DELAY;
                }
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
                self.hand.reset_attack_strength_ticker();
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
                }
            }
            // An entity, nothing, or an already-air block: stop breaking.
            _ => self.stop_destroy(out),
        }
    }
}

#[cfg(test)]
mod tests;
