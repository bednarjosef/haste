//! The input half of [`Ui`] (jvm-host): winit's events forwarded to the open screen, and with
//! no screen open, the game key bindings mirrored into the vanilla `KeyMapping`s (the [`bits`]
//! masks the client reads back). See the module documentation of [`crate::ui`].

use super::*;

/// The bit values of `haste.gui.OptionBridge`'s game-input and click masks (the Java constants
/// are the source; keep these in step with them).
pub mod bits {
    /// The movement and action `KeyMapping`s' down state (`Bridge.gameInput`).
    pub const B_FORWARD: i32 = 1;
    pub const B_BACK: i32 = 1 << 1;
    pub const B_LEFT: i32 = 1 << 2;
    pub const B_RIGHT: i32 = 1 << 3;
    pub const B_JUMP: i32 = 1 << 4;
    pub const B_SNEAK: i32 = 1 << 5;
    pub const B_SPRINT: i32 = 1 << 6;
    pub const B_ATTACK: i32 = 1 << 7;
    pub const B_USE: i32 = 1 << 8;

    /// Clicks of the action `KeyMapping`s not yet consumed (`Bridge.consumeClicks`).
    pub const C_HOTBAR_0: i32 = 1;
    pub const C_DROP: i32 = 1 << 9;
    pub const C_SWAP_OFFHAND: i32 = 1 << 10;
    pub const C_JUMP: i32 = 1 << 11;
    /// The drop click consumed while Ctrl is held: the whole stack leaves the slot (vanilla
    /// `Minecraft.handleKeybinds` passes `hasControlDown()` to `dropItem`).
    pub const C_DROP_STACK: i32 = 1 << 12;
}

/// Feeds the action-mapping click bits ([`Ui::consume_clicks`]) into the shared interact
/// input: the hotbar picks, the drop (one item, or the whole stack when Ctrl was held at the
/// click) and the off-hand swap. The jump click is not an interact field: the caller routes
/// it to its own press logic.
pub fn feed_clicks(c: i32, i: &mut haste_core::input::InteractInput) {
    for slot in 0..9 {
        if c & (bits::C_HOTBAR_0 << slot) != 0 {
            i.hotbar = Some(slot as u8);
        }
    }
    i.drop_item |= c & bits::C_DROP != 0;
    i.drop_stack |= c & bits::C_DROP_STACK != 0;
    i.swap_offhand |= c & bits::C_SWAP_OFFHAND != 0;
}
impl Ui {
    /// A game key event (no screen open, SDL3 scancode) mirrored into the vanilla `KeyMapping`s,
    /// vanilla's `KeyboardHandler.keyPress` (press sets the down state and counts a click).
    /// `mods` is the SDL keymod state, kept current in Java for the Ctrl-at-consume reads
    /// (the drop's one-item-or-stack).
    pub fn game_key(&self, scancode: i32, pressed: bool, mods: i32) {
        self.gui().game_key(scancode, pressed, mods);
    }

    /// A game mouse button (no screen open, vanilla numbering) mirrored into the vanilla
    /// `KeyMapping`s, vanilla's `MouseHandler.onPress`.
    pub fn game_mouse(&self, button: i32, pressed: bool) {
        self.gui().game_mouse(button, pressed);
    }

    /// The down state of the movement and action `KeyMapping`s as a [`bits`] mask: the controls
    /// the client reads are the player's own bindings.
    pub fn game_input(&self) -> i32 {
        self.gui().game_input()
    }

    /// Consumes the action `KeyMapping` clicks (hotbar, drop, drop stack, swap offhand, jump)
    /// since the last call as a [`bits`] mask, one bit per click.
    pub fn consume_clicks(&self) -> i32 {
        self.gui().consume_clicks()
    }

    /// A key event (26.3 numbering, see [`keys`]): `key` is the SDL scancode, `keycode` the SDL
    /// keycode, `mods` SDL keymod bits. Returns true when the UI used it (the client must not also
    /// treat it as game input).
    pub fn key(&self, key: i32, keycode: i32, mods: i32, pressed: bool) -> bool {
        let gui = self.gui();
        if gui.is_open() {
            gui.key(key, keycode, mods, pressed);
            // Vanilla code the keypress made send (E or Escape closing a container sends
            // `container_close`) leaves at once, like `Connection.send` inside the event.
            self.send_screen_packets_now();
            return true;
        }
        // No screen: vanilla's `KeyboardHandler.keyPress` sets the `KeyMapping`s (`handlesGameInput`),
        // including the HUD's own keys, which are KeyMappings in vanilla too. The client reads the
        // movement and action mappings back through `game_input`/`consume_clicks`; the keymod
        // state travels along, so Java can read Ctrl where vanilla polls the live keyboard.
        gui.game_key(key, pressed, mods);
        if key == keys::TAB {
            self.shared.tab_down.store(pressed, Ordering::Relaxed);
            gui.set_tab_list_visible(pressed);
            return true;
        }
        if !pressed {
            return false;
        }
        let open = |kind| {
            if let Err(e) = gui.open_screen(kind) {
                tracing::warn!("opening {kind:?}: {e:#}");
            }
            true
        };
        match key {
            k if k == keys::T => open(ScreenKind::Chat),
            k if k == keys::SLASH => open(ScreenKind::Command),
            k if k == keys::E => open(ScreenKind::Inventory),
            k if k == keys::ESCAPE => open(ScreenKind::Pause),
            k if k == keys::F1 => {
                gui.toggle_hud();
                self.shared.hud_hidden.fetch_xor(true, Ordering::Relaxed);
                true
            }
            k if k == keys::F3 => {
                gui.toggle_debug();
                true
            }
            k if (keys::NUM_1..keys::NUM_1 + 9).contains(&k) => {
                // Vanilla `Minecraft.handleKeybinds`' hotbar loop: in spectator the digits
                // go to the spectator menu (`SpectatorGui.onHotbarSelected`), never the
                // inventory slot.
                if self.state(|s| s.game_mode == GameMode::Spectator) {
                    self.spectator_hotbar_selected(k - keys::NUM_1);
                } else {
                    self.state(|s| s.selected_slot = k - keys::NUM_1);
                }
                true
            }
            _ => false,
        }
    }

    /// A typed character (to the open screen's focused text field).
    pub fn char_typed(&self, c: char, mods: i32) -> bool {
        self.gui().is_open() && self.gui().char_typed(c, mods)
    }

    /// The mouse moved (GUI-scaled coordinates). True when a screen is open.
    pub fn mouse_moved(&self, x: f64, y: f64) -> bool {
        if !self.gui().is_open() {
            return false;
        }
        self.gui().mouse(x, y, None);
        true
    }

    /// A mouse button (vanilla numbering: 1 left, 2 middle, 3 right) at (x, y) in GUI pixels.
    pub fn mouse_button(&self, x: f64, y: f64, button: i32, pressed: bool) -> bool {
        let gui = self.gui();
        if !gui.is_open() {
            // No screen: vanilla's `MouseHandler.onPress` sets the mouse `KeyMapping`s.
            gui.game_mouse(button, pressed);
            // Vanilla `Minecraft.handleKeybinds`: the key bound to `keySpectatorHotbar`
            // (mouse middle) opens or activates the spectator menu, in spectator mode only.
            if pressed && button == 2 && self.state(|s| s.game_mode == GameMode::Spectator) {
                self.spectator_hotbar_action_key();
            }
            return false;
        }
        gui.mouse(x, y, Some((button, pressed)));
        // What the click made the screen send (a `container_click`) leaves at once, like
        // vanilla's `Connection.send` inside the event handler.
        self.send_screen_packets_now();
        true
    }

    /// The scroll wheel: to the open screen, else the hotbar selection (one slot per notch, like
    /// vanilla; wheel down moves right).
    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64) -> bool {
        if self.gui().is_open() {
            self.gui().scroll(x, y, dx, dy);
            return true;
        }
        if dy == 0.0 {
            return false;
        }
        let step = if dy > 0.0 { -1 } else { 1 };
        self.state(|s| s.selected_slot = (s.selected_slot + step).rem_euclid(9));
        true
    }

    // ---- the spectator menu (vanilla `SpectatorGui`; see `ui/spectator`) ----

    /// Whether the spectator menu is open (`SpectatorGui.isMenuActive`; the wheel's
    /// spectator branch asks before stepping the flying speed instead).
    pub fn spectator_menu_active(&self) -> bool {
        self.shared.spectator.lock().menu_active()
    }

    /// `SpectatorGui.onHotbarSelected` (a digit key).
    pub fn spectator_hotbar_selected(&self, slot: i32) {
        let mut fx = super::spectator::MenuEffects::default();
        self.shared
            .spectator
            .lock()
            .on_hotbar_selected(slot, &mut fx);
        self.apply_menu_effects(fx);
    }

    /// `SpectatorGui.onMouseScrolled` (the wheel with the menu open).
    pub fn spectator_mouse_scrolled(&self, wheel: i32) {
        let mut fx = super::spectator::MenuEffects::default();
        self.shared
            .spectator
            .lock()
            .on_mouse_scrolled(wheel, &mut fx);
        self.apply_menu_effects(fx);
    }

    /// `SpectatorGui.onHotbarActionKeyPressed` (the `keySpectatorHotbar` key: middle click).
    pub fn spectator_hotbar_action_key(&self) {
        let mut fx = super::spectator::MenuEffects::default();
        self.shared.spectator.lock().on_hotbar_action_key(&mut fx);
        self.apply_menu_effects(fx);
    }

    /// What activating a menu item did: its packets queue in the outbox (the per-tick
    /// drain sends them, like vanilla's `handleKeybinds`-tick `Connection.send`).
    fn apply_menu_effects(&self, fx: super::spectator::MenuEffects) {
        self.shared.outbox.lock().extend(fx.packets);
    }
}

#[cfg(test)]
mod tests;
