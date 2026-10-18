//! The click-bit feed (`consume_clicks` -> the shared interact input).

use super::*;
use haste_core::input::InteractInput;

/// The click bits land in their interact fields: a plain drop click sets `drop_item`, the
/// Ctrl-held drop click (`C_DROP_STACK`, vanilla `Minecraft.handleKeybinds` ->
/// `dropItem(player, hasControlDown())`) sets `drop_stack` instead, and the hotbar and swap
/// bits pick the slot and flag the swap.
#[test]
fn click_bits_feed_the_interact_input() {
    let mut i = InteractInput::default();
    feed_clicks(bits::C_DROP, &mut i);
    assert!(i.drop_item, "a plain drop click drops one item");
    assert!(!i.drop_stack);
    let mut i = InteractInput::default();
    feed_clicks(bits::C_DROP_STACK, &mut i);
    assert!(i.drop_stack, "the Ctrl-held drop click drops the stack");
    assert!(!i.drop_item, "it is not also a single-item drop");
    let mut i = InteractInput::default();
    feed_clicks(bits::C_HOTBAR_0 << 3, &mut i);
    assert_eq!(i.hotbar, Some(3));
    let mut i = InteractInput::default();
    feed_clicks(bits::C_SWAP_OFFHAND, &mut i);
    assert!(i.swap_offhand);
    // Nothing else in the mask feeds a field (the jump click is the caller's own).
    let mut i = InteractInput::default();
    feed_clicks(bits::C_JUMP, &mut i);
    assert_eq!(
        (i.drop_item, i.drop_stack, i.swap_offhand, i.hotbar),
        (false, false, false, None)
    );
}
