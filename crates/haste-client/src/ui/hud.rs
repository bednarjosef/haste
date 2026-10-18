//! The HUD half of [`Ui`] (jvm-host), vanilla's `Gui`/`Hud`: the state the HUD mirrors
//! (vitals, selected slot, the mount HUD, the F3 overlay) and the [`GuiFrame`] that draws it.
//! See the module documentation of [`crate::ui`].

use super::*;

impl Ui {
    /// Whether F1 hid the HUD (and with it the first-person hand).
    pub fn hud_hidden(&self) -> bool {
        self.shared.hud_hidden.load(Ordering::Relaxed)
    }

    /// Whether any hotbar slot (vanilla `Inventory` 0-8) holds an item. The check scripts wait
    /// on this after the runner's RCON `give`: on a busy shared server the inventory sync can
    /// lag seconds behind the command's answer.
    pub fn hotbar_has_items(&self) -> bool {
        self.state(|s| s.inventory.iter().take(9).any(|s| !s.is_empty()))
    }

    /// The GUI frame: the HUD (once joined) and the open screen, GUI-scaled size `width` x
    /// `height` at `scale`, the mouse at the last position given to [`Ui::mouse_moved`].
    pub fn frame(
        &mut self,
        width: u32,
        height: u32,
        scale: u32,
        mouse: (f64, f64),
        partial: f32,
    ) -> &GuiFrame {
        match self.gui().render(width, height, scale, mouse, partial) {
            Ok(f) => {
                self.trace_vitals(&f);
                self.frame = f;
            }
            Err(e) => {
                tracing::warn!("ui frame: {e:#}");
                self.frame.commands.clear();
            }
        }
        &self.frame
    }

    /// [`Ui::frame`] moved out instead of borrowed: for callers that keep the frame for the
    /// rest of the render (`App.gui`, the screenshot and script loops), so the whole
    /// command list is moved rather than deep-cloned every frame.
    pub fn take_frame(
        &mut self,
        width: u32,
        height: u32,
        scale: u32,
        mouse: (f64, f64),
        partial: f32,
    ) -> GuiFrame {
        self.frame(width, height, scale, mouse, partial);
        std::mem::take(&mut self.frame)
    }

    /// What the HUD drew of the player's vitals, at debug level (the frame path stays clean
    /// when debug logging is off): the game mode the HUD renders for and the heart, food and
    /// attack indicator sprite counts, so a "no health, no hunger" report can be traced to
    /// the side that has them wrong — the state, or the Java HUD that reads it
    /// (`Hud.extractHotbarAndDecorations` draws them only while `gameMode.canHurtPlayer()`).
    fn trace_vitals(&self, f: &GuiFrame) {
        if !tracing::enabled!(tracing::Level::DEBUG) {
            return;
        }
        let (hearts, food, attack) =
            f.commands
                .iter()
                .fold((0usize, 0usize, 0usize), |(h, fd, a), c| {
                    if let GuiCommand::Sprite { sprite, .. }
                    | GuiCommand::SpriteRegion { sprite, .. } = c
                    {
                        let path = sprite.path();
                        (
                            h + usize::from(path.starts_with("hud/heart")),
                            fd + usize::from(path.starts_with("hud/food")),
                            a + usize::from(path.starts_with("hud/crosshair_attack_indicator")),
                        )
                    } else {
                        (h, fd, a)
                    }
                });
        let mode = self.state(|s| s.game_mode);
        let charge = self.state(|s| s.attack_strength);
        tracing::debug!(
            "hud frame: {mode:?} hearts {hearts} food {food} attack {attack} charge {charge:?}"
        );
    }

    /// Whether the F3 debug overlay is on (its strings are only built then).
    pub fn debug_shown(&self) -> bool {
        self.gui().debug_shown()
    }

    /// The selected hotbar slot (0-8): the HUD's and the server's, kept here.
    pub fn selected_slot(&self) -> i32 {
        self.state(|s| s.selected_slot)
    }

    /// The client selected a hotbar slot itself (1-9, the wheel): the HUD shows it from here.
    /// The client's own `set_carried_item` tells the server, so `sent_slot` is set with it (the
    /// UI must not send it a second time).
    pub fn set_selected_slot(&self, slot: i32) {
        self.state(|s| s.selected_slot = slot);
        self.shared.sent_slot.store(slot, Ordering::Relaxed);
    }

    /// The player started riding (vanilla
    /// `ClientPacketListener.handleSetEntityPassengersPacket`): the "Press <sneak key> to
    /// dismount" overlay message.
    pub fn show_mount_message(&self) {
        self.gui().show_mount_message();
    }

    /// The client left the world (pause screen "Disconnect", death screen "Title Screen"):
    /// the HUD stops drawing (`Hud.extractRenderState` runs only while
    /// [`GameState::in_world`] is set, like vanilla's `Gui` once its level is null).
    pub fn left_world(&self) {
        self.state(|s| s.in_world = false);
    }

    /// The F3 overlay's numbers (FPS, position, rotation, chunk and entity counts), every frame.
    pub fn set_debug(&self, d: haste_jvm::DebugInfo) {
        self.state(|s| s.debug = d);
    }

    /// The player's eyes are in water (air bubbles and the water overlay), from physics.
    pub fn set_eye_in_water(&self, v: bool) {
        self.state(|s| s.vitals.eye_in_water = v);
    }

    /// The player's damage cooldown (vanilla `LivingEntity.damageCooldownTime`, set by a
    /// hit or a heal and ticked down by the session): the HUD's heart blink gates on it
    /// (`Hud.extractPlayerHealth` blinks while it is positive).
    pub fn set_damage_cooldown(&self, ticks: i32) {
        self.state(|s| s.vitals.damage_cooldown_ticks = ticks);
    }

    /// The mount HUD's values for this tick (vanilla `Hud`: the vehicle's hearts, the charged
    /// jump bar), `None` on foot.
    pub fn set_mount_hud(&self, mount: Option<haste_jvm::MountHud>) {
        self.state(|s| s.mount = mount);
    }

    /// The attack indicator's charge for this tick (vanilla `Player.getAttackStrengthScale`'s
    /// ticker and the held item's delay), from the first-person hand's tick.
    pub fn set_attack_strength(&self, charge: haste_jvm::AttackStrengthHud) {
        self.state(|s| s.attack_strength = charge);
    }

    /// Whether the crosshair picks a living, alive entity in reach this tick (vanilla
    /// `Minecraft.crosshairPickEntity`): the attack indicator's full icon.
    pub fn set_crosshair_pick(&self, pick: bool) {
        self.state(|s| s.crosshair_pick_living = pick);
    }
}
