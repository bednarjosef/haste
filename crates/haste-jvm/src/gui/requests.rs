//! Menu-mode requests the screens put into the Rust-owned state, and the disconnect screens:
//! the client polls these every frame.

use jni::objects::JString;
use jni::{jni_sig, jni_str};

use super::GuiPipeline;
use crate::error::Result;
use crate::proxies::with_state;

impl GuiPipeline {
    /// The connection ended: vanilla's disconnect screen with the reason.
    pub fn disconnected(&self, reason: &haste_core::Component) -> anyhow::Result<()> {
        self.disconnected_json(&serde_json::to_string(reason)?)
    }

    /// The multiplayer screen's Join: the address to connect to, if any was clicked since the
    /// last call (menu mode; the connection is Rust's).
    pub fn take_join_request(&self) -> Option<String> {
        self.with_state(|s| s.join_requested.take())
    }

    /// Whether the connecting screen's Cancel was clicked since the last call (a join still in
    /// progress must be dropped).
    pub fn take_join_cancelled(&self) -> bool {
        self.with_state(|s| std::mem::take(&mut s.join_cancelled))
    }

    /// The title screen's Quit Game was clicked (the client should exit).
    pub fn take_quit_request(&self) -> bool {
        self.with_state(|s| std::mem::take(&mut s.quit_requested))
    }

    /// The world list's "Play Selected World": the world folder to start the local server for
    /// and join, if any was clicked since the last call.
    pub fn take_open_world_request(&self) -> Option<String> {
        self.with_state(|s| s.open_world_requested.take())
    }

    /// The create screen's "Create New World": the world to create, if requested since the last
    /// call.
    pub fn take_create_world_request(&self) -> Option<crate::proxies::CreateWorldRequest> {
        self.with_state(|s| s.create_world_requested.take())
    }

    /// The resource pack selection the pack screen committed since the last call (vanilla's
    /// `Minecraft.reloadResourcePacks`): repository ids, bottom first. The caller rebuilds its
    /// asset stack and renderer from them.
    pub fn take_resource_pack_ids(&self) -> Option<Vec<String>> {
        self.with_state(|s| s.resource_pack_ids.take())
    }

    /// Tells the screens whether the local singleplayer server is running (the pause screen's
    /// "Save and Quit" label). Set when the client starts/stops it.
    pub fn set_local_world_running(&self, running: bool) {
        self.with_state(|s| s.local_world_running = running);
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

    /// F3: shows or hides the debug overlay (vanilla `DebugScreenEntryList.toggleDebugOverlay`).
    /// The state lives in Rust (`GameState::debug_shown`); Java reads it from the mirror.
    pub fn toggle_debug(&self) {
        with_state(|s| s.debug_shown = !s.debug_shown);
    }

    /// Whether the F3 overlay is on.
    pub fn debug_shown(&self) -> bool {
        with_state(|s| s.debug_shown)
    }

    fn call_void(&self, name: &'static str) -> Result<()> {
        self.with_bridge(|env, bridge| {
            let name = jni::strings::JNIString::new(name);
            env.call_method(bridge, &name, jni_sig!("()V"), &[])?;
            Ok(())
        })
    }
}
