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
                        Some(Target::Entity {
                            id,
                            location,
                            pos,
                            kind,
                        }) => {
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

    /// Whether the client half of `Player.interactOn` consumes the use (an `InteractionResult`
    /// that `consumesAction`, so `Minecraft.startUseItem` stops at the entity and does not go
    /// on to `gameMode.useItem` / the off hand). The packet went out either way; the server
    /// runs the real interact. Only pairs whose client-side branch vanilla resolves without the
    /// server count: a mob without a use for the hand, food on a zombie or a shield against one
    /// are a Pass, and the use falls through like the block branch's Pass.
    ///
    /// The item half (`ItemStack.interactLivingEntity`) runs first in vanilla; only a dye on
    /// a sheep consumes on the client (a saddle needs the mob's slot to be free, a name tag a
    /// custom name in the stack — the server answers those, and vanilla's `use_item` after
    /// them is a Pass too, so falling through is faithful).
    ///
    /// The entity half (`entity.interact` / `mobInteract`): the mobs whose client-side branch
    /// consumes — a villager's or wandering trader's trade, an animal held its food
    /// (`Animal.mobInteract`'s client `CONSUME`; the item tags the server synced, else the
    /// common foods), armor stands, item frames (the frame's content is not tracked: an empty
    /// hand on an empty frame is the client's Pass, which the server's answer settles), boats
    /// and minecarts offered a ride. Everything else the client leaves to the server (a
    /// `SUCCESS_SERVER`, `Entity.interact`'s leash branches, a `Pass`) falls through; an
    /// unknown (mod) type's interact is the server's to answer.
    fn entity_use_consumes(
        &self,
        kind: RegistryIndex,
        stack: &ItemStackData,
        data: &haste_data::GameData,
    ) -> bool {
        let Some(name) = data.registries.name("minecraft:entity_type", kind) else {
            // An unknown (mod) type: its interact is the server's to answer.
            return false;
        };
        if name.namespace() != "minecraft" {
            return false;
        }
        let item = stack
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))
            .map(|i| i.path().to_owned());
        // `ItemStack.interactLivingEntity`: only a dye on a sheep consumes client-side (a
        // saddle needs the mob's free slot, a name tag a custom name — the server answers).
        match item.as_deref() {
            Some(i) if i.ends_with("_dye") && name.path() == "sheep" => return true,
            _ => {}
        }
        // The entity's own `interact` / `mobInteract`, client-visible halves.
        match name.path() {
            "villager" | "wandering_trader" => true,
            // An item frame: the client consumes unless the frame is empty and so is the hand
            // (the frame's content is unknown here; a held item always consumes).
            "item_frame" | "glow_item_frame" => !stack.is_empty(),
            "armor_stand" => true,
            // `AbstractBoat.interact`'s client `startRiding` branch (not shift-held).
            "boat" | "chest_boat" | "raft" => true,
            "minecart"
            | "chest_minecart"
            | "furnace_minecart"
            | "hopper_minecart"
            | "tnt_minecart"
            | "command_block_minecart" => true,
            // `Animal.mobInteract` / the tameable mobs' food branches: the hand holds the
            // mob's food (`Animal.isFood`; the item tags the server synced, else the common
            // foods of the mob).
            animal => animal_food_tag(animal).is_some_and(|tag| {
                item.as_deref().is_some_and(|i| {
                    self.int.item_tags.as_ref().is_some_and(|t| {
                        let index = data
                            .registries
                            .index("minecraft:item", &Identifier::minecraft(i))
                            .map_or(u32::MAX, |i| i.0);
                        t.contains(&Identifier::minecraft(tag), index)
                    }) || is_common_food(animal, i)
                })
            }),
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

/// The `ItemTags.<ANIMAL>_FOOD` tag an entity type's `Animal.isFood` reads (`None` for mobs
/// that are not animals or never eat). The item tags themselves come from the server's
/// `update_tags` (registry `minecraft:item`); `is_common_food` is the fallback when they are
/// missing.
fn animal_food_tag(animal: &str) -> Option<&'static str> {
    Some(match animal {
        "cow" => "cow_food",
        "sheep" => "sheep_food",
        "pig" => "pig_food",
        "chicken" => "chicken_food",
        "horse" | "skeleton_horse" | "zombie_horse" => "horse_food",
        "mule" | "donkey" => "horse_food",
        "llama" | "trader_llama" => "llama_food",
        "goat" => "goat_food",
        "fox" => "fox_food",
        "cat" => "cat_food",
        "ocelot" => "ocelot_food",
        "rabbit" => "rabbit_food",
        "turtle" => "turtle_food",
        "panda" => "panda_food",
        "bee" => "bee_food",
        "axolotl" => "axolotl_food",
        "frog" => "frog_food",
        "sniffer" => "sniffer_food",
        "armadillo" => "armadillo_food",
        "happy_ghast" => "happy_ghast_food",
        "strider" => "strider_food",
        "hoglin" => "hoglin_food",
        "camel" => "camel_food",
        "wolf" => "wolf_food",
        _ => return None,
    })
}

/// The common foods of the animals whose item tag is missing (no `update_tags` for
/// `minecraft:item`): the items vanilla's tags name most, per mob. A wrong `true` here only
/// skips the `use_item` fall-through; the server still runs the real interact.
fn is_common_food(animal: &str, item: &str) -> bool {
    match animal {
        "cow" | "sheep" | "goat" | "horse" | "mule" | "donkey" => {
            matches!(item, "wheat" | "golden_apple" | "enchanted_golden_apple")
        }
        "pig" | "hoglin" | "strider" => matches!(item, "carrot" | "potato" | "beetroot"),
        "chicken" => matches!(item, "wheat_seeds" | "melon_seeds" | "pumpkin_seeds"),
        "wolf" => matches!(item, "bone" | "meat" | "chicken" | "cooked_chicken"),
        _ => false,
    }
}
