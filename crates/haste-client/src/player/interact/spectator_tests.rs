//! Spectator-mode interaction tests: the input rules vanilla's `Minecraft.handleKeybinds`
//! routes differently for a spectator. The rig and fixtures are [`super::tests`]'s.

use haste_core::game::GameMode;
use haste_core::input::InteractInput;
use haste_protocol::ServerboundPlay;

use super::tests::{Rig, ATTACK};

/// `Minecraft.handleKeybinds`' hotbar loop: in spectator the digit keys go to the
/// spectator menu (`SpectatorGui.onHotbarSelected`), never the inventory — no slot change,
/// no `set_carried_item` packet. (The menu itself is not wired up yet, so the pick is
/// dropped entirely.)
#[test]
fn spectator_hotbar_keys_change_no_slot_and_send_nothing() {
    let mut r = Rig::new(GameMode::Spectator);
    let pick = InteractInput {
        hotbar: Some(4),
        ..ATTACK
    };
    // The pick alone (no attack): the tick must produce no interaction packets at all.
    let pick = InteractInput {
        attack: false,
        ..pick
    };
    let out = r.tick(pick);
    assert!(
        out.is_empty(),
        "no set_carried_item or anything else for a spectator's digit key: {out:?}"
    );
    assert_eq!(r.p.held_slot, 0, "the slot did not move");
    // The same pick in survival sends set_carried_item, so the gate is the mode.
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(pick);
    assert!(matches!(
        out.as_slice(),
        [ServerboundPlay::SetCarriedItem(c)] if c.slot == 4
    ));
}
