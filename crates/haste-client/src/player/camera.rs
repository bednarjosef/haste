//! The local player's side of the camera: the camera's smoothed eye height (vanilla
//! `Camera.eyeHeight` / `eyeHeightOld`, kept here because the player's tick is where Haste runs
//! `Camera.tick`), and what changes while another entity is the camera entity (vanilla
//! `LocalPlayer.isControlledCamera` is false): the eye height eases toward that entity's, the
//! movement keys stop moving us (`LocalPlayer.applyInput` and the flying branch of
//! `LocalPlayer.aiStep`), no position packets leave (`LocalPlayer.sendPosition`), and the
//! crosshair picks from that entity's eyes (`LocalPlayer.raycastHitResult(cameraEntity)`).
//! The input packet still goes out every change: its shift flag is how a spectator asks the
//! server to stop spectating (`ServerPlayer.wantsToStopRiding` -> `setCamera(this)`).

use glam::DVec3;
use haste_core::input::InputState;
use haste_core::EntityId;

use super::LocalPlayer;
use crate::game::camera_entity::CameraEntitySnapshot;

impl LocalPlayer {
    /// Eye position and look angles interpolated by partial tick, for the camera. The eye
    /// height is the camera's own smoothed pair (vanilla `Camera.setup` lerps
    /// `eyeHeightOld`/`eyeHeight`, not the entity's eye height).
    pub fn camera(&self, partial_tick: f32) -> (DVec3, f32, f32) {
        let t = partial_tick.clamp(0.0, 1.0);
        let pos = self.prev_pos.lerp(self.state.pos, t as f64);
        let eye = self.camera_eye_old + (self.camera_eye - self.camera_eye_old) * t;
        (
            pos + DVec3::new(0.0, f64::from(eye), 0.0),
            self.state.yaw,
            self.state.pitch,
        )
    }

    /// The camera's eye height pair (`Camera.eyeHeightOld`, `eyeHeight`): the camera keeps one
    /// pair whatever entity it views from, so a switch eases from the old eye height.
    pub fn camera_eye_heights(&self) -> (f32, f32) {
        (self.camera_eye_old, self.camera_eye)
    }

    /// The camera entity for the next tick (`None`: we are it). The session sets it before
    /// each tick from its [`crate::game::camera_entity::CameraEntity`].
    pub fn set_camera_entity(&mut self, camera: Option<CameraEntitySnapshot>) {
        self.camera_entity = camera;
    }

    /// `LocalPlayer.isControlledCamera`: we are the camera entity.
    pub fn is_controlled_camera(&self) -> bool {
        self.camera_entity.is_none()
    }

    /// What `Camera.tick` eases the camera's eye height toward: the camera entity's
    /// `getEyeHeight()`, ours while we are the camera.
    pub(super) fn camera_eye_target(&self) -> f32 {
        self.camera_entity
            .map_or(self.eye_height() as f32, |c| c.eye_height)
    }

    /// The physics tick's input: the keys with the look plus the server's rotation offset.
    /// While another entity is the camera, the movement keys move nothing (vanilla
    /// `LocalPlayer.applyInput` leaves `xxa`/`zza`/`jumping` to the base `LivingEntity` decay
    /// and `aiStep`'s flying up/down needs `isControlledCamera`; the decay of the impulse a
    /// key left is not modelled, the keys simply stop).
    pub(super) fn movement_input(&self, input: &InputState) -> InputState {
        let mut i = *input;
        i.yaw = input.yaw + self.look_offset.0;
        i.pitch = (input.pitch + self.look_offset.1).clamp(-90.0, 90.0);
        if !self.is_controlled_camera() {
            i.forward = false;
            i.back = false;
            i.left = false;
            i.right = false;
            i.jump = false;
            i.sneak = false;
        }
        i
    }

    /// Where the crosshair pick starts, the look it follows and the entity it skips (vanilla
    /// `LocalPlayer.pick(cameraEntity, ...)`: the camera entity's eye position and view vector,
    /// excluding the camera entity itself). `look` is our own, used while we are the camera.
    pub(crate) fn pick_view(&self, look: (f32, f32)) -> (DVec3, (f32, f32), EntityId) {
        match &self.camera_entity {
            Some(c) => (c.eye_position(), c.rot, c.id),
            None => (self.eye(), look, self.entity_id),
        }
    }
}

#[cfg(test)]
mod tests;
