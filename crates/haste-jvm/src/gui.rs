//! The batched GUI pipeline: real vanilla screens in, one [`GuiFrame`] per render out.
//!
//! Every method is one JNI call into `haste.gui.Bridge`. `render` returns the whole frame from that
//! one call as `{int[] commands, String[] strings}` (layout in `java/haste/rt/GuiBatch.java`),
//! which [`decode_frame`] turns into [`GuiCommand`]s.
//!
//! Coordinates: GUI-scaled pixels (vanilla "GUI scale"), for both `render`'s mouse position and the
//! input methods. Key numbering: vanilla 26.3's (`InputConstants`, SDL3 scancodes for `key`,
//! SDL keymod bits for `mods`), see [`crate::keys`]. Mouse buttons: 1 left, 2 middle, 3 right.
use std::sync::Arc;

use haste_core::gui::{GuiCommand, GuiFrame, TextMetrics};
use haste_core::{Identifier, RegistryIndex};
use jni::objects::{JIntArray, JObject, JObjectArray, JString};
use jni::refs::Global;
use jni::signature::{JavaType, Primitive};
use jni::sys::jvalue;
use jni::{jni_sig, jni_str, Env, JavaVM};
use parking_lot::Mutex;

use crate::error::{JvmError, Result};
use crate::host::{caught, jstring_to_string};
use crate::proxies::{with_state, GameState};

/// The vanilla screens Haste opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKind {
    /// `net.minecraft.client.gui.screens.ChatScreen`.
    Chat,
    /// `net.minecraft.client.gui.screens.PauseScreen`.
    Pause,
    /// `net.minecraft.client.gui.screens.inventory.InventoryScreen`.
    Inventory,
}

impl ScreenKind {
    fn id(self) -> i32 {
        match self {
            ScreenKind::Chat => 0,
            ScreenKind::Pause => 1,
            ScreenKind::Inventory => 2,
        }
    }
}

#[derive(Clone, Copy)]
struct Methods {
    open: jni::objects::JMethodID,
    close: jni::objects::JMethodID,
    is_open: jni::objects::JMethodID,
    screen_name: jni::objects::JMethodID,
    render: jni::objects::JMethodID,
    tick: jni::objects::JMethodID,
    key: jni::objects::JMethodID,
    char_typed: jni::objects::JMethodID,
    mouse: jni::objects::JMethodID,
    scroll: jni::objects::JMethodID,
    metrics_changed: jni::objects::JMethodID,
    mouse_grabbed: jni::objects::JMethodID,
    set_mouse_grabbed: jni::objects::JMethodID,
}

/// Drives real vanilla screens through the bridge. All calls are serialised (the Java side is
/// single-threaded like vanilla's render thread).
pub struct GuiPipeline {
    bridge: Global<JObject<'static>>,
    m: Methods,
    lock: Mutex<()>,
    last_error: Mutex<Option<String>>,
}

const BOOL: JavaType = JavaType::Primitive(Primitive::Boolean);
const VOID: JavaType = JavaType::Primitive(Primitive::Void);

impl GuiPipeline {
    pub(crate) fn new(env: &mut Env, bridge: Global<JObject<'static>>) -> Result<GuiPipeline> {
        let cls = env.get_object_class(&*bridge)?;
        let m = Methods {
            open: env.get_method_id(&cls, jni_str!("open"), jni_sig!("(I)V"))?,
            close: env.get_method_id(&cls, jni_str!("close"), jni_sig!("()V"))?,
            is_open: env.get_method_id(&cls, jni_str!("isOpen"), jni_sig!("()Z"))?,
            screen_name: env.get_method_id(
                &cls,
                jni_str!("screenName"),
                jni_sig!("()Ljava/lang/String;"),
            )?,
            render: env.get_method_id(
                &cls,
                jni_str!("render"),
                jni_sig!("(IIIDDF)[Ljava/lang/Object;"),
            )?,
            tick: env.get_method_id(&cls, jni_str!("tick"), jni_sig!("()V"))?,
            key: env.get_method_id(&cls, jni_str!("key"), jni_sig!("(IIIZ)Z"))?,
            char_typed: env.get_method_id(&cls, jni_str!("charTyped"), jni_sig!("(II)Z"))?,
            mouse: env.get_method_id(&cls, jni_str!("mouse"), jni_sig!("(DDIZ)Z"))?,
            scroll: env.get_method_id(&cls, jni_str!("scroll"), jni_sig!("(DDDD)Z"))?,
            metrics_changed: env.get_method_id(
                &cls,
                jni_str!("metricsChanged"),
                jni_sig!("()V"),
            )?,
            mouse_grabbed: env.get_method_id(&cls, jni_str!("mouseGrabbed"), jni_sig!("()Z"))?,
            set_mouse_grabbed: env.get_method_id(
                &cls,
                jni_str!("setMouseGrabbed"),
                jni_sig!("(Z)V"),
            )?,
        };
        Ok(GuiPipeline {
            bridge,
            m,
            lock: Mutex::new(()),
            last_error: Mutex::new(None),
        })
    }

    /// One call on the bridge with the given return type; Java exceptions become errors (cleared).
    fn call<T>(
        &self,
        method: jni::objects::JMethodID,
        ret: JavaType,
        args: &[jvalue],
        f: impl FnOnce(&mut Env, jni::objects::JValueOwned) -> Result<T>,
    ) -> Result<T> {
        let _g = self.lock.lock();
        let vm = JavaVM::singleton()?;
        vm.attach_current_thread(|env| -> Result<T> {
            // Safety: method ids were resolved on the bridge's class with matching signatures/args.
            let v = unsafe { env.call_method_unchecked(&*self.bridge, method, ret, args) }
                .map_err(|e| caught(env, e))?;
            f(env, v)
        })
    }

    fn call_bool(&self, what: &str, method: jni::objects::JMethodID, args: &[jvalue]) -> bool {
        match self.call(method, BOOL, args, |_, v| Ok(v.z()?)) {
            Ok(b) => b,
            Err(e) => {
                self.failed(what, e);
                false
            }
        }
    }

    fn failed(&self, what: &str, e: JvmError) {
        tracing::warn!("GuiPipeline::{what}: {e}");
        *self.last_error.lock() = Some(format!("{what}: {e}"));
    }

    /// The last error swallowed by a method that cannot return one (input, close, tick), if any.
    pub fn take_last_error(&self) -> Option<String> {
        self.last_error.lock().take()
    }

    /// Opens a real vanilla screen (replacing the open one), like `Gui.setScreen(new ...)`.
    pub fn open_screen(&self, screen: ScreenKind) -> anyhow::Result<()> {
        self.call(self.m.open, VOID, &[jvalue { i: screen.id() }], |_, _| {
            Ok(())
        })?;
        Ok(())
    }

    /// Closes the open screen (`Gui.setScreen(null)`).
    pub fn close_screen(&self) {
        if let Err(e) = self.call(self.m.close, VOID, &[], |_, _| Ok(())) {
            self.failed("close_screen", e);
        }
    }

    /// Whether a screen is open (screens close themselves, e.g. Escape or "Back to Game").
    pub fn is_open(&self) -> bool {
        self.call_bool("is_open", self.m.is_open, &[])
    }

    /// Java class name of the open screen, if any.
    pub fn screen_class(&self) -> Option<String> {
        self.call(self.m.screen_name, JavaType::Object, &[], |env, v| {
            jstring_to_string(env, v.l()?)
        })
        .ok()
        .filter(|s| !s.is_empty())
    }

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

    /// Ticks the open screen (call at 20 Hz from the game thread's tick, like vanilla).
    pub fn tick(&self) {
        if let Err(e) = self.call(self.m.tick, VOID, &[], |_, _| Ok(())) {
            self.failed("tick", e);
        }
    }

    /// Runs the screen's `extractRenderStateWithTooltipAndSubtitles` into the batch and returns the
    /// frame: ONE JNI call plus one drain of its two arrays. `width`/`height` are GUI-scaled.
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
        let commands = self.call(self.m.render, JavaType::Array, &args, |env, v| {
            decode_frame(env, v.l()?)
        })?;
        Ok(GuiFrame {
            commands,
            scale,
            width,
            height,
            cursor: None,
        })
    }

    /// Installs the renderer's text metrics; `Font.width` and friends call back into them.
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>) {
        crate::natives::set_metrics(m);
        if let Err(e) = self.call(self.m.metrics_changed, VOID, &[], |_, _| Ok(())) {
            self.failed("set_text_metrics", e);
        }
    }

    /// Reads or updates the Rust-owned state the proxies see (inventory, chat history, outbox).
    pub fn with_state<T>(&self, f: impl FnOnce(&mut GameState) -> T) -> T {
        with_state(f)
    }
}

// Opcodes; must match java/haste/rt/GuiBatch.java.
const FILL: i32 = 1;
const GRADIENT: i32 = 2;
const TEXT: i32 = 3;
const TEXTURE: i32 = 4;
const SPRITE: i32 = 5;
const ITEM: i32 = 6;
const SCISSOR_PUSH: i32 = 7;
const SCISSOR_POP: i32 = 8;
const TRANSLATE_PUSH: i32 = 9;
const TRANSLATE_POP: i32 = 10;

fn decode_frame(env: &mut Env, frame: JObject) -> Result<Vec<GuiCommand>> {
    // Safety: Bridge.render returns Object[]{int[], String[]}.
    let frame = unsafe { JObjectArray::<JObject>::from_raw(env, frame.into_raw()) };
    let ints = frame.get_element(env, 0)?;
    let ints = unsafe { JIntArray::from_raw(env, ints.into_raw()) };
    let n = ints.len(env)?;
    let mut buf = vec![0i32; n];
    ints.get_region(env, 0, &mut buf)?;
    let strs = frame.get_element(env, 1)?;
    let strs = unsafe { JObjectArray::<JString>::from_raw(env, strs.into_raw()) };
    let ns = strs.len(env)?;
    let mut strings = Vec::with_capacity(ns);
    for i in 0..ns {
        let s = strs.get_element(env, i)?;
        strings.push(s.try_to_string(env)?);
        env.delete_local_ref(s);
    }
    decode(&buf, &strings)
}

/// Decodes a frame's int records and string table into commands.
pub(crate) fn decode(b: &[i32], strings: &[String]) -> Result<Vec<GuiCommand>> {
    let bad = |what: &str| JvmError::Bridge(format!("malformed frame: {what}"));
    let s = |i: i32| {
        strings
            .get(i as usize)
            .cloned()
            .ok_or_else(|| bad("string index"))
    };
    let ident = |i: i32| -> Result<Identifier> {
        let t = strings.get(i as usize).ok_or_else(|| bad("string index"))?;
        Identifier::parse(t).map_err(|e| JvmError::Bridge(e.to_string()))
    };
    let f = f32::from_bits;
    let mut out = Vec::with_capacity(b.len() / 5);
    let mut i = 0;
    while i < b.len() {
        let need = |n: usize| {
            if i + n < b.len() + 1 {
                Ok(())
            } else {
                Err(bad("truncated record"))
            }
        };
        match b[i] {
            FILL => {
                need(6)?;
                out.push(GuiCommand::Fill {
                    x1: b[i + 1],
                    y1: b[i + 2],
                    x2: b[i + 3],
                    y2: b[i + 4],
                    color: b[i + 5] as u32,
                });
                i += 6;
            }
            GRADIENT => {
                need(7)?;
                out.push(GuiCommand::FillGradient {
                    x1: b[i + 1],
                    y1: b[i + 2],
                    x2: b[i + 3],
                    y2: b[i + 4],
                    top: b[i + 5] as u32,
                    bottom: b[i + 6] as u32,
                });
                i += 7;
            }
            TEXT => {
                need(6)?;
                out.push(GuiCommand::Text {
                    x: b[i + 1],
                    y: b[i + 2],
                    color: b[i + 3] as u32,
                    shadow: b[i + 4] != 0,
                    text: s(b[i + 5])?,
                });
                i += 6;
            }
            TEXTURE => {
                need(13)?;
                out.push(GuiCommand::Texture {
                    texture: ident(b[i + 1])?,
                    x: b[i + 2],
                    y: b[i + 3],
                    w: b[i + 4],
                    h: b[i + 5],
                    u: f(b[i + 6] as u32),
                    v: f(b[i + 7] as u32),
                    uw: f(b[i + 8] as u32),
                    vh: f(b[i + 9] as u32),
                    tex_w: b[i + 10] as u32,
                    tex_h: b[i + 11] as u32,
                    tint: b[i + 12] as u32,
                });
                i += 13;
            }
            SPRITE => {
                need(7)?;
                out.push(GuiCommand::Sprite {
                    sprite: ident(b[i + 1])?,
                    x: b[i + 2],
                    y: b[i + 3],
                    w: b[i + 4],
                    h: b[i + 5],
                    tint: b[i + 6] as u32,
                });
                i += 7;
            }
            ITEM => {
                need(5)?;
                out.push(GuiCommand::Item {
                    item: RegistryIndex(b[i + 1] as u32),
                    x: b[i + 2],
                    y: b[i + 3],
                    count: b[i + 4],
                });
                i += 5;
            }
            SCISSOR_PUSH => {
                need(5)?;
                out.push(GuiCommand::ScissorPush {
                    x1: b[i + 1],
                    y1: b[i + 2],
                    x2: b[i + 3],
                    y2: b[i + 4],
                });
                i += 5;
            }
            SCISSOR_POP => {
                out.push(GuiCommand::ScissorPop);
                i += 1;
            }
            TRANSLATE_PUSH => {
                need(4)?;
                out.push(GuiCommand::TranslatePush {
                    dx: f(b[i + 1] as u32),
                    dy: f(b[i + 2] as u32),
                    dz: f(b[i + 3] as u32),
                });
                i += 4;
            }
            TRANSLATE_POP => {
                out.push(GuiCommand::TranslatePop);
                i += 1;
            }
            op => return Err(bad(&format!("opcode {op}"))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_every_opcode() {
        let strings = vec![
            "hi".to_owned(),
            "minecraft:widget/button".to_owned(),
            "minecraft:textures/gui/x.png".to_owned(),
        ];
        let fb = |x: f32| x.to_bits() as i32;
        let b = vec![
            FILL,
            0,
            0,
            10,
            10,
            -1,
            GRADIENT,
            0,
            0,
            5,
            5,
            1,
            2,
            TEXT,
            3,
            4,
            0xFFFFFF,
            1,
            0,
            TEXTURE,
            2,
            1,
            2,
            3,
            4,
            fb(0.5),
            fb(1.0),
            fb(2.0),
            fb(3.0),
            256,
            128,
            -1,
            SPRITE,
            1,
            10,
            20,
            200,
            20,
            -1,
            ITEM,
            7,
            8,
            9,
            64,
            SCISSOR_PUSH,
            1,
            2,
            3,
            4,
            SCISSOR_POP,
            TRANSLATE_PUSH,
            fb(1.5),
            fb(-2.0),
            fb(0.0),
            TRANSLATE_POP,
        ];
        let c = decode(&b, &strings).unwrap();
        assert_eq!(c.len(), 10);
        assert_eq!(
            c[0],
            GuiCommand::Fill {
                x1: 0,
                y1: 0,
                x2: 10,
                y2: 10,
                color: 0xFFFF_FFFF
            }
        );
        assert_eq!(
            c[2],
            GuiCommand::Text {
                x: 3,
                y: 4,
                text: "hi".into(),
                color: 0xFFFFFF,
                shadow: true
            }
        );
        assert!(
            matches!(&c[3], GuiCommand::Texture { texture, u, tex_w: 256, .. } if texture.path() == "textures/gui/x.png" && *u == 0.5)
        );
        assert!(
            matches!(&c[4], GuiCommand::Sprite { sprite, w: 200, .. } if sprite.path() == "widget/button")
        );
        assert_eq!(
            c[5],
            GuiCommand::Item {
                item: RegistryIndex(7),
                x: 8,
                y: 9,
                count: 64
            }
        );
        assert_eq!(
            c[8],
            GuiCommand::TranslatePush {
                dx: 1.5,
                dy: -2.0,
                dz: 0.0
            }
        );
        assert_eq!(c[9], GuiCommand::TranslatePop);
    }

    #[test]
    fn rejects_malformed_frames() {
        assert!(decode(&[FILL, 1, 2], &[]).is_err());
        assert!(decode(&[TEXT, 0, 0, 0, 0, 5], &[]).is_err());
        assert!(decode(&[99], &[]).is_err());
    }
}
