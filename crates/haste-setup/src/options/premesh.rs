//! Haste's pre-mesh option (`[options] premesh`): turn prediction for the mesh gate, a
//! Haste-only setting with no vanilla counterpart (vanilla's `LevelExtractor` meshes
//! only the visible sections and eats the pop-in on quick turns).

use serde::{Deserialize, Serialize};

/// The pre-mesh mode: `off` is exactly vanilla's visible-only meshing; `cone` widens the
/// mesh gate's frustum toward the camera's turn; `predict` drives that widening with an
/// online predictor of the next half-second of yaw. Default `cone`: since the pre-mesh
/// uploads take only the frame's slack (the 1 ms slice after the visible sections'
/// budget), `cone`'s frame cost is inside the turns bench's run-to-run noise (2026-10-20,
/// rd 12: 2.32/2.44 ms avg across two runs vs 2.21/2.76 with `off`; rd 32: 8.09 vs
/// 7.67-8.07) while cutting pop-in to near zero (rd 12: 1.2-1.3 frames avg, worst 2-3,
/// 30-35 visible-unmeshed section-frames vs 90-98 / 3500-4200 / 37000-43000 with `off`).
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
