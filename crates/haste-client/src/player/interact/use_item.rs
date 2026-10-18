//! Using items and blocks (player workstream): `Minecraft.startUseItem` per hand (entity
//! interact, block use, item use), `MultiPlayerGameMode.releaseUsingItem`, the client-side
//! screens a use opens, and the item-in-use state (`LivingEntity.useItem` /
//! `useItemRemaining`, `Item.getUseDuration` / `getUseAnimation`).

use glam::DVec3;
use haste_core::entity::ItemStackData;
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, Direction, Identifier, RegistryIndex};
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
    fn from_id(id: i32) -> Self {
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UsingItem {
    /// The item type this use started with (`ItemStack.isSameItem` check each tick).
    pub item: Option<RegistryIndex>,
    /// `getUseDuration` (ticks).
    pub duration: i32,
    /// `useItemRemainingTicks`, decremented each tick.
    pub remaining: i32,
    /// `getUseAnimation`.
    pub animation: UseAnimation,
}

impl super::Interaction {
    /// `Item.getUseDuration` / `Item.getUseAnimation` for a held stack: the `minecraft:consumable`
    /// component when present, else the item classes that override them. `None` when the item is
    /// not used over time (duration 0, animation `NONE`).
    pub(super) fn use_data(
        &self,
        stack: &ItemStackData,
        data: &haste_data::GameData,
    ) -> Option<(i32, UseAnimation)> {
        let item = stack
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))?;
        if let Some(ic) = &self.item_components {
            if let Some(c) = ic.consumable(item) {
                return Some((c.consume_ticks(), UseAnimation::from_id(c.animation)));
            }
        }
        // Items whose class overrides `getUseDuration`/`getUseAnimation`
        // (`BowItem`/`CrossbowItem`/`TridentItem`/`SpyglassItem`/`ShieldItem`); a shield's
        // `minecraft:blocks_attacks` makes the default `Item` return 72000 and `BLOCK`.
        match item.path() {
            "bow" => Some((72000, UseAnimation::Bow)),
            "crossbow" => Some((72000, UseAnimation::Crossbow)),
            "trident" => Some((72000, UseAnimation::Trident)),
            "spyglass" => Some((1200, UseAnimation::Spyglass)),
            "shield" => Some((72000, UseAnimation::Block)),
            _ => None,
        }
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

    /// `LivingEntity.getUseItemRemainingTicks`.
    pub fn use_item_remaining(&self) -> i32 {
        self.int.using.map_or(0, |u| u.remaining)
    }

    /// `LivingEntity.getUseItem`'s duration (`Item.getUseDuration`), 0 when not using.
    pub fn use_duration(&self) -> i32 {
        self.int.using.map_or(0, |u| u.duration)
    }

    /// `Item.getUseAnimation` of the item being used.
    pub fn use_animation(&self) -> UseAnimation {
        self.int.using.map_or(UseAnimation::None, |u| u.animation)
    }

    /// `LivingEntity.getTicksUsingItem`: how long the current use has run.
    pub fn ticks_using_item(&self) -> i32 {
        self.int.using.map_or(0, |u| u.duration - u.remaining)
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
            self.int.use_delay = USE_DELAY;
            if !self.int.is_using_item {
                for hand in 0..=1 {
                    let holding = !self.hand(hand).is_empty();
                    match self.int.target {
                        Some(Target::Entity { id, location, pos }) => {
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
                            let placing = self.is_block_item(&stack, env.data);
                            if placing || self.block_has_use(world.block_state(h.pos), env.data) {
                                self.hand.swing();
                                if placing {
                                    self.hand.item_used();
                                    // `BlockItem.place` plays the place sound on the client
                                    // too (the server's copy excludes us). The held block
                                    // item's block decides; unknown items play nothing.
                                    self.own_place_sound(h.pos, &stack, env);
                                }
                                return;
                            }
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
                        // `itemStack.use`: a writable book opens its editor
                        // (`WritableBookItem.use` -> `LocalPlayer.openItemGui`).
                        self.use_opens_screen(env, None, hand);
                        // `LivingEntity.startUsingItem`: an item with a use duration starts
                        // using (`Item.getUseDuration`/`getUseAnimation` from the consumable
                        // component or the item class).
                        // Food starts only when the player can eat (`Consumable.startConsuming`
                        // -> `canConsume` -> `Player.canEat`); the packet goes out either way.
                        let stack = self.hand(hand).clone();
                        if !self.can_consume(&stack, env.data) {
                            return;
                        }
                        if let Some((duration, animation)) = self.int.use_data(&stack, env.data) {
                            self.int.using = Some(UsingItem {
                                item: stack.item,
                                duration,
                                remaining: duration,
                                animation,
                            });
                            self.int.is_using_item = true;
                        }
                        return;
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
    fn use_opens_screen(
        &mut self,
        env: &InteractEnv<'_>,
        target: Option<(BlockPos, &dyn BlockView)>,
        hand: i32,
    ) {
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
                return;
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
        }
    }

    /// The held item's block, if it is a block item (item id = block id in vanilla,
    /// `BlockItemId.create(name)`; the 14 crop pairs with differing names are missing
    /// here — placing crops is rare and the server confirms anyway).
    fn held_block<'d>(
        &self,
        item: &ItemStackData,
        data: &'d haste_data::GameData,
    ) -> Option<&'d haste_core::Identifier> {
        if item.is_empty() {
            return None;
        }
        let index = item.item?;
        data.registries
            .name("minecraft:item", index)
            .and_then(|id| data.blocks.default_state(id).map(|_| id))
    }

    /// Whether `item` is a block item (`BlockItem`): its `use_item_on` places a block, a Success
    /// on the client.
    fn is_block_item(&self, item: &ItemStackData, data: &haste_data::GameData) -> bool {
        self.held_block(item, data).is_some()
    }

    /// Whether the block's own `useWithoutItem`/`useItemOn` is known to succeed on the client
    /// (containers, doors, redstone switches, workstations, beds, ...), so `startUseItem`
    /// stops at it instead of falling through to `useItem`. Mod blocks and anything not listed
    /// pass through; the server still runs the real use.
    fn block_has_use(&self, state: BlockStateId, data: &haste_data::GameData) -> bool {
        const USES: &[&str] = &[
            "chest",
            "barrel",
            "shulker_box",
            "furnace",
            "smoker",
            "crafting_table",
            "door",
            "trapdoor",
            "fence_gate",
            "button",
            "lever",
            "bed",
            "anvil",
            "enchanting_table",
            "brewing_stand",
            "hopper",
            "dispenser",
            "dropper",
            "note_block",
            "repeater",
            "comparator",
            "daylight_detector",
            "bell",
            "lectern",
            "cartography_table",
            "smithing_table",
            "grindstone",
            "stonecutter",
            "loom",
            "beacon",
            "command_block",
            "jukebox",
            "crafter",
            "respawn_anchor",
        ];
        let path = data.blocks.block_of(state).path();
        USES.iter().any(|u| {
            path.strip_suffix(*u)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('_'))
        })
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

    /// The place sound for a `use_item_on` with a block item held (see `use_item`).
    fn own_place_sound(&mut self, pos: BlockPos, item: &ItemStackData, env: &InteractEnv<'_>) {
        let Some(block) = self.held_block(item, env.data) else {
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
}
