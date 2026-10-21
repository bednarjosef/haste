//! Leashes on the client (`Leashable`, `Mob.leashHolder`): which entity is tied to which, and
//! where the rope runs between its two ends. The attached end follows `Leashable.getLeashOffset`;
//! the holder end follows `Entity.getRopeHoldPosition` for a mob and
//! `LeashFenceKnotEntity.getRopeHoldPosition` for a knot.

use ahash::AHashMap;
use haste_core::{DVec3, EntityId, RegistryIndex};

use crate::registry::entity_type_name;
use crate::store::{Entity, EntityStore};

/// `LeashFenceKnotEntity`'s render origin from its block corner: `recalculateBoundingBox` centres
/// the knot's box on the block's x and z, and lifts it to 0.375 of a block.
pub const KNOT_OFFSET: DVec3 = DVec3::new(0.5, 0.375, 0.5);

/// `Entity.getRopeHoldPosition`: a mob's rope hangs at this fraction of its eye height.
pub const MOB_ROPE_HOLD: f64 = 0.7;

/// `LeashFenceKnotEntity.getRopeHoldPosition`: the rope's point, this far above the knot's origin.
pub const KNOT_ROPE_HOLD_Y: f64 = 0.2;

/// Whether an entity type is the fence knot a leash ties to (`minecraft:leash_knot`).
pub fn is_knot(kind: RegistryIndex) -> bool {
    entity_type_name(kind) == Some("minecraft:leash_knot")
}

/// The knot's render origin from the block position its spawn packet carries.
pub fn knot_center(block_pos: DVec3) -> DVec3 {
    block_pos + KNOT_OFFSET
}

/// `Vec3.yRot(angle)`: rotates about the Y axis (`x' = x*cos + z*sin`, `z' = z*cos - x*sin`).
pub fn y_rot(v: DVec3, angle: f64) -> DVec3 {
    let (sin, cos) = angle.sin_cos();
    DVec3::new(v.x * cos + v.z * sin, v.y, v.z * cos - v.x * sin)
}

/// `Leashable.getLeashOffset`: the rope's start on the attached entity, relative to its position.
/// It sits at the eye height, 0.4 of the width out in front of the body, turned by the body yaw.
pub fn attach_offset(eye_height: f64, width: f64, body_yaw_deg: f32) -> DVec3 {
    let body = f64::from(body_yaw_deg).to_radians();
    y_rot(DVec3::new(0.0, eye_height, width * 0.4), -body)
}

/// The holder's rope point from its render origin: a knot's sits `KNOT_ROPE_HOLD_Y` up; a mob's
/// sits `MOB_ROPE_HOLD` of its eye height up.
pub fn rope_hold(origin: DVec3, eye_height: f64, knot: bool) -> DVec3 {
    let up = if knot {
        KNOT_ROPE_HOLD_Y
    } else {
        eye_height * MOB_ROPE_HOLD
    };
    origin + DVec3::new(0.0, up, 0.0)
}

/// One rope on screen: both ends in world space. Light is sampled at each end by the client.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeashLine {
    pub start: DVec3,
    pub end: DVec3,
}

/// The leashed entities and the holder each is tied to (`attached` -> `holder`).
#[derive(Default)]
pub(crate) struct Leashes {
    holders: AHashMap<EntityId, EntityId>,
}

impl Leashes {
    /// `SetEntityLink`: ties `attached` to `holder`, or unties it when `holder` is `None`.
    pub(crate) fn set(&mut self, attached: EntityId, holder: Option<EntityId>) {
        match holder {
            Some(h) => {
                self.holders.insert(attached, h);
            }
            None => {
                self.holders.remove(&attached);
            }
        }
    }

    /// Drops every link a removed entity is part of, on either end.
    pub(crate) fn remove_entities(&mut self, ids: &[EntityId]) {
        if !self.holders.is_empty() {
            self.holders
                .retain(|a, h| !ids.contains(a) && !ids.contains(h));
        }
    }

    pub(crate) fn clear(&mut self) {
        self.holders.clear();
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (EntityId, EntityId)> + '_ {
        self.holders.iter().map(|(a, h)| (*a, *h))
    }
}

impl EntityStore {
    /// `SetEntityLink`: ties `attached` to `holder`, or unties it when `holder` is `None`.
    pub fn set_leash(&mut self, attached: EntityId, holder: Option<EntityId>) {
        self.leashes.set(attached, holder);
    }

    /// The ropes at `partial`, written into `out` (cleared first). A link whose end is not
    /// tracked yet draws nothing; the link itself stays until removed or untied.
    pub fn leash_lines(&self, partial: f32, out: &mut Vec<LeashLine>) {
        out.clear();
        for (attached, holder) in self.leashes.iter() {
            let (Some(a), Some(h)) = (self.get(attached), self.get(holder)) else {
                continue;
            };
            out.push(LeashLine {
                start: self.leash_attach_point(a, partial),
                end: self.leash_hold_point(h, partial),
            });
        }
    }

    /// The attached end: the entity's position plus `attach_offset` (`Leashable.getLeashOffset`).
    fn leash_attach_point(&self, e: &Entity, partial: f32) -> DVec3 {
        let pos = e.prev_pos.lerp(e.pos, f64::from(partial));
        let eye = self
            .dims
            .eye_height(e.kind, crate::registry::is_baby(&e.metadata));
        pos + crate::leash::attach_offset(
            f64::from(eye),
            f64::from(e.dimensions.0),
            e.body_yaw_at(partial),
        )
    }

    /// The holder end: a knot's rope point, or a mob's `rope_hold` from its position.
    fn leash_hold_point(&self, e: &Entity, partial: f32) -> DVec3 {
        let pos = e.prev_pos.lerp(e.pos, f64::from(partial));
        if is_knot(e.kind) {
            return rope_hold(knot_center(pos), 0.0, true);
        }
        let eye = self
            .dims
            .eye_height(e.kind, crate::registry::is_baby(&e.metadata));
        rope_hold(pos, f64::from(eye), false)
    }
}
