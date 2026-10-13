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

use haste_core::gui::{CursorShape, GuiBlend, GuiCommand, GuiFrame, TextMetrics, TextStyle};
use haste_core::{EntityId, Identifier, RegistryIndex};
use jni::objects::{JIntArray, JObject, JObjectArray, JString};
use jni::refs::Global;
use jni::signature::{JavaType, Primitive};
use jni::sys::jvalue;
use jni::{jni_sig, jni_str, Env, JavaVM};
use parking_lot::Mutex;

use crate::error::{JvmError, Result};
use crate::host::{caught, jstring_to_string};
use crate::proxies::{encode_stack, with_state, GameState};
use haste_core::entity::ItemStackData;

/// The vanilla screens Haste opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKind {
    /// `net.minecraft.client.gui.screens.ChatScreen`.
    Chat,
    /// `net.minecraft.client.gui.screens.PauseScreen`.
    Pause,
    /// `net.minecraft.client.gui.screens.inventory.InventoryScreen`.
    Inventory,
    /// `ChatScreen` opened with "/" typed (the command key).
    Command,
    /// `net.minecraft.client.gui.screens.TitleScreen` (the game's main menu).
    Title,
    /// `net.minecraft.client.gui.screens.options.OptionsScreen` (vanilla opens it from the title
    /// screen; "Done" returns to the screen that opened it).
    Options,
    /// `net.minecraft.client.gui.screens.multiplayer.JoinMultiplayerScreen`, whose parent is a
    /// `TitleScreen`.
    Multiplayer,
}

impl ScreenKind {
    fn id(self) -> i32 {
        match self {
            ScreenKind::Chat => 0,
            ScreenKind::Pause => 1,
            ScreenKind::Inventory => 2,
            ScreenKind::Command => 3,
            ScreenKind::Title => 4,
            ScreenKind::Options => 5,
            ScreenKind::Multiplayer => 6,
        }
    }
}

#[derive(Clone, Copy)]
struct Methods {
    open: jni::objects::JMethodID,
    close: jni::objects::JMethodID,
    reset_for_test: jni::objects::JMethodID,
    is_open: jni::objects::JMethodID,
    screen_name: jni::objects::JMethodID,
    render: jni::objects::JMethodID,
    tick: jni::objects::JMethodID,
    key: jni::objects::JMethodID,
    char_typed: jni::objects::JMethodID,
    mouse: jni::objects::JMethodID,
    scroll: jni::objects::JMethodID,
    metrics_changed: jni::objects::JMethodID,
    blend_debug: jni::objects::JMethodID,
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
            reset_for_test: env.get_method_id(&cls, jni_str!("resetForTest"), jni_sig!("()V"))?,
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
            blend_debug: env.get_method_id(
                &cls,
                jni_str!("blendDebug"),
                jni_sig!("()Ljava/lang/String;"),
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

    /// Drops the GUI state earlier tests leave behind in the process-wide JVM: the chat log and its
    /// scroll, the held mouse button and position, the open screen and the mouse grab. Tests call
    /// this at their start (the JVM is one per process, so everything vanilla remembers about the
    /// GUI survives a test).
    pub fn reset_for_test(&self) {
        if let Err(e) = self.call(self.m.reset_for_test, VOID, &[], |_, _| Ok(())) {
            self.failed("reset_for_test", e);
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
        let (commands, cursor) = self.call(self.m.render, JavaType::Array, &args, |env, v| {
            decode_frame(env, v.l()?)
        })?;
        Ok(GuiFrame {
            commands,
            scale,
            width,
            height,
            cursor,
        })
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

    /// Runs `f` with the JNI environment and the bridge object, serialised like every other call;
    /// a Java exception is returned as an error (cleared).
    fn with_bridge<T>(
        &self,
        f: impl FnOnce(&mut Env, &JObject) -> std::result::Result<T, jni::errors::Error>,
    ) -> Result<T> {
        let _g = self.lock.lock();
        let vm = JavaVM::singleton()?;
        vm.attach_current_thread(|env| -> Result<T> {
            let bridge: &JObject = &self.bridge;
            f(env, bridge).map_err(|e| caught(env, e))
        })
    }

    /// Calls a `void` bridge method taking one `byte[]` (and optionally a boolean).
    fn call_bytes(&self, name: &'static str, bytes: &[u8], flag: Option<bool>) -> Result<()> {
        self.with_bridge(|env, bridge| {
            let arr = env.byte_array_from_slice(bytes)?;
            let name = jni::strings::JNIString::new(name);
            match flag {
                None => env.call_method(bridge, &name, jni_sig!("([B)V"), &[(&arr).into()])?,
                Some(z) => env.call_method(
                    bridge,
                    &name,
                    jni_sig!("([BZ)V"),
                    &[(&arr).into(), jni::objects::JValue::Bool(z)],
                )?,
            };
            env.delete_local_ref(arr);
            Ok(())
        })
    }

    /// One `registry_data` packet of the configuration phase (body without the packet id, exactly
    /// as received): collected and built into vanilla registries by [`Self::finish_configuration`].
    pub fn registry_data(&self, body: &[u8]) -> anyhow::Result<()> {
        Ok(self.call_bytes("registryData", body, None)?)
    }

    /// An `update_tags` packet body (without id). `play`: received in the play phase, where the tags
    /// are applied to the current registries at once; in configuration they wait for
    /// [`Self::finish_configuration`].
    pub fn tags(&self, body: &[u8], play: bool) -> anyhow::Result<()> {
        Ok(self.call_bytes("tags", body, Some(play))?)
    }

    /// End of the configuration phase: vanilla's `RegistryDataCollector` builds the synchronised
    /// registries (entries decoded with their codecs, tags bound, item components initialised).
    pub fn finish_configuration(&self) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("finishConfiguration"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Builds the synchronised registries from the vanilla data pack inside the player's client jar
    /// instead of a server's `registry_data` (no tags). For tests and anything that needs registry
    /// data without a connection; a connection's registries replace these.
    pub fn load_jar_registries(&self) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            env.call_method(bridge, jni_str!("loadJarRegistries"), jni_sig!("()V"), &[])?;
            Ok(())
        })?;
        Ok(())
    }

    /// A clientbound play packet whose state lives in tier 3 (chat, boss bars, scoreboard, titles,
    /// tab list, commands, containers, ...): VarInt packet id + body, the exact wire bytes. Java
    /// decodes it with vanilla's codec and handles it.
    pub fn packet(&self, id_and_body: &[u8]) -> anyhow::Result<()> {
        Ok(self.call_bytes("packet", id_and_body, None)?)
    }

    /// Sets a player inventory slot (vanilla `Inventory` numbering) in Java; the mirror in
    /// [`GameState::inventory`] follows.
    pub fn set_inventory_slot(&self, slot: i32, stack: &ItemStackData) -> anyhow::Result<()> {
        let b = encode_stack(stack);
        self.with_bridge(|env, bridge| {
            let arr = env.byte_array_from_slice(&b)?;
            env.call_method(
                bridge,
                jni_str!("setInventorySlot"),
                jni_sig!("(I[B)V"),
                &[jni::objects::JValue::Int(slot), (&arr).into()],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// The local player as the server introduced it: entity id (from `login`; effect, attribute
    /// and entity-data packets for this id update the player proxy), name and profile id.
    pub fn set_player(&self, entity_id: i32, name: &str, uuid: u128) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            let n = JString::from_str(env, name)?;
            env.call_method(
                bridge,
                jni_str!("setPlayer"),
                jni_sig!("(ILjava/lang/String;JJ)V"),
                &[
                    jni::objects::JValue::Int(entity_id),
                    (&n).into(),
                    jni::objects::JValue::Long((uuid >> 64) as i64),
                    jni::objects::JValue::Long(uuid as u64 as i64),
                ],
            )?;
            Ok(())
        })?;
        with_state(|s| s.player_name = name.to_owned());
        Ok(())
    }

    /// The joined world's dimension type: its id in the synchronised `minecraft:dimension_type`
    /// registry (from `login`/`respawn`).
    pub fn set_dimension_type(&self, id: i32) {
        if let Err(e) = self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("setDimensionType"),
                jni_sig!("(I)V"),
                &[jni::objects::JValue::Int(id)],
            )?;
            Ok(())
        }) {
            self.failed("set_dimension_type", e);
        }
    }

    /// The server's enabled feature flags (configuration phase `update_enabled_features`).
    pub fn set_features(&self, ids: &[String]) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            let arr = JObjectArray::<JString>::new(env, ids.len(), JString::null())?;
            for (i, id) in ids.iter().enumerate() {
                let s = JString::from_str(env, id)?;
                arr.set_element(env, i, &s)?;
            }
            env.call_method(
                bridge,
                jni_str!("setFeatures"),
                jni_sig!("([Ljava/lang/String;)V"),
                &[(&arr).into()],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Tab held or released: vanilla's player list overlay is shown while it is down.
    pub fn set_tab_list_visible(&self, visible: bool) {
        if let Err(e) = self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("setTabListVisible"),
                jni_sig!("(Z)V"),
                &[jni::objects::JValue::Bool(visible)],
            )?;
            Ok(())
        }) {
            self.failed("set_tab_list_visible", e);
        }
    }

    /// The connection ended: vanilla's disconnect screen with the reason.
    pub fn disconnected(&self, reason: &haste_core::Component) -> anyhow::Result<()> {
        self.disconnected_json(&serde_json::to_string(reason)?)
    }

    /// [`Self::disconnected`] with the reason as a text component in its JSON form, e.g.
    /// `{"text":"Kicked"}` or `{"translate":"disconnect.timeout"}`.
    pub fn disconnected_json(&self, reason_json: &str) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            let r = JString::from_str(env, reason_json)?;
            env.call_method(
                bridge,
                jni_str!("disconnected"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[(&r).into()],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// F1: hides or shows the HUD (vanilla `Hud.toggle`).
    pub fn toggle_hud(&self) {
        if let Err(e) = self.call_void("toggleHud") {
            self.failed("toggle_hud", e);
        }
    }

    /// F3: shows or hides the debug overlay (Haste's stub: FPS, position, chunk and entity counts
    /// from [`GameState::debug`]).
    pub fn toggle_debug(&self) {
        if let Err(e) = self.call_void("toggleDebug") {
            self.failed("toggle_debug", e);
        }
    }

    fn call_void(&self, name: &'static str) -> Result<()> {
        self.with_bridge(|env, bridge| {
            let name = jni::strings::JNIString::new(name);
            env.call_method(bridge, &name, jni_sig!("()V"), &[])?;
            Ok(())
        })
    }

    /// The chat log as plain text, oldest first (vanilla's `ChatComponent`: what the chat shows,
    /// after decoration such as "<name> message").
    pub fn chat_history(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .call_string("chatHistory")?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// The server's commands the local player may use (the root of the client-side command tree
    /// built from the `commands` packet).
    pub fn command_names(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .call_string("commandNames")?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    #[doc(hidden)]
    pub fn debug_suggestions(&self) -> anyhow::Result<String> {
        Ok(self.call_string("debugSuggestions")?)
    }

    /// The value of a vanilla option by its accessor name (`"autoSuggestions"`, `"guiScale"`, ...)
    /// as Java prints it.
    pub fn option(&self, name: &str) -> anyhow::Result<String> {
        let s = self.with_bridge(|env, bridge| {
            let l = JString::from_str(env, name)?;
            let v = env
                .call_method(
                    bridge,
                    jni_str!("option"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[(&l).into()],
                )?
                .l()?;
            // Safety: option returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            v.try_to_string(env)
        })?;
        Ok(s)
    }

    /// Brigadier's completions of a command line (without "/") from the server's command tree, as
    /// the chat screen computes them (server-side suggestions are not waited for).
    pub fn completions(&self, line: &str) -> anyhow::Result<Vec<String>> {
        let s = self.with_bridge(|env, bridge| {
            let l = JString::from_str(env, line)?;
            let v = env
                .call_method(
                    bridge,
                    jni_str!("completions"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[(&l).into()],
                )?
                .l()?;
            // Safety: completions returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            v.try_to_string(env)
        })?;
        Ok(s.lines().map(str::to_owned).collect())
    }

    /// Every registry Java sees with its size (built-in from vanilla's bootstrap, plus the
    /// synchronised ones after [`Self::finish_configuration`]).
    pub fn registry_sizes(&self) -> anyhow::Result<std::collections::BTreeMap<String, usize>> {
        let s = self.call_string("registrySizes")?;
        Ok(s.lines()
            .filter_map(|l| l.split_once('='))
            .filter_map(|(k, v)| Some((k.to_owned(), v.parse().ok()?)))
            .collect())
    }

    /// The name of entry `id` of `registry` as Java sees it (`None` if absent).
    pub fn registry_name(&self, registry: &str, id: i32) -> anyhow::Result<Option<String>> {
        let s = self.with_bridge(|env, bridge| {
            let r = JString::from_str(env, registry)?;
            let v = env
                .call_method(
                    bridge,
                    jni_str!("registryName"),
                    jni_sig!("(Ljava/lang/String;I)Ljava/lang/String;"),
                    &[(&r).into(), jni::objects::JValue::Int(id)],
                )?
                .l()?;
            if v.is_null() {
                return Ok(None);
            }
            // Safety: registryName returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            Ok(Some(v.try_to_string(env)?))
        })?;
        Ok(s)
    }

    fn call_string(&self, name: &'static str) -> Result<String> {
        self.with_bridge(|env, bridge| {
            let name = jni::strings::JNIString::new(name);
            let v = env
                .call_method(bridge, &name, jni_sig!("()Ljava/lang/String;"), &[])?
                .l()?;
            if v.is_null() {
                return Ok(String::new());
            }
            // Safety: the method returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            v.try_to_string(env)
        })
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
const SPRITE_REGION: i32 = 11;
const STYLED_TEXT: i32 = 12;
const ENTITY: i32 = 13;

/// The cursor shape for a code from `GuiBatch.frame` (`None` = the default arrow).
fn cursor_shape(code: i32) -> Option<CursorShape> {
    Some(match code {
        1 => CursorShape::Arrow,
        2 => CursorShape::IBeam,
        3 => CursorShape::Crosshair,
        4 => CursorShape::PointingHand,
        5 => CursorShape::ResizeNs,
        6 => CursorShape::ResizeEw,
        7 => CursorShape::ResizeAll,
        8 => CursorShape::NotAllowed,
        _ => return None,
    })
}

/// The blend mode of a blit (`GuiBatch`'s code, set by `GfxImpl.blend` in Java; 0 also falls back
/// to normal for codes this build does not know).
fn blend_of(code: i32) -> GuiBlend {
    match code {
        1 => GuiBlend::Multiply,
        2 => GuiBlend::Invert,
        3 => GuiBlend::Additive,
        _ => GuiBlend::Normal,
    }
}

fn decode_frame(env: &mut Env, frame: JObject) -> Result<(Vec<GuiCommand>, Option<CursorShape>)> {
    // Safety: Bridge.render returns Object[]{int[], String[], int[1]}.
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
    let cur = frame.get_element(env, 2)?;
    let cur = unsafe { JIntArray::from_raw(env, cur.into_raw()) };
    let mut code = [0i32; 1];
    cur.get_region(env, 0, &mut code)?;
    Ok((decode(&buf, &strings)?, cursor_shape(code[0])))
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
                need(14)?;
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
                    blend: blend_of(b[i + 13]),
                });
                i += 14;
            }
            SPRITE => {
                need(8)?;
                out.push(GuiCommand::Sprite {
                    sprite: ident(b[i + 1])?,
                    x: b[i + 2],
                    y: b[i + 3],
                    w: b[i + 4],
                    h: b[i + 5],
                    tint: b[i + 6] as u32,
                    blend: blend_of(b[i + 7]),
                });
                i += 8;
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
            SPRITE_REGION => {
                need(12)?;
                out.push(GuiCommand::SpriteRegion {
                    sprite: ident(b[i + 1])?,
                    sprite_w: b[i + 2],
                    sprite_h: b[i + 3],
                    u: b[i + 4],
                    v: b[i + 5],
                    x: b[i + 6],
                    y: b[i + 7],
                    w: b[i + 8],
                    h: b[i + 9],
                    tint: b[i + 10] as u32,
                    blend: blend_of(b[i + 11]),
                });
                i += 12;
            }
            STYLED_TEXT => {
                need(7)?;
                let fl = b[i + 6];
                out.push(GuiCommand::StyledText {
                    x: b[i + 1],
                    y: b[i + 2],
                    color: b[i + 3] as u32,
                    shadow: b[i + 4] != 0,
                    text: s(b[i + 5])?,
                    style: TextStyle {
                        bold: fl & 1 != 0,
                        italic: fl & 2 != 0,
                        underline: fl & 4 != 0,
                        strikethrough: fl & 8 != 0,
                        obfuscated: fl & 16 != 0,
                    },
                });
                i += 7;
            }
            ENTITY => {
                need(9)?;
                out.push(GuiCommand::Entity {
                    entity: EntityId(b[i + 1]),
                    x1: b[i + 2],
                    y1: b[i + 3],
                    x2: b[i + 4],
                    y2: b[i + 5],
                    scale: f(b[i + 6] as u32),
                    mouse_x: f(b[i + 7] as u32),
                    mouse_y: f(b[i + 8] as u32),
                });
                i += 9;
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
            0,
            SPRITE,
            1,
            10,
            20,
            200,
            20,
            -1,
            3,
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
            SPRITE_REGION,
            1,
            182,
            5,
            0,
            0,
            10,
            20,
            91,
            5,
            -1,
            0,
            STYLED_TEXT,
            1,
            2,
            0xFFFFFF,
            0,
            0,
            1 | 4,
            ENTITY,
            42,
            1,
            2,
            3,
            4,
            fb(30.0),
            fb(5.5),
            fb(6.5),
        ];
        let c = decode(&b, &strings).unwrap();
        assert_eq!(c.len(), 13);
        assert!(
            matches!(&c[10], GuiCommand::SpriteRegion { sprite, sprite_w: 182, w: 91, x: 10, .. } if sprite.path() == "widget/button")
        );
        assert!(
            matches!(&c[11], GuiCommand::StyledText { style, text, .. } if style.bold && style.underline && !style.italic && text == "hi")
        );
        assert_eq!(
            c[12],
            GuiCommand::Entity {
                entity: EntityId(42),
                x1: 1,
                y1: 2,
                x2: 3,
                y2: 4,
                scale: 30.0,
                mouse_x: 5.5,
                mouse_y: 6.5
            }
        );
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
            matches!(&c[3], GuiCommand::Texture { texture, u, tex_w: 256, blend: GuiBlend::Normal, .. } if texture.path() == "textures/gui/x.png" && *u == 0.5)
        );
        assert!(
            matches!(&c[4], GuiCommand::Sprite { sprite, w: 200, blend: GuiBlend::Additive, .. } if sprite.path() == "widget/button")
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

    #[test]
    fn decodes_blend_codes() {
        assert_eq!(blend_of(0), GuiBlend::Normal);
        assert_eq!(blend_of(1), GuiBlend::Multiply);
        assert_eq!(blend_of(2), GuiBlend::Invert);
        assert_eq!(blend_of(3), GuiBlend::Additive);
    }
}
