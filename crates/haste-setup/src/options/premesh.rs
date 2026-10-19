//! Haste's pre-mesh option (`[options] premesh`): turn prediction for the mesh gate, a
//! Haste-only setting with no vanilla counterpart (vanilla's `LevelExtractor` meshes
//! only the visible sections and eats the pop-in on quick turns).

use serde::{Deserialize, Serialize};

/// The pre-mesh mode: `off` is exactly vanilla's visible-only meshing; `cone` widens the
/// mesh gate's frustum toward the camera's turn; `predict` drives that widening with an
/// online predictor of the next half-second of yaw. Default `off`: the pre-mesh uploads
/// now take only the frame's slack (the 1 ms slice after the visible sections' budget),
/// and the turns bench at the screenshot spot showed cone inside run-to-run noise — but
/// the lead's A/B at the 144.5 100 -191.5 reference spot did not confirm it (rd 12 pop-in
/// far from near-zero, rd 32 +50 %), so the default stays `off` until interleaved runs at
/// that spot back the switch.
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
