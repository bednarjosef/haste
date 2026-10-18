//! The tick's input step: taking what the render thread sampled from winit into the player,
//! the way vanilla's `Minecraft.tick` -> `handleKeybinds` consumes the key bindings.

use super::shared::{Shared, SharedInput};
use crate::net::NetDriver;

/// This tick's input, sampled from winit by the render thread.
pub(super) fn sample(driver: &mut NetDriver, shared: &Shared) -> SharedInput {
    let mut shared_input = shared.input.lock();
    if std::mem::take(&mut shared_input.toggle_flight) {
        driver.session.player.toggle_flight();
    }
    // A screen being open is vanilla's `handleKeybinds` gate (`Minecraft.tick` runs the key
    // bindings only when `gui.screen() == null`): no attack or use happens while a screen
    // holds the input, and clicks queued for this tick are dropped, not saved (the release
    // that ends a use that opened a screen is the screen's, and a held button must not
    // repeat the use through the server's block editors — the sign editor would reopen).
    let screen_open = shared.ui.lock().as_ref().is_some_and(|ui| ui.screen_open());
    // Held buttons stay; the press-only middle click, drop/swap presses and the hotbar choice
    // are consumed.
    let mut interact = shared_input.interact;
    // Vanilla runs `Minecraft.handleKeybinds` (the attack/use clicks, the held use,
    // `continueAttack` and the keybind picks and drops) only with no screen open
    // (`Minecraft.tick`: `if (gui.overlay() == null && gui.screen() == null)`): the
    // player skips the whole key handling below while one is, so a running break and
    // an item use hang until the screen closes (`set_screen_open`). A screen taking the
    // mouse also owns the buttons (`KeyMapping.releaseAll` at the open): the held state
    // is zeroed, so the close tick releases a use and aborts a break, never repeats
    // them.
    driver.session.player.set_screen_open(screen_open);
    let attack_clicks;
    let use_clicks;
    if screen_open {
        interact.attack = false;
        interact.use_item = false;
        attack_clicks = 0;
        use_clicks = 0;
        shared_input.interact.attack_clicks = 0;
        shared_input.interact.use_clicks = 0;
    } else {
        // The click counts (vanilla's KeyMapping.clickCount) are drained into the player,
        // which consumes them in `Minecraft.handleKeybinds`' consumeClick loops.
        attack_clicks = std::mem::take(&mut shared_input.interact.attack_clicks);
        use_clicks = std::mem::take(&mut shared_input.interact.use_clicks);
    }
    shared_input.interact.pick_block = false;
    shared_input.interact.pick_block_ctrl = false;
    shared_input.interact.drop_item = false;
    shared_input.interact.drop_stack = false;
    shared_input.interact.swap_offhand = false;
    shared_input.interact.hotbar = None;
    // A hotbar slot the UI picked since the last tick (the HUD's digits and wheel, or
    // Java-side code) goes through the normal interact path, like the client's own
    // digits did before the UI owned them: `set_carried_item` is sent and a changed
    // held item restarts breaking. The server's `set_held_slot` changes both sides
    // alike, so it never looks like a UI pick.
    if interact.hotbar.is_none() {
        if let Some(ui) = shared.ui.lock().as_ref() {
            let slot = ui.selected_slot();
            if slot != driver.session.player.held_slot {
                interact.hotbar = Some(slot.clamp(0, 8) as u8);
            }
        }
    }
    driver.session.player.set_interact(interact);
    for _ in 0..attack_clicks {
        driver.session.player.queue_attack_click();
    }
    for _ in 0..use_clicks {
        driver.session.player.queue_use_click();
    }
    SharedInput {
        toggle_flight: shared_input.toggle_flight,
        interact: shared_input.interact,
        perspective: shared_input.perspective,
        state: shared_input.state,
    }
}
