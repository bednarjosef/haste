//! The HUD half of [`Ui`] (jvm-host), vanilla's `Gui`/`Hud`: the state the HUD mirrors
//! (vitals, selected slot, the mount HUD, the F3 overlay) and the [`GuiFrame`] that draws it.
//! See the module documentation of [`crate::ui`].

use std::time::{Duration, Instant};

use super::*;

#[cfg(test)]
mod tests;

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

/// A freshly joined HUD warming up before it goes live: the JVM's first GUI frames load the
/// HUD render path's classes, and the first seconds of frames cross the JVM's code-cache and
/// metadata GC thresholds with full GC pauses of 100-200 ms (`-Xlog:gc`; measured ~5 s into
/// the JVM's life, right after the HUD goes live). Rendering those frames here — on the game
/// thread, budgeted per tick, before the UI is published — keeps every pause off the render
/// thread, which would otherwise stall the moment it starts calling into the JVM. Vanilla
/// needs no warm-up: its whole client is the JVM, warm before the loading screen ends.
pub struct HudWarmup {
    ui: Ui,
    frames: u32,
    /// The frame count when the last slow frame (a class load or a full GC) happened; warm
    /// once enough frames have passed without another. The young GCs of the growing heap
    /// (1-3 ms) do not count: they are not the hitches being waited out.
    last_slow: u32,
    /// The slowest warm-up frame so far (the class load or GC we are waiting out).
    slowest_ms: f32,
    started: Instant,
}

/// A frame at least this slow is a pause being waited out (the hitch threshold).
const SLOW_MS: f32 = 20.0;
/// Warm-up continues until at least this many frames (the boot GCs land a few hundred
/// frames into GUI rendering, not on frame one).
const MIN_FRAMES: u32 = 600;
/// ...and until this many frames have passed without another slow one.
const SETTLE_FRAMES: u32 = 300;
/// Never warm longer than this many frames (a JVM whose pauses never stop still goes live,
/// slow rather than never).
const MAX_FRAMES: u32 = 4000;

impl HudWarmup {
    /// Starts warming `ui`, already joined: its frames render the real HUD state.
    pub fn new(ui: Ui) -> Self {
        Self {
            ui,
            frames: 0,
            last_slow: 0,
            slowest_ms: 0.0,
            started: Instant::now(),
        }
    }

    /// Renders warm-up frames for up to `budget` of wall time (one game-thread tick's
    /// share; a single frame may overrun it — the class-loading first frame). `true` when
    /// the HUD is warm: [`HudWarmup::into_ui`] hands it over to go live.
    pub fn step(&mut self, budget: Duration) -> bool {
        let start = Instant::now();
        while self.frames < MAX_FRAMES && (self.frames == 0 || start.elapsed() < budget) {
            // Vanilla's default window (854x480) at GUI scale 2: the size only shifts the
            // layout, the classes and allocation are the same at any size.
            let t = Instant::now();
            self.ui.frame(854, 480, 2, (0.0, 0.0), 0.5);
            let ms = t.elapsed().as_secs_f32() * 1e3;
            self.slowest_ms = self.slowest_ms.max(ms);
            self.frames += 1;
            if ms >= SLOW_MS {
                self.last_slow = self.frames;
            }
        }
        let warm = self.frames >= MAX_FRAMES
            || (self.frames >= MIN_FRAMES && self.frames - self.last_slow >= SETTLE_FRAMES);
        if warm {
            tracing::info!(
                "HUD warm after {} frames in {:.2} s (slowest frame {:.0} ms)",
                self.frames,
                self.started.elapsed().as_secs_f32(),
                self.slowest_ms
            );
        }
        warm
    }

    /// The warmed UI, ready to publish.
    pub fn into_ui(self) -> Ui {
        self.ui
    }
}
