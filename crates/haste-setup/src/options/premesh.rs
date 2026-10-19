//! Haste's pre-mesh option (`[options] premesh`): turn prediction for the mesh gate, a
//! Haste-only setting with no vanilla counterpart (vanilla's `LevelExtractor` meshes
//! only the visible sections and eats the pop-in on quick turns).

use serde::{Deserialize, Serialize};

/// The pre-mesh mode: `off` is exactly vanilla's visible-only meshing; `cone` widens the
/// mesh gate's frustum toward the camera's turn; `predict` drives that widening with an
/// online predictor of the next half-second of yaw. Default `off`: the lead's A/B on the
/// turns bench (2026-10-19, rd 12 and 32) showed `cone` cuts unmeshed section-frames by
/// 20-55 % but makes frames 10-25 % slower; it becomes the default once that cost is gone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PremeshMode {
    #[default]
    Off,
    Cone,
    Predict,
}

impl PremeshMode {
    /// Parses one option value: the serialized spellings (`off`, `cone`, `predict`).
    pub fn parse(value: &str) -> Option<PremeshMode> {
        match value.trim() {
            "off" => Some(PremeshMode::Off),
            "cone" => Some(PremeshMode::Cone),
            "predict" => Some(PremeshMode::Predict),
            _ => None,
        }
    }
}
