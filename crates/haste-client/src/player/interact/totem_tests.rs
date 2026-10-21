//! `findTotem`: the main hand, then the off hand, then a fresh totem (vanilla
//! `ClientPacketListener.findTotem`), with `ItemStack.has(DEATH_PROTECTION)` read from the
//! stack's component patch first and the item's defaults second.

use std::sync::Arc;

use super::Interaction;
use haste_core::entity::ItemStackData;
use haste_core::RegistryIndex;
use haste_protocol::packets::ProtocolTables;

const TOTEM: u32 = 900;
const SWORD: u32 = 7;
/// `minecraft:death_protection`'s data component id in these tests.
const DEATH_PROTECTION: u8 = 3;

fn item(id: u32) -> ItemStackData {
    ItemStackData {
        count: 1,
        item: Some(RegistryIndex(id)),
        ..Default::default()
    }
}

/// `item` with a component patch that adds death protection (no death effects).
fn with_death_protection(id: u32) -> ItemStackData {
    ItemStackData {
        // One add, no removes; the component's id, then an empty `death_effects` list.
        components_raw: vec![1, 0, DEATH_PROTECTION, 0].into(),
        ..item(id)
    }
}

/// `item` with a component patch that removes death protection.
fn without_death_protection(id: u32) -> ItemStackData {
    ItemStackData {
        components_raw: vec![0, 1, DEATH_PROTECTION].into(),
        ..item(id)
    }
}

fn interaction(off: ItemStackData) -> Interaction {
    Interaction {
        totem_item: Some(RegistryIndex(TOTEM)),
        // The item component reports: only the totem carries death protection by default.
        death_protection_items: Some([RegistryIndex(TOTEM)].into_iter().collect()),
        component_tables: Some(Arc::new(ProtocolTables::for_tests(&[
            (0, "minecraft:max_stack_size"),
            (DEATH_PROTECTION as i32, "minecraft:death_protection"),
        ]))),
        offhand: off,
        ..Default::default()
    }
}

fn fresh_totem() -> ItemStackData {
    ItemStackData {
        count: 1,
        item: Some(RegistryIndex(TOTEM)),
        components_raw: Default::default(),
    }
}

#[test]
fn main_hand_stack_with_death_protection_wins() {
    let int = interaction(item(TOTEM));
    let main = with_death_protection(SWORD);
    assert_eq!(int.find_totem(&main), main);
    let int = interaction(item(SWORD));
    assert_eq!(int.find_totem(&item(TOTEM)), item(TOTEM));
}

#[test]
fn off_hand_totem_when_the_main_hand_has_no_death_protection() {
    let off = item(TOTEM);
    let int = interaction(off.clone());
    assert_eq!(int.find_totem(&item(SWORD)), off);
    // A main-hand totem whose patch removes the component is passed over.
    assert_eq!(int.find_totem(&without_death_protection(TOTEM)), off);
}

#[test]
fn a_patched_stack_counts_whatever_its_item() {
    // A sword given death protection by a command, in the off hand.
    let off = with_death_protection(SWORD);
    let int = interaction(off.clone());
    assert_eq!(int.find_totem(&item(SWORD)), off);
}

#[test]
fn fresh_totem_when_neither_hand_has_death_protection() {
    let int = interaction(item(SWORD));
    assert_eq!(int.find_totem(&item(SWORD)), fresh_totem());
    // Totems whose patches remove the component, and empty hands, do not count either.
    let int = interaction(without_death_protection(TOTEM));
    assert_eq!(int.find_totem(&ItemStackData::default()), fresh_totem());
}

#[test]
fn an_undecodable_patch_falls_back_to_the_item_default() {
    // Component id 9 is unknown to the tables: the patch does not decode.
    let broken = ItemStackData {
        components_raw: vec![1, 0, 9, 0].into(),
        ..item(TOTEM)
    };
    let int = interaction(item(SWORD));
    assert_eq!(int.find_totem(&broken), broken);
}
