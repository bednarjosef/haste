//! Dropping from the hotbar (Q, Ctrl+Q) and the off-hand swap (F):
//! `Minecraft.handleKeybinds`' keyDrop / keySwapOffhand loops ->
//! `MultiPlayerGameMode.dropItem` and the swap's player_action.

use super::*;

#[test]
fn drop_item_key_sends_status_5_once_per_press() {
    let mut r = Rig::new(GameMode::Survival);
    let q = InteractInput {
        drop_item: true,
        ..Default::default()
    };
    let out = r.tick(q);
    assert_eq!(actions(&out), [(action::DROP_ITEM, 0)]);
    // Held, and on the tick after release: nothing more.
    assert!(r.tick(q).is_empty());
    assert!(r.tick(InteractInput::default()).is_empty());
    let out = r.tick(q);
    assert_eq!(actions(&out), [(action::DROP_ITEM, 0)]);
}

#[test]
fn drop_stack_key_sends_status_4() {
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(InteractInput {
        drop_stack: true,
        ..Default::default()
    });
    assert_eq!(actions(&out), [(action::DROP_ALL_ITEMS, 0)]);
    assert!(r
        .tick(InteractInput {
            drop_stack: true,
            ..Default::default()
        })
        .is_empty());
}

#[test]
fn swap_offhand_key_sends_status_7_with_zeroed_pos() {
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(InteractInput {
        swap_offhand: true,
        ..Default::default()
    });
    // Vanilla zeroes pos, face and sequence for these actions; the server updates the slots.
    let sw: Vec<&sb::PlayerAction> = out
        .iter()
        .filter_map(|p| match p {
            ServerboundPlay::PlayerAction(a) => Some(a),
            _ => None,
        })
        .collect();
    assert_eq!(
        sw,
        [&sb::PlayerAction {
            status: action::SWAP_ITEM_WITH_OFFHAND,
            pos: BlockPos::new(0, 0, 0),
            face: 0,
            sequence: 0,
        }]
    );
    assert!(r
        .tick(InteractInput {
            swap_offhand: true,
            ..Default::default()
        })
        .is_empty());
}

/// `MultiPlayerGameMode.dropItem` predicts the removal client-side
/// (`Inventory.removeFromSelected`) and swings the main hand when the removed stack was not
/// empty. `Minecraft.handleKeybinds`' keyDrop / keySwapOffhand loops run independently, all
/// gated on `!player.isSpectator()` (not health).
#[test]
fn drop_predicts_the_removal_and_swings_and_runs_independently() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = ItemStackData {
        count: 3,
        item: Some(RegistryIndex(3)),
        components_raw: Default::default(),
    };
    // One item leaves the slot, the hand swings.
    let out = r.tick(InteractInput {
        drop_item: true,
        ..Default::default()
    });
    assert_eq!(actions(&out), [(action::DROP_ITEM, 0)]);
    assert_eq!(r.p.main_hand().count, 2, "the removal is predicted");
    assert!(r.p.hand.is_swinging(), "a non-empty removal swings");
    // The stack drop empties the slot.
    let out = r.tick(InteractInput {
        drop_stack: true,
        ..Default::default()
    });
    assert_eq!(actions(&out), [(action::DROP_ALL_ITEMS, 0)]);
    assert!(r.p.main_hand().is_empty());
    // Dropping from an empty slot sends the packet but does not swing.
    r.p.hand = crate::hand::Hand::new();
    let out = r.tick(InteractInput {
        drop_item: true,
        ..Default::default()
    });
    assert_eq!(actions(&out), [(action::DROP_ITEM, 0)]);
    assert!(!r.p.hand.is_swinging(), "an empty removal does not swing");

    // All three in one tick (Q + Ctrl+Q + F): each loop runs, in vanilla's order.
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(InteractInput {
        drop_stack: true,
        drop_item: true,
        swap_offhand: true,
        ..Default::default()
    });
    assert_eq!(
        actions(&out),
        [
            (action::DROP_ALL_ITEMS, 0),
            (action::DROP_ITEM, 0),
            (action::SWAP_ITEM_WITH_OFFHAND, 0)
        ]
    );

    // A spectator drops and swaps nothing.
    let mut r = Rig::new(GameMode::Spectator);
    let out = r.tick(InteractInput {
        drop_stack: true,
        drop_item: true,
        swap_offhand: true,
        ..Default::default()
    });
    assert!(out.is_empty(), "{out:?}");
}

/// The drop asks the Java inventory for the removal — the one the HUD and screens read —
/// with `all`, and keeps vanilla's rules (`MultiPlayerGameMode.dropItem`): the packet always
/// goes out, the main hand swings only when the removed stack was not empty. The closure
/// stands in for the live UI bridge.
#[test]
fn drop_item_asks_the_java_inventory_and_swings_only_for_a_non_empty_removal() {
    let mut r = Rig::new(GameMode::Survival);
    let asked = std::cell::RefCell::new(Vec::new());
    // A non-empty stack left the slot: asked with all=false, packet out, hand swings.
    let mut out = Vec::new();
    r.p.drop_item(
        &|all| {
            asked.borrow_mut().push(all);
            false
        },
        false,
        &mut out,
    );
    assert_eq!(*asked.borrow(), [false], "the Java removal is requested");
    assert_eq!(actions(&out), [(action::DROP_ITEM, 0)]);
    assert!(r.p.hand.is_swinging(), "a non-empty removal swings");
    // Nothing left the slot: asked with all=true, the packet still goes out, no swing.
    r.p.hand = crate::hand::Hand::new();
    let mut out = Vec::new();
    r.p.drop_item(
        &|all| {
            asked.borrow_mut().push(all);
            true
        },
        true,
        &mut out,
    );
    assert_eq!(*asked.borrow(), [false, true]);
    assert_eq!(actions(&out), [(action::DROP_ALL_ITEMS, 0)]);
    assert!(!r.p.hand.is_swinging(), "an empty removal does not swing");
}
