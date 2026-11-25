//! The batched GUI pipeline: real vanilla screens in, one [`haste_core::gui::GuiFrame`] per render
//! out.
//!
//! Every method is one JNI call into `haste.gui.Bridge`. `render` returns the whole frame from that
//! one call as `{int[] commands, String[] strings}` (layout in `java/haste/rt/GuiBatch.java`),
//! which [`frame::decode_frame`] turns into [`haste_core::gui::GuiCommand`]s.
//!
//! Coordinates: GUI-scaled pixels (vanilla "GUI scale"), for both `render`'s mouse position and the
//! input methods. Key numbering: vanilla 26.3's (`InputConstants`, SDL3 scancodes for `key`,
//! SDL keymod bits for `mods`), see [`crate::keys`]. Mouse buttons: 1 left, 2 middle, 3 right.
//!
//! The methods are split by responsibility into child modules: [`call`] resolves and performs the
//! low-level JNI calls, [`input`] forwards input and renders the frame, [`screens`] opens and
//! inspects screens, [`session`] feeds the connection's data, [`blocks`] mirrors block states,
//! [`requests`] polls the menu screens' requests, [`queries`] reads chat/commands/registries,
//! [`bridge_call`] calls one-off bridge methods, [`frame`] decodes a frame and [`pack_prompt`]
//! runs the resource pack prompt.

use std::sync::atomic::AtomicU32;

use jni::objects::JObject;
use jni::refs::Global;
use jni::signature::{JavaType, Primitive};
use parking_lot::Mutex;

mod blocks;
mod bridge_call;
mod call;
mod frame;
mod input;
mod pack_prompt;
mod queries;
mod requests;
mod screens;
mod session;

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
    /// `net.minecraft.client.gui.screens.packs.PackSelectionScreen` (the Options screen's
    /// Resource Packs button; commits through `Minecraft.reloadResourcePacks`, which the
    /// ClientImpl proxy reports to Rust).
    Packs,
    /// `net.minecraft.client.gui.screens.multiplayer.JoinMultiplayerScreen`, whose parent is a
    /// `TitleScreen`.
    Multiplayer,
    /// `net.minecraft.client.gui.screens.worldselection.SelectWorldScreen`, whose parent is a
    /// `TitleScreen` (the title screen's Singleplayer button).
    SelectWorld,
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
            ScreenKind::Packs => 8,
            ScreenKind::Multiplayer => 6,
            ScreenKind::SelectWorld => 7,
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
    game_key: jni::objects::JMethodID,
    game_mouse: jni::objects::JMethodID,
    game_input: jni::objects::JMethodID,
    consume_clicks: jni::objects::JMethodID,
    block_entity_count: jni::objects::JMethodID,
}

/// Drives real vanilla screens through the bridge. All calls are serialised (the Java side is
/// single-threaded like vanilla's render thread).
pub struct GuiPipeline {
    bridge: Global<JObject<'static>>,
    m: Methods,
    lock: Mutex<()>,
    last_error: Mutex<Option<String>>,
    /// The last frame's panorama spin as f32 bits, [`NO_PANORAMA`] for none.
    panorama: AtomicU32,
}

/// [`GuiPipeline::panorama`]'s "no panorama this frame" (a NaN pattern a spin never has).
const NO_PANORAMA: u32 = u32::MAX;

const BOOL: JavaType = JavaType::Primitive(Primitive::Boolean);
const VOID: JavaType = JavaType::Primitive(Primitive::Void);
