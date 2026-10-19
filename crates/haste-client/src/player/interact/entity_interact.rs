//! The entity half of a use (player workstream): `Player.interactOn`'s client-side halves,
//! moved out of `use_item.rs` to keep that file within the size guard (a pure move).

use haste_core::entity::ItemStackData;
use haste_core::{Identifier, RegistryIndex};

use super::super::LocalPlayer;

impl LocalPlayer {
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
    pub(super) fn entity_use_consumes(
        &self,
        kind: RegistryIndex,
        stack: &ItemStackData,
        data: &haste_data::GameData,
    ) -> bool {
        let Some(name) = data.registries.name("minecraft:entity_type", kind) else {
            // An unknown (mod) type: its interact is the server's to answer.
            return false;
        };
        // `MultiPlayerGameMode.interact` answers spectator with a Pass: the packet went out,
        // nothing client-side consumes, the use falls through (to a useItem that does nothing).
        if self.game_mode == haste_core::game::GameMode::Spectator {
            return false;
        }
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
