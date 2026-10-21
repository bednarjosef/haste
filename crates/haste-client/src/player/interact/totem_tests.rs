//! `findTotem`: the main hand, then the off hand, then a fresh totem (vanilla
//! `ClientPacketListener.findTotem`).

use super::Interaction;
use haste_core::entity::ItemStackData;
use haste_core::RegistryIndex;

const TOTEM: u32 = 900;
const SWORD: u32 = 7;

fn item(id: u32) -> ItemStackData {
    ItemStackData {
        count: 1,
        item: Some(RegistryIndex(id)),
        ..Default::default()
    }
}

fn interaction(off: ItemStackData) -> Interaction {
    Interaction {
        totem_item: Some(RegistryIndex(TOTEM)),
        offhand: off,
        ..Default::default()
    }
}

#[test]
fn main_hand_totem_wins() {
    let int = interaction(item(TOTEM));
    assert_eq!(int.find_totem(&item(TOTEM)), item(TOTEM));
    let int = interaction(item(SWORD));
    assert_eq!(int.find_totem(&item(TOTEM)), item(TOTEM));
}

#[test]
fn off_hand_totem_when_the_main_hand_is_not_one() {
    let off = item(TOTEM);
    let int = interaction(off.clone());
    assert_eq!(int.find_totem(&item(SWORD)), off);
}

#[test]
fn fresh_totem_when_neither_hand_holds_one() {
    let int = interaction(item(SWORD));
    assert_eq!(
        int.find_totem(&item(SWORD)),
        ItemStackData {
            count: 1,
            item: Some(RegistryIndex(TOTEM)),
            components_raw: Default::default(),
        }
    );
}
