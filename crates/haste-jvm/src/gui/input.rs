//! Input forwarding into the open screen and the vanilla `KeyMapping`s, the GUI tick, and the
//! per-frame render call that produces the [`GuiFrame`] (one JNI call).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use haste_core::gui::{GuiFrame, TextMetrics};
use jni::signature::JavaType;
use jni::sys::jvalue;

use super::frame::decode_frame;
use super::{GuiPipeline, NO_PANORAMA, VOID};
use crate::error::Result;
use crate::host::jstring_to_string;
use crate::proxies::{with_state, GameState};

impl GuiPipeline {
    /// Key press/release into `Screen.keyPressed/keyReleased(KeyEvent)`. 26.3 numbering, see
    /// [`crate::keys`]: `key` is `KeyEvent.key` (SDL3 scancode: what Escape/Enter/key mappings match),
    /// `scancode` is passed as `KeyEvent.keycode` (SDL keycode: what text editing matches, e.g.
    /// Backspace = 8; [`crate::keys::keycode`]); `mods` are SDL keymod bits. Returns whether the
    /// screen consumed it.
    pub fn key(&self, key: i32, scancode: i32, mods: i32, pressed: bool) -> bool {
        self.call_bool(
            "key",
            self.m.key,
            &[
                jvalue { i: key },
                jvalue { i: scancode },
                jvalue { i: mods },
                jvalue { z: pressed },
            ],
        )
    }

    /// A typed character into `Screen.charTyped(CharacterEvent)`.
    pub fn char_typed(&self, c: char, mods: i32) -> bool {
        self.call_bool(
            "char_typed",
            self.m.char_typed,
            &[jvalue { i: c as i32 }, jvalue { i: mods }],
        )
    }

    /// Mouse at (x, y) in GUI pixels: a move (`None`; drags while a button is held) or a button
    /// press/release (`Some((button, pressed))`, vanilla numbering 1 left / 2 middle / 3 right).
    pub fn mouse(&self, x: f64, y: f64, button: Option<(i32, bool)>) -> bool {
        let (b, p) = button.unwrap_or((-1, false));
        self.call_bool(
            "mouse",
            self.m.mouse,
            &[
                jvalue { d: x },
                jvalue { d: y },
                jvalue { i: b },
                jvalue { z: p },
            ],
        )
    }

    /// Scroll wheel at (x, y).
    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64) -> bool {
        self.call_bool(
            "scroll",
            self.m.scroll,
            &[
                jvalue { d: x },
                jvalue { d: y },
                jvalue { d: dx },
                jvalue { d: dy },
            ],
        )
    }

    /// Whether vanilla code asked to grab the mouse (`MouseHandler.grabMouse`, e.g. "Back to Game")
    /// and has not released it since. The client applies it to the window.
    pub fn mouse_grabbed(&self) -> bool {
        self.call_bool("mouse_grabbed", self.m.mouse_grabbed, &[])
    }

    /// Tells the Java side the client grabbed/released the mouse itself.
    pub fn set_mouse_grabbed(&self, grabbed: bool) {
        if let Err(e) = self.call(
            self.m.set_mouse_grabbed,
            VOID,
            &[jvalue { z: grabbed }],
            |_, _| Ok(()),
        ) {
            self.failed("set_mouse_grabbed", e);
        }
    }

    /// A game key event (no screen open) mirrored into the vanilla `KeyMapping`s, vanilla's
    /// `KeyboardHandler.keyPress`: press sets the down state and counts a click, release clears it.
    /// `scancode` is the SDL3 scancode the key mappings match.
    pub fn game_key(&self, scancode: i32, pressed: bool) {
        if let Err(e) = self.call(
            self.m.game_key,
            VOID,
            &[jvalue { i: scancode }, jvalue { z: pressed }],
            |_, _| Ok(()),
        ) {
            self.failed("game_key", e);
        }
    }

    /// A game mouse button (no screen open, vanilla numbering 1 left / 2 middle / 3 right)
    /// mirrored into the vanilla `KeyMapping`s, vanilla's `MouseHandler.onPress`.
    pub fn game_mouse(&self, button: i32, pressed: bool) {
        if let Err(e) = self.call(
            self.m.game_mouse,
            VOID,
            &[jvalue { i: button }, jvalue { z: pressed }],
            |_, _| Ok(()),
        ) {
            self.failed("game_mouse", e);
        }
    }

    /// The down state of the movement and action `KeyMapping`s as a bitmask (the bit values are
    /// `haste.gui.OptionBridge.B_*`, mirrored in [`crate::keys`]'s callers).
    pub fn game_input(&self) -> i32 {
        self.call_int("game_input", self.m.game_input, &[])
    }

    /// Consumes the clicks of the action `KeyMapping`s (hotbar, drop, swap offhand, jump) as a
    /// bitmask (the `haste.gui.OptionBridge.C_*` bits), one bit per click since the last call.
    pub fn consume_clicks(&self) -> i32 {
        self.call_int("consume_clicks", self.m.consume_clicks, &[])
    }

    /// One 20 Hz client tick (call from the game thread's tick, like vanilla): the HUD's timers
    /// (chat fade, titles, action bar), the chat queue, effect durations and the open screen.
    pub fn tick(&self) {
        if let Err(e) = self.call(self.m.tick, VOID, &[], |_, _| Ok(())) {
            self.failed("tick", e);
        }
    }

    /// One GUI frame: the vanilla HUD (`Hud.extractRenderState`, once [`GameState::in_world`] is
    /// set) and the open screen's `extractRenderStateWithTooltipAndSubtitles` on top, into the
    /// batch: ONE JNI call plus one drain of its two arrays. `width`/`height` are GUI-scaled.
    pub fn render(
        &self,
        width: u32,
        height: u32,
        scale: u32,
        mouse: (f64, f64),
        partial: f32,
    ) -> anyhow::Result<GuiFrame> {
        let args = [
            jvalue { i: width as i32 },
            jvalue { i: height as i32 },
            jvalue { i: scale as i32 },
            jvalue { d: mouse.0 },
            jvalue { d: mouse.1 },
            jvalue { f: partial },
        ];
        let f = self.call(self.m.render, JavaType::Array, &args, |env, v| {
            decode_frame(env, v.l()?)
        })?;
        self.panorama.store(
            f.panorama.map_or(NO_PANORAMA, f32::to_bits),
            Ordering::Relaxed,
        );
        Ok(GuiFrame {
            commands: f.commands,
            scale,
            width,
            height,
            cursor: f.cursor,
            blur_after: f.blur_after,
        })
    }

    /// The title panorama of the last [`GuiPipeline::render`]: its spin in degrees when the
    /// open screen extracted it (vanilla `Screen.extractPanorama` -> `Panorama.extractRenderState`,
    /// the frame's `PanoramaRenderState`), `None` when the frame had none. The renderer draws
    /// the cube map under that frame's commands.
    pub fn panorama(&self) -> Option<f32> {
        match self.panorama.load(Ordering::Relaxed) {
            NO_PANORAMA => None,
            bits => Some(f32::from_bits(bits)),
        }
    }

    /// Installs the renderer's text metrics; `Font.width` and friends call back into them.
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>) {
        crate::natives::set_metrics(m);
        if let Err(e) = self.call(self.m.metrics_changed, VOID, &[], |_, _| Ok(())) {
            self.failed("set_text_metrics", e);
        }
    }

    /// Where the last frame's blend modes came from: the pipeline and the name counts, then the
    /// multiply/invert blits' pipelines as `<pipeline location> <code> <count>` lines.
    pub fn blend_debug(&self) -> Result<String> {
        self.call(self.m.blend_debug, JavaType::Object, &[], |env, v| {
            jstring_to_string(env, v.l()?)
        })
    }

    /// Reads or updates the Rust-owned state the proxies see (inventory mirror, chat history, outbox).
    pub fn with_state<T>(&self, f: impl FnOnce(&mut GameState) -> T) -> T {
        with_state(f)
    }
}
