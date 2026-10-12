//! Per-instance pose overrides for the renderer. Task 2 fills in the walking limb swing; Task 1
//! hides nothing and returns empty overrides.
use haste_core::model::PoseOverrides;

use crate::store::Entity;

pub(crate) fn pose_overrides(_e: &Entity) -> PoseOverrides {
    PoseOverrides::default()
}
