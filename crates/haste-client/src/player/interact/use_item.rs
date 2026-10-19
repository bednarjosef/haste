//! Using items and blocks (player workstream): `Minecraft.startUseItem` per hand (entity
//! interact, block use, item use), `MultiPlayerGameMode.releaseUsingItem`, the client-side
//! screens a use opens, and the item-in-use state (`LivingEntity.useItem` /
//! `useItemRemaining`, `Item.getUseDuration` / `getUseAnimation`).

use haste_core::entity::ItemStackData;
use haste_core::game::GameMode;
use haste_core::view::BlockView;
use haste_core::{BlockPos, Direction, Identifier, RegistryIndex};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::super::LocalPlayer;
use super::{action, InteractEnv, Target, UsedScreen, USE_DELAY};

/// `ItemUseAnimation` (vanilla's order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseAnimation {
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Trident,
    Crossbow,
    Spyglass,
    TootHorn,
    Brush,
    Bundle,
    Spear,
}

impl UseAnimation {
    pub(super) fn from_id(id: i32) -> Self {
        match id {
            0 => UseAnimation::None,
            2 => UseAnimation::Drink,
            3 => UseAnimation::Block,
            4 => UseAnimation::Bow,
            5 => UseAnimation::Trident,
            6 => UseAnimation::Crossbow,
            7 => UseAnimation::Spyglass,
            8 => UseAnimation::TootHorn,
            9 => UseAnimation::Brush,
            10 => UseAnimation::Bundle,
            11 => UseAnimation::Spear,
            // `eat` (1) and unknown ids.
            _ => UseAnimation::Eat,
        }
    }
}

/// The item being used and how long is left (`LivingEntity.useItem`/`useItemRemaining`).
#[derive(Clone, Debug, PartialEq)]
pub struct UsingItem {
    /// The item type this use started with (`ItemStack.isSameItem` check each tick).
    pub item: Option<RegistryIndex>,
    /// `getUseDuration` (ticks).
    pub duration: i32,
    /// `useItemRemainingTicks`, decremented each tick.
    pub remaining: i32,
    /// `getUseAnimation`.
    pub animation: UseAnimation,
    /// The stack's `minecraft:consumable` (`ItemStack.onUseTick` reads it off the
    /// in-use stack for the per-tick effects; resolved once where the use starts, like
    /// vanilla's captured `useItem` stack). `None` when the stack has none — an item
    /// whose use an item class owns (a bow's draw) keeps the component it carries, though
    /// vanilla's `shouldEmitParticlesAndSounds` math keeps it silent for the class's long
    /// durations.
    pub consumable: Option<haste_data::Consumable>,
}

impl super::Interaction {
    /// `Item.getUseDuration` / `Item.getUseAnimation` for a held stack: the item classes
    /// that override them (virtual dispatch: the class wins over any consumable
    /// component, patched or default), else the stack's `minecraft:consumable`. `None`
    /// when the item is not used over time (duration 0, animation `NONE`).
    pub(super) fn use_data(
        &self,
        stack: &ItemStackData,
        data: &haste_data::GameData,
    ) -> Option<UsingItem> {
        let item = stack
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))?;
        let consumable = self.stack_consumable(stack, data);
        // Items whose class overrides `getUseDuration`/`getUseAnimation`
        // (`BowItem`/`CrossbowItem`/`TridentItem`/`SpyglassItem`/`ShieldItem`); a shield's
        // `minecraft:blocks_attacks` makes the default `Item` return 72000 and `BLOCK`.
        let (duration, animation) = match item.path() {
            "bow" => (72000, UseAnimation::Bow),
            "crossbow" => (72000, UseAnimation::Crossbow),
            "trident" => (72000, UseAnimation::Trident),
            "spyglass" => (1200, UseAnimation::Spyglass),
            "shield" => (72000, UseAnimation::Block),
            _ => {
                let c = consumable.as_ref()?;
                (c.consume_ticks(), UseAnimation::from_id(c.animation))
            }
        };
        Some(UsingItem {
            item: stack.item,
            duration,
            remaining: duration,
            animation,
            consumable,
        })
    }

    /// Test/script hook for the item-in-use state (vanilla: `player.isUsingItem()`; eating blocks
    /// attack and use and releasing the button then sends `release_use_item`).
    pub fn set_using_item(&mut self, using: bool) {
        self.is_using_item = using;
    }

    /// `LivingEntity.onSyncedDataUpdated` for `DATA_LIVING_ENTITY_FLAGS`: the server's
    /// `isUsingItem` bit is the truth. Cleared (a consume completed, or the server refused the
    /// use), the local use ends without a packet; set while nothing is in use locally, the held
    /// item of `hand` starts being used (filled in on the next tick, see `interact_step`).
    pub fn synced_using_item(&mut self, using: bool, hand: i32) {
        if using {
            if !self.is_using_item {
                self.is_using_item = true;
                self.using = None;
                self.pending_use_hand = Some(hand);
            }
        } else if self.is_using_item {
            self.is_using_item = false;
            self.using = None;
            self.pending_use_hand = None;
        }
    }
}

impl LocalPlayer {
    /// `LivingEntity.isUsingItem`: an item is being used over time.
    pub fn is_using_item(&self) -> bool {
        self.int.is_using_item
    }

    /// The `minecraft:use_effects` of the item in use (vanilla reads them off the used stack
    /// in `LocalPlayer.isSlowDueToUsingItem` and `itemUseSpeedMultiplier`): the item's default
    /// components from the report. `UseEffects.DEFAULT` when it names none or the item cannot
    /// be resolved (no environment, no stack yet — every item's prototype carries the
    /// component). A stack's component patch can override them, which this side does not
    /// decode.
    fn used_item_effects(&self, env: Option<&InteractEnv<'_>>) -> haste_data::UseEffects {
        let Some(env) = env else {
            return haste_data::UseEffects::DEFAULT;
        };
        let Some(item) = self
            .int
            .using
            .as_ref()
            .and_then(|u| u.item)
            .and_then(|i| env.data.registries.name("minecraft:item", i))
        else {
            return haste_data::UseEffects::DEFAULT;
        };
        env.data.item_components.use_effects(item)
    }

    /// Feeds the physics state the used item's `use_effects`, before the movement tick
    /// (vanilla `LocalPlayer.isSlowDueToUsingItem` and `itemUseSpeedMultiplier`): using an
    /// item keeps a sprint from starting unless the stack's `can_sprint`, and scales the
    /// movement input by its speed multiplier (1.0 when no item is in use).
    pub fn apply_item_use_effects(&mut self, env: Option<&InteractEnv<'_>>) {
        let effects = self.used_item_effects(env);
        let using = self.int.is_using_item;
        self.state.using_item = using;
        self.state.slow_using_item = using && !effects.can_sprint;
        self.state.item_use_speed = if using { effects.speed_multiplier } else { 1.0 };
    }

    /// `LivingEntity.getUseItemRemainingTicks`.
    pub fn use_item_remaining(&self) -> i32 {
        self.int.using.as_ref().map_or(0, |u| u.remaining)
    }

    /// `LivingEntity.getUseItem`'s duration (`Item.getUseDuration`), 0 when not using.
    pub fn use_duration(&self) -> i32 {
        self.int.using.as_ref().map_or(0, |u| u.duration)
    }

    /// `Item.getUseAnimation` of the item being used.
    pub fn use_animation(&self) -> UseAnimation {
        self.int
            .using
            .as_ref()
            .map_or(UseAnimation::None, |u| u.animation)
    }

    /// `LivingEntity.getTicksUsingItem`: how long the current use has run.
    pub fn ticks_using_item(&self) -> i32 {
        self.int
            .using
            .as_ref()
            .map_or(0, |u| u.duration - u.remaining)
    }

    /// The stack of hand `i` (0 main, 1 off), as `InteractionHand.values()` numbers them.
    pub(super) fn hand(&self, hand: i32) -> &ItemStackData {
        if hand == 1 {
            &self.int.offhand
        } else {
            self.main_hand()
        }
    }

    /// `Minecraft.startUseItem`: `rightClickDelay` 4, then per hand (main first, then off
    /// hand, `InteractionHand.values()`): entity interact, block use, item use. Nothing at all
    /// while a block is being destroyed (`!gameMode.isDestroying()`).
    pub(super) fn start_use_item(
        &mut self,
        look: (f32, f32),
        sneak_key: bool,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        if self.int.destroying.is_none() {
            // The delay is set even when the use cannot run (vanilla sets rightClickDelay
            // before the isHandsBusy check). The using-item gate is handleKeybinds' (the
            // click loop and the held repeat in interact_step); this one is
            // `LocalPlayer.isHandsBusy` — rowing (a movement key held while controlling a
            // boat): no use at all.
            self.int.use_delay = USE_DELAY;
            if !self.int.is_using_item && !self.hands_busy {
                for hand in 0..=1 {
                    // `MultiPlayerGameMode.useItem` answers spectator with a Pass before it sends
                    // anything: no `use_item` packet and no use. `useItemOn`'s packet still
                    // goes out (see the BLOCK arm), but its client half is a CONSUME without a
                    // swing, so the use stops at the block.
                    let spectator = self.game_mode == GameMode::Spectator;
                    let holding = !spectator && !self.hand(hand).is_empty();
                    match self.int.target {
                        Some(Target::Entity {
                            id,
                            location,
                            pos,
                            kind,
                        }) => {
                            // `Minecraft.startUseItem`'s ENTITY case returns before anything
                            // when the entity stands outside the world border: no interact
                            // packet, no fall-through to the other hand.
                            if !env
                                .border
                                .is_none_or(|b| b.is_within_bounds(BlockPos::from_dvec3(pos)))
                            {
                                return;
                            }
                            // gameMode.interact: the interact packet goes out (the mod-driven
                            // result that decides the swing comes back with the server's answer),
                            // with the hit location relative to the entity and the shift key's
                            // state (`player.isShiftKeyDown`, the raw key).
                            out.push(ServerboundPlay::Interact(sb::Interact {
                                entity_id: id.0,
                                hand,
                                target_offset: location - pos,
                                sneaking: sneak_key,
                            }));
                            // `Player.interactOn` runs the entity's `interact` on the client
                            // too; a Pass (a mob without a use for our hand, food on a zombie)
                            // falls through to `gameMode.useItem` with the same hand, then the
                            // off hand, like the block branch above. Only a pair whose
                            // client-side result consumes counts as the Success that returns.
                            let held = self.hand(hand).clone();
                            if self.entity_use_consumes(kind, &held, env.data) {
                                // `Minecraft.startUseItem` swings on a successful use
                                // (vanilla's swing source PREDICTED).
                                self.hand.swing();
                                return;
                            }
                        }
                        Some(Target::Block(h)) => {
                            // `MultiPlayerGameMode.useItemOn` fails before it sends anything
                            // when the block is outside the world border, and `Minecraft
                            // .startUseItem` returns on a Fail: no packet, no fall-through to
                            // `useItem` or the other hand.
                            if !env.border.is_none_or(|b| b.is_within_bounds(h.pos)) {
                                return;
                            }
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
                            // Spectator: `performUseItemOn`'s only answer is a CONSUME (no
                            // swing, no item change), so the use stops at the packet.
                            if spectator {
                                return;
                            }
                            // The block's own use opens the game-master editors on the client
                            // (`CommandBlock.useWithoutItem` -> `LocalPlayer.openCommandBlock`;
                            // the book's `useItemOn` is never consumed, so `Minecraft.startUseItem`
                            // falls through to `useItem` -> `WritableBookItem.use`).
                            self.use_opens_screen(env, Some((h.pos, world)), hand);
                            // `Minecraft.startUseItem` swings only on a Success (swing source
                            // PREDICTED) and returns; `itemUsed` when the stack changed (a
                            // placed block). A Pass (food, a tool, an empty hand on a plain
                            // block) goes on to `useItem` with the same hand. Without the
                            // block's Java `useItemOn`, a placed block item or a block the
                            // client knows to have a use count as Success; the server decides
                            // the rest. No `punch`: that packet belongs to attacks only.
                            let stack = self.hand(hand).clone();
                            // `performUseItemOn` runs the item's `useOn` only when the stack
                            // is not on cooldown (`!itemStack.isEmpty() &&
                            // !player.getCooldowns().isOnCooldown(itemStack)`): a cooldown
                            // answers a Pass — no placement prediction, and the use falls
                            // through to `useItem` with the same hand. The packet above went
                            // out either way, inside `startPrediction` as vanilla sends it.
                            let placing = self.is_block_item(&stack, env.data)
                                && !self.int.is_on_cooldown(&stack, env.data);
                            if placing {
                                // `BlockItem.place` -> `canPlace` -> `Level.isUnobstructed`:
                                // an entity box in the placement state's collision shape
                                // fails the placement (FAIL), and `Minecraft.startUseItem`
                                // returns on a Fail — no swing, no place sound, no
                                // fall-through to `useItem` or the other hand. The
                                // `use_item_on` packet above went out either way, inside
                                // `startPrediction` as vanilla sends it.
                                if let Some(place) = self.placement(h, &stack, world, env, look.0) {
                                    // The prediction's local half (`ClientLevel.setBlock`
                                    // under `startPredicting`): the block is in the client
                                    // world this same tick, so physics and the renderer see
                                    // it at once, and the server's state of the cell is
                                    // retained — with where we stand — until the ack
                                    // (`BlockStatePredictionHandler`, see `prediction.rs`).
                                    if let Some(state) = place.state {
                                        let old = world.block_state(place.pos);
                                        self.int.prediction.set_predicted(
                                            seq,
                                            place.pos,
                                            old,
                                            self.state.pos,
                                            state,
                                        );
                                    }
                                    self.hand.swing();
                                    self.hand.item_used();
                                    // `BlockItem.place` plays the place sound on the client
                                    // too (the server's copy excludes us), at the placement
                                    // position. The held block item's block decides; unknown
                                    // items play nothing.
                                    self.own_place_sound(place.pos, &stack, env);
                                }
                                return;
                            }
                            if self.block_has_use(world.block_state(h.pos), env.data) {
                                self.hand.swing();
                                return;
                            }
                        }
                        None => {}
                    }
                    // gameMode.useItem with nothing targeted: the packet goes out, then
                    // `itemStack.use` runs on the client. Only a Success answers the swing
                    // and the equip drop (`Minecraft.startUseItem`: the swing on
                    // `SwingSource.PREDICTED`, `player.itemUsed` on any Success — CONSUME,
                    // eating or drawing a bow, drops the hand without swinging); a Pass
                    // (a dirt block, redstone dust, a stick) and a Fail (food the player
                    // cannot eat) swing nothing and fall through to the other hand.
                    if holding {
                        let seq = self.int.next_sequence();
                        out.push(ServerboundPlay::UseItem(sb::UseItem {
                            hand,
                            sequence: seq,
                            yaw: look.0,
                            pitch: look.1,
                        }));
                        let stack = self.hand(hand).clone();
                        // `MultiPlayerGameMode.useItem` sends the packet even for a stack on
                        // cooldown (it returns the packet from the prediction), but skips
                        // `itemStack.use` — no screen opens and no use starts locally; the
                        // Pass falls through to the other hand.
                        if self.int.is_on_cooldown(&stack, env.data) {
                            continue;
                        }
                        // `WritableBookItem.use`: SUCCESS — swing, itemUsed, its editor.
                        if self.use_opens_screen(env, None, hand) {
                            self.hand.swing();
                            self.hand.item_used();
                            return;
                        }
                        // `LivingEntity.startUsingItem`: an item with a use duration starts
                        // using (`Item.getUseDuration`/`getUseAnimation` from the consumable
                        // component or the item class).
                        // Food starts only when the player can eat (`Consumable.startConsuming`
                        // -> `canConsume` -> `Player.canEat`); the packet goes out either way.
                        if self.can_consume(&stack, env.data) {
                            if let Some(using) = self.int.use_data(&stack, env.data) {
                                self.int.using = Some(using);
                                self.int.is_using_item = true;
                                // The CONSUME Success's `player.itemUsed(hand)`.
                                self.hand.item_used();
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    /// `MultiPlayerGameMode.releaseUsingItem`: `player_action` release_use_item with a zeroed
    /// pos, then the player stops using the item.
    pub(super) fn release_using_item(&mut self, out: &mut Vec<ServerboundPlay>) {
        out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
            status: action::RELEASE_USE_ITEM,
            pos: BlockPos::new(0, 0, 0),
            face: Direction::Down as u8,
            sequence: 0,
        }));
        self.int.is_using_item = false;
        self.int.using = None;
    }

    /// The screens a use opens client-side (vanilla `MultiPlayerGameMode.performUseItemOn` /
    /// `useItem`): the command-block editor on its block (with `target`), the book editor for a
    /// writable book in hand (also with `target`: no block consumes a book's use, so vanilla
    /// falls through to `itemStack.use`). The game thread takes it and hands it to the UI.
    /// Returns whether a screen opened: the book's use is a SUCCESS, so `Minecraft
    /// .startUseItem` swings and stops there instead of trying the other hand.
    fn use_opens_screen(
        &mut self,
        env: &InteractEnv<'_>,
        target: Option<(BlockPos, &dyn BlockView)>,
        hand: i32,
    ) -> bool {
        if let Some((pos, world)) = target {
            let block = env.data.blocks.block_of(world.block_state(pos));
            if [
                Identifier::minecraft("command_block"),
                Identifier::minecraft("chain_command_block"),
                Identifier::minecraft("repeating_command_block"),
            ]
            .contains(block)
            {
                self.int.used_screen = Some(UsedScreen::CommandBlock(pos));
                return true;
            }
        }
        let stack = self.hand(hand);
        if stack.count > 0
            && stack.item.is_some_and(|item| {
                env.data
                    .registries
                    .name("minecraft:item", item)
                    .is_some_and(|n| *n == Identifier::minecraft("writable_book"))
            })
        {
            self.int.used_screen = Some(UsedScreen::ItemGui(hand));
            return true;
        }
        false
    }

    /// `Player.canEat` for a food item (`Consumable.canConsume`): invulnerable, `can_always_eat`
    /// or a food level below 20. Anything that is not food can always be consumed.
    fn can_consume(&self, item: &ItemStackData, data: &haste_data::GameData) -> bool {
        let Some(ic) = &self.int.item_components else {
            return true;
        };
        let Some(name) = item
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))
        else {
            return true;
        };
        match ic.food(name) {
            Some(can_always_eat) => self.abilities.invulnerable || can_always_eat || self.food < 20,
            None => true,
        }
    }
}
