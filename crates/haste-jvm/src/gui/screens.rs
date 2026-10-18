//! Opening, closing and inspecting the open vanilla screen (`Gui.setScreen`).

use jni::signature::JavaType;
use jni::sys::jvalue;

use super::{GuiPipeline, ScreenKind, VOID};
use crate::host::jstring_to_string;

impl GuiPipeline {
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
}
