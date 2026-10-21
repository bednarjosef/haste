//! What a camera viewing the world from an entity reads off it (vanilla `Camera.alignWithEntity`,
//! `Camera.tick` and `Camera.extractRenderState` with a spectated entity as `Camera.entity`):
//! the view rotation (`Entity.getViewYRot` / `getViewXRot`), the eye height
//! (`Entity.getEyeHeight`) and the living entity's hurt and death counters.

use haste_core::entity::Pose;
use haste_core::ids::EntityId;

use crate::store::{Entity, EntityStore, DEATH_TICKS, HURT_TICKS};
use crate::walk_swing::{lerp, rot_lerp};

/// `LivingEntity.SLEEPING_DIMENSIONS`' eye height: every living entity's, sleeping, unscaled.
const SLEEPING_EYE_HEIGHT: f32 = 0.2;

/// The eye height of an avatar (player or mannequin) in `pose`: `Avatar.POSES`, whose
/// `getOrDefault` falls back to the standing dimensions for every pose it does not name.
pub fn avatar_eye_height(pose: Pose) -> f32 {
    match pose {
        Pose::Crouching => 1.27,
        Pose::FallFlying | Pose::Swimming | Pose::SpinAttack => 0.4,
        Pose::Sleeping => SLEEPING_EYE_HEIGHT,
        // STANDING and DYING (1.62 both), and the poses the map lacks.
        _ => 1.62,
    }
}

/// A living entity's camera tilt inputs (vanilla `CameraRenderState.entityRenderState`'s
/// `hurtTime` and `deathTime`, before the partial tick is folded in).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LivingTilt {
    /// `LivingEntity.hurtTime`: ticks of the hurt animation left (10 per hit).
    pub hurt_time: u32,
    /// `LivingEntity.deathTime` while `isDeadOrDying`, `None` alive.
    pub death_time: Option<u32>,
}

impl Entity {
    /// Whether the type is a `LivingEntity` (the camera's hurt and death tilt, and the head
    /// yaw as the view yaw, only apply to one).
    pub fn is_living(&self) -> bool {
        self.is_living
    }

    /// `Entity.getViewYRot(partial)`: a living entity looks along its head yaw
    /// (`LivingEntity.getViewYRot`: `yHeadRot` at 1, else `rotLerp(yHeadRotO, yHeadRot)`), any
    /// other entity along its body rotation (`Entity.getYRot(partial)`).
    pub fn view_y_rot(&self, partial: f32) -> f32 {
        match (self.is_living, partial == 1.0) {
            (true, true) => self.head_yaw,
            (true, false) => rot_lerp(partial, self.prev_head_yaw, self.head_yaw),
            (false, _) => self.yaw_at(partial),
        }
    }

    /// `Entity.getViewXRot(partial)`: `Entity.getXRot(partial)` for every entity.
    pub fn view_x_rot(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.pitch
        } else {
            lerp(partial, self.prev_pitch, self.pitch)
        }
    }

    /// The hurt and death counters a camera viewing from this entity tilts with (`None` for
    /// an entity that is not living: `GameRenderer.bobHurt` skips it). While dead, the store's
    /// `hurt_ticks` runs the 20-tick death counter down, so `deathTime` is the ticks it has
    /// counted (0 on the tick of death, like vanilla's) and the killing blow's own hurt
    /// animation is what is left of its 10 ticks inside that window.
    pub fn living_tilt(&self) -> Option<LivingTilt> {
        if !self.is_living {
            return None;
        }
        Some(if self.dead {
            LivingTilt {
                hurt_time: self.hurt_ticks.saturating_sub(DEATH_TICKS - HURT_TICKS),
                death_time: Some(DEATH_TICKS.saturating_sub(self.hurt_ticks)),
            }
        } else {
            LivingTilt {
                hurt_time: self.hurt_ticks.min(HURT_TICKS),
                death_time: None,
            }
        })
    }
}

impl EntityStore {
    /// `Entity.getEyeHeight()` of a tracked entity (`None` when it is not tracked): an avatar's
    /// from `Avatar.POSES` by pose; another living entity's is the sleeping eye height while
    /// sleeping (`LivingEntity.getDimensions`), else its type's, halved for a baby (the age
    /// scale); any other entity's is its type's. The `SCALE` attribute is not applied (the
    /// store does not see attributes), nor are the few types whose dimensions depend on their
    /// own data (slime size, phantom size).
    pub fn eye_height(&self, id: EntityId) -> Option<f32> {
        let e = self.get(id)?;
        let avatar = e.is_player_type
            || crate::registry::entity_type_name(e.kind) == Some("minecraft:mannequin");
        Some(if avatar {
            avatar_eye_height(e.pose)
        } else if e.is_living && e.pose == Pose::Sleeping {
            SLEEPING_EYE_HEIGHT
        } else {
            let baby = e.is_living && crate::registry::is_baby(&e.metadata);
            self.dims.eye_height(e.kind, baby)
        })
    }
}
