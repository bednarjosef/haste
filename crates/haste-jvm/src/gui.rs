//! The batched GUI pipeline.
use std::sync::Arc;

use haste_core::gui::TextMetrics;
use jni::objects::JObject;
use jni::refs::Global;
use jni::Env;

use crate::error::Result;

/// The vanilla screens Haste opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKind {
    /// `ChatScreen`.
    Chat,
    /// `PauseScreen`.
    Pause,
    /// `InventoryScreen`.
    Inventory,
}

/// Drives real vanilla screens.
pub struct GuiPipeline {
    #[allow(dead_code)]
    bridge: Global<JObject<'static>>,
}

impl GuiPipeline {
    pub(crate) fn new(_env: &mut Env, bridge: Global<JObject<'static>>) -> Result<GuiPipeline> {
        Ok(GuiPipeline { bridge })
    }
}

impl GuiPipeline {
    /// Installs the renderer's text metrics; `Font.width` and friends call back into them.
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>) {
        crate::natives::set_metrics(m);
    }
}
