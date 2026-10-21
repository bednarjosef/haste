//! The client side of `NewMinecartBehavior`: the minecart movement of the
//! `minecart_improvements` experiment (`AbstractMinecart.useExperimentalMovement`).
//!
//! A cart on the new behaviour does not interpolate move packets (its interpolation handler is
//! `InterpolationHandler.NO_OP`, see [`crate::lerp`]); the server sends its path instead, as the
//! steps of `move_minecart_along_track` (`ClientPacketListener.handleMinecartAlongTrack` adds
//! them to `lerpSteps`). Every three ticks the client takes the steps that arrived as one window
//! (`lerpClientPositionAndRotation`) and walks the cart through them over the next three ticks,
//! each step taking the share of the window its weight (the distance it covers) gives it
//! (`getCurrentLerpStep`). The renderer, the passengers and the riding camera read the same walk
//! at the frame's partial tick (`getCartLerpPosition`/`getCartLerpYRot`/`getCartLerpXRot`), so
//! the cart follows its track through corners and slopes between the server's packets.
use haste_core::math::Vec3;
use haste_core::model::{BodyOp, BodyTransform};
use haste_core::{DVec3, EntityId};

use crate::store::{rot_lerp, Entity, EntityStore};

/// `NewMinecartBehavior.POS_ROT_LERP_TICKS`: the ticks one window of steps is spread over.
pub const POS_ROT_LERP_TICKS: i32 = 3;

/// One point of the cart's path (`NewMinecartBehavior.MinecartStep`): where it is, its
/// movement, its rotation (degrees) and the step's weight (the distance it covers; 0 for an
/// instant snap onto the rails).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinecartStep {
    pub position: DVec3,
    pub movement: DVec3,
    pub y_rot: f32,
    pub x_rot: f32,
    pub weight: f32,
}

impl MinecartStep {
    /// `MinecartStep.ZERO`: what `oldLerp` holds before the first window.
    pub const ZERO: MinecartStep = MinecartStep {
        position: DVec3::ZERO,
        movement: DVec3::ZERO,
        y_rot: 0.0,
        x_rot: 0.0,
        weight: 0.0,
    };
}

/// `NewMinecartBehavior.StepPartialTicks`: the step the walk is in at some partial tick, the
/// one before it, and how far between the two it is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct StepPartialTicks {
    partial_ticks_in_step: f32,
    current: MinecartStep,
    previous: MinecartStep,
}

/// A minecart's `NewMinecartBehavior` state on the client: the steps waiting for the next
/// window and the window being walked. The two lists keep their capacity, so a running cart
/// allocates nothing per tick or per packet once they have grown to its usual window.
#[derive(Clone, Debug)]
pub struct NewMinecartBehavior {
    /// `lerpDelay`: ticks left in the current window; counts below zero while idle.
    lerp_delay: i32,
    /// `lerpSteps`: the steps that arrived since the window started.
    lerp_steps: Vec<MinecartStep>,
    /// `currentLerpSteps`: the window being walked.
    current_lerp_steps: Vec<MinecartStep>,
    /// `currentLerpStepsTotalWeight`.
    current_lerp_steps_total_weight: f64,
    /// `oldLerp`: where the cart was when the window started (the walk's starting point).
    old_lerp: MinecartStep,
}

impl Default for NewMinecartBehavior {
    fn default() -> Self {
        NewMinecartBehavior::new()
    }
}

impl NewMinecartBehavior {
    pub fn new() -> NewMinecartBehavior {
        NewMinecartBehavior {
            lerp_delay: 0,
            lerp_steps: Vec::new(),
            current_lerp_steps: Vec::new(),
            current_lerp_steps_total_weight: 0.0,
            old_lerp: MinecartStep::ZERO,
        }
    }

    /// `ClientPacketListener.handleMinecartAlongTrack`: the packet's steps join `lerpSteps`,
    /// walked from the next window on.
    pub fn add_lerp_steps(&mut self, steps: impl IntoIterator<Item = MinecartStep>) {
        self.lerp_steps.extend(steps);
    }

    /// `NewMinecartBehavior.lerpClientPositionAndRotation`, the client half of its `tick`:
    /// when the window has run out, the waiting steps become the next one (starting from
    /// `current`, the cart's present pose, `setOldLerpValues`); then, while a window is being
    /// walked, the cart's pose becomes the walk's point at the end of this tick (returned; the
    /// caller sets position, movement and rotation from it). `None` leaves the cart as it is.
    pub fn lerp_client_position_and_rotation(
        &mut self,
        current: MinecartStep,
    ) -> Option<MinecartStep> {
        self.lerp_delay -= 1;
        if self.lerp_delay <= 0 {
            self.old_lerp = MinecartStep {
                weight: 0.0,
                ..current
            };
            self.current_lerp_steps.clear();
            if !self.lerp_steps.is_empty() {
                // The waiting list becomes the window; the old window's storage takes the
                // next arrivals (no allocation once both have grown).
                std::mem::swap(&mut self.current_lerp_steps, &mut self.lerp_steps);
                self.current_lerp_steps_total_weight = self
                    .current_lerp_steps
                    .iter()
                    .fold(0.0, |total, s| total + f64::from(s.weight));
                self.lerp_delay = if self.current_lerp_steps_total_weight == 0.0 {
                    0
                } else {
                    POS_ROT_LERP_TICKS
                };
            }
        }
        if !self.cart_has_pos_rot_lerp() {
            return None;
        }
        Some(MinecartStep {
            position: self.cart_lerp_position(1.0),
            movement: self.cart_lerp_movements(1.0),
            y_rot: self.cart_lerp_y_rot(1.0),
            x_rot: self.cart_lerp_x_rot(1.0),
            weight: 0.0,
        })
    }

    /// `cartHasPosRotLerp`: a window is being walked, so the render state, the passengers and
    /// the riding camera take the cart's pose from the walk.
    pub fn cart_has_pos_rot_lerp(&self) -> bool {
        !self.current_lerp_steps.is_empty()
    }

    /// `getCartLerpXRot`: the cart's pitch at `partial_ticks` (degrees).
    pub fn cart_lerp_x_rot(&self, partial_ticks: f32) -> f32 {
        let s = self.current_lerp_step(partial_ticks);
        rot_lerp(s.partial_ticks_in_step, s.previous.x_rot, s.current.x_rot)
    }

    /// `getCartLerpYRot`: the cart's yaw at `partial_ticks` (degrees).
    pub fn cart_lerp_y_rot(&self, partial_ticks: f32) -> f32 {
        let s = self.current_lerp_step(partial_ticks);
        rot_lerp(s.partial_ticks_in_step, s.previous.y_rot, s.current.y_rot)
    }

    /// `getCartLerpPosition`: the cart's position at `partial_ticks`.
    pub fn cart_lerp_position(&self, partial_ticks: f32) -> DVec3 {
        let s = self.current_lerp_step(partial_ticks);
        lerp_vec(
            s.partial_ticks_in_step,
            s.previous.position,
            s.current.position,
        )
    }

    /// `getCartLerpMovements`: the cart's movement at `partial_ticks`.
    pub fn cart_lerp_movements(&self, partial_ticks: f32) -> DVec3 {
        let s = self.current_lerp_step(partial_ticks);
        lerp_vec(
            s.partial_ticks_in_step,
            s.previous.movement,
            s.current.movement,
        )
    }

    /// `getCurrentLerpStep`: how far through the window the walk is at `partial_tick` (a third
    /// of it per tick, `(3 - lerpDelay + partialTick) / 3`), and so which step it is in: the
    /// first whose running weight reaches that share of the total, entered from the step
    /// before it (or from `oldLerp` for the first). Steps without weight are passed over; when
    /// none reaches the share, the walk rests on the last step.
    ///
    /// Vanilla memoises the result per (partial tick, `lerpDelay`); the walk is a few steps
    /// long, so it is recomputed instead (the memo changes nothing but the cost).
    fn current_lerp_step(&self, partial_tick: f32) -> StepPartialTicks {
        let steps = &self.current_lerp_steps;
        let total = self.current_lerp_steps_total_weight;
        let alpha = (POS_ROT_LERP_TICKS - self.lerp_delay) as f32 + partial_tick;
        let alpha = alpha / POS_ROT_LERP_TICKS as f32;
        let mut count_up = 0.0f32;
        let mut indexed_partial_tick = 1.0f32;
        let mut found = None;
        for (index, step) in steps.iter().enumerate() {
            let weight = step.weight;
            if weight <= 0.0 {
                continue;
            }
            count_up += weight;
            if f64::from(count_up) >= total * f64::from(alpha) {
                let before = count_up - weight;
                indexed_partial_tick =
                    ((f64::from(alpha) * total - f64::from(before)) / f64::from(weight)) as f32;
                found = Some(index);
                break;
            }
        }
        let index = found.unwrap_or(steps.len().saturating_sub(1));
        let current = steps.get(index).copied().unwrap_or(self.old_lerp);
        let previous = if index > 0 {
            steps[index - 1]
        } else {
            self.old_lerp
        };
        StepPartialTicks {
            partial_ticks_in_step: indexed_partial_tick,
            current,
            previous,
        }
    }
}

/// `Mth.lerp(double, Vec3, Vec3)`, with the float partial widened like vanilla's call.
fn lerp_vec(alpha: f32, from: DVec3, to: DVec3) -> DVec3 {
    let a = f64::from(alpha);
    DVec3::new(
        from.x + a * (to.x - from.x),
        from.y + a * (to.y - from.y),
        from.z + a * (to.z - from.z),
    )
}

impl Entity {
    /// `AbstractMinecart`'s constructor in a level with the `minecart_improvements` feature:
    /// the cart runs `NewMinecartBehavior`, and its interpolation handler is the no-op one.
    pub(crate) fn start_new_minecart_behavior(&mut self) {
        if self.new_minecart.is_none() {
            self.new_minecart = Some(Box::default());
            self.lerp.set_no_op();
        }
    }
}

impl EntityStore {
    /// A minecart spawned in a level with the `minecart_improvements` feature
    /// (`AbstractMinecart.useExperimentalMovement`, read by its constructor): it runs
    /// `NewMinecartBehavior`. Any other entity type is left alone.
    pub fn use_new_minecart_behavior(&mut self, id: EntityId) {
        if let Some(e) = self.get_mut(id) {
            if crate::minecart::variant_for_kind(e.kind).is_some() {
                e.start_new_minecart_behavior();
            }
        }
    }

    /// `ClientPacketListener.handleMinecartAlongTrack`: a minecart on the new behaviour adds
    /// the packet's steps to the ones waiting for its next window; any other entity ignores
    /// them.
    pub fn handle_minecart_along_track(
        &mut self,
        id: EntityId,
        steps: impl IntoIterator<Item = MinecartStep>,
    ) {
        if let Some(behavior) = self.get_mut(id).and_then(|e| e.new_minecart.as_deref_mut()) {
            behavior.add_lerp_steps(steps);
        }
    }
}

/// `NewMinecartBehavior.tick` on the client (from `AbstractMinecart.tick`, after the tick's
/// `setOldPosAndRot`): the track walk sets the cart's position, movement and rotation.
/// Nothing for an entity without the new behaviour.
pub(crate) fn tick(e: &mut Entity) {
    let Some(behavior) = e.new_minecart.as_deref_mut() else {
        return;
    };
    let current = MinecartStep {
        position: e.pos,
        movement: e.velocity,
        y_rot: e.yaw,
        x_rot: e.pitch,
        weight: 0.0,
    };
    if let Some(pose) = behavior.lerp_client_position_and_rotation(current) {
        e.pos = pose.position;
        e.velocity = pose.movement;
        e.pitch = pose.x_rot;
        e.yaw = pose.y_rot;
    }
}

/// `AbstractMinecartRenderer.newExtractState` plus `getRenderOffset`: where a cart on the new
/// behaviour is drawn and how it is turned at `partial` — the walk's point and rotation while a
/// window is walked (`renderPos`, `getCartLerpYRot`/`getCartLerpXRot`), else the entity's
/// interpolated position with its present yaw and pitch (not interpolated, like vanilla).
/// `None` for a cart on the old behaviour (and every other entity).
pub fn render_pose(e: &Entity, partial: f32) -> Option<(DVec3, f32, f32)> {
    let behavior = e.new_minecart.as_deref()?;
    Some(if behavior.cart_has_pos_rot_lerp() {
        (
            behavior.cart_lerp_position(partial),
            behavior.cart_lerp_y_rot(partial),
            behavior.cart_lerp_x_rot(partial),
        )
    } else {
        (e.prev_pos.lerp(e.pos, f64::from(partial)), e.yaw, e.pitch)
    })
}

/// `AbstractMinecartRenderer.newRender`: the cart turned by its yaw, then pitched about its
/// long axis, then lifted 0.375 in that turned frame, handed to the renderer as the whole-model
/// transform (`EntityInstance::body`, whose ops compose in `poseStack` order). A cart whose
/// body carries ops draws this way; one without draws the old, rail-snapped way.
pub fn new_render(y_rot: f32, x_rot: f32) -> BodyTransform {
    let mut body = BodyTransform::default();
    body.push(BodyOp::RotateY(y_rot.to_radians()));
    body.push(BodyOp::RotateZ((-x_rot).to_radians()));
    body.push(BodyOp::Translate(Vec3::new(0.0, 0.375, 0.0)));
    body
}

/// `EntityRenderer.extractRenderState`'s `passengerOffset` for a passenger of `cart`: while the
/// cart walks a window, its passengers are drawn moved by the difference between the walk's
/// point and the cart's own interpolated position, so they ride along the track with it.
/// `None` otherwise (no offset).
pub fn passenger_offset(cart: &Entity, partial: f32) -> Option<DVec3> {
    let behavior = cart.new_minecart.as_deref()?;
    if !behavior.cart_has_pos_rot_lerp() {
        return None;
    }
    let lerped = cart.prev_pos.lerp(cart.pos, f64::from(partial));
    Some(behavior.cart_lerp_position(partial) - lerped)
}
