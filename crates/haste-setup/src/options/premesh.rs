//! Haste's pre-mesh option (`[options] premesh`): turn prediction for the mesh gate, a
//! Haste-only setting with no vanilla counterpart (vanilla's `LevelExtractor` meshes
//! only the visible sections and eats the pop-in on quick turns).

use serde::{Deserialize, Serialize};

/// The pre-mesh mode: `off` is exactly vanilla's visible-only meshing; `cone` widens the
/// mesh gate's frustum toward the camera's turn; `predict` drives that widening with an
/// online predictor of the next half-second of yaw. Default `cone` (measured: it removes
/// nearly all turn pop-in at render distance 12 and 32; `--premesh off` restores the
/// vanilla behaviour).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PremeshMode {
    Off,
    #[default]
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
