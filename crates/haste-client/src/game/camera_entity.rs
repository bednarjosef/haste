//! The camera entity (vanilla `Minecraft.getCameraEntity`: the `Camera.entity` that
//! `Minecraft.setCameraEntity` sets). It is the local player, except while a spectator
//! spectates another entity: the server's `set_camera` (`ClientPacketListener.handleSetCamera`)
//! names the entity, and the world is then viewed from its eyes. Login and respawn give the
//! camera back to the player (`handleLogin` / `handleRespawn` call `setCameraEntity(player)`),
//! and so does a `set_camera` naming the player itself (the server's answer to sneaking).
//!
//! What follows the camera entity instead of the player, as vanilla does:
//! - the camera's position and rotation (`Camera.alignWithEntity`: the entity's interpolated
//!   position plus the camera's own smoothed eye height, its `getViewYRot`/`getViewXRot`);
//! - the eye height the camera eases toward (`Camera.tick`);
//! - the FOV modifier (`Camera.tickFov`: 1 for anything but a player) and the hurt and death
//!   tilt (`Camera.extractRenderState` reads them off the camera entity);
//! - which entities the world draws (`LevelExtractor.extractVisibleEntities`: the camera
//!   entity only when the camera is detached, the local player only when it is the camera);
//! - the crosshair pick (`Minecraft.pick` -> `LocalPlayer.raycastHitResult(cameraEntity)`);
//! - the player's own movement input and position packets stop (`LocalPlayer
//!   .isControlledCamera`), while its input packet keeps going (sneak is how a spectator
//!   asks the server to stop spectating).

use glam::DVec3;
use haste_core::EntityId;
use haste_entity::camera_view::LivingTilt;
use haste_entity::EntityStore;

use super::view::ViewState;

/// One tick of the entity the camera views from, bracketed for the frames' interpolation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraEntitySnapshot {
    pub id: EntityId,
    /// `Entity.xo/yo/zo` and the position after the tick.
    pub prev_pos: DVec3,
    pub pos: DVec3,
    /// (`getViewYRot`, `getViewXRot`) at the start and at the end of the tick.
    pub prev_rot: (f32, f32),
    pub rot: (f32, f32),
    /// `Entity.getEyeHeight()`, the target the camera's eye height eases toward.
    pub eye_height: f32,
    /// The hurt and death counters of a living entity (`None`: not living, no tilt).
    pub tilt: Option<LivingTilt>,
}

impl CameraEntitySnapshot {
    /// The tracked entity `id` as its last tick left it, `None` when the store lacks it.
    pub fn of(store: &EntityStore, id: EntityId) -> Option<Self> {
        let e = store.get(id)?;
        Some(CameraEntitySnapshot {
            id,
            prev_pos: e.prev_pos,
            pos: e.pos,
            prev_rot: (e.view_y_rot(0.0), e.view_x_rot(0.0)),
            rot: (e.view_y_rot(1.0), e.view_x_rot(1.0)),
            eye_height: store.eye_height(id)?,
            tilt: e.living_tilt(),
        })
    }

    /// The view rotation at `partial` (`Entity.getViewYRot`: `Mth.rotLerp`, exact at 1;
    /// `getViewXRot`: `Mth.lerp`).
    pub fn rotation_at(&self, partial: f32) -> (f32, f32) {
        if partial == 1.0 {
            return self.rot;
        }
        (
            rot_lerp(partial, self.prev_rot.0, self.rot.0),
            self.prev_rot.1 + partial * (self.rot.1 - self.prev_rot.1),
        )
    }

    /// The camera at `partial` (vanilla `Camera.alignWithEntity`, the branch for anything but
    /// a minecart with its new behaviour): the entity's interpolated position raised by the
    /// camera's own eye height pair (`eye_heights` = `eyeHeightOld`, `eyeHeight`), looking along
    /// the entity's view rotation. Returns (eye, yaw, pitch).
    pub fn camera_at(&self, partial: f32, eye_heights: (f32, f32)) -> (DVec3, f32, f32) {
        let t = partial.clamp(0.0, 1.0);
        let eye = eye_heights.0 + (eye_heights.1 - eye_heights.0) * t;
        let pos = self.prev_pos.lerp(self.pos, f64::from(t)) + DVec3::new(0.0, f64::from(eye), 0.0);
        let (yaw, pitch) = self.rotation_at(t);
        (pos, yaw, pitch)
    }

    /// The entity's own eye position after the tick (`Entity.getEyePosition(1)`: its eye
    /// height, not the camera's smoothed one), where the crosshair pick starts.
    pub fn eye_position(&self) -> DVec3 {
        self.pos + DVec3::new(0.0, f64::from(self.eye_height), 0.0)
    }
}

/// `Mth.rotLerp`: the shortest way round from `start` to `end`.
fn rot_lerp(delta: f32, start: f32, end: f32) -> f32 {
    let mut d = (end - start) % 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    start + delta * d
}

/// The camera entity's state on the session: which entity it is, and that entity as the last
/// refresh found it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraEntity {
    /// `None`: the local player is the camera entity.
    entity: Option<EntityId>,
    /// The spectated entity's last snapshot. A removed entity keeps it: vanilla's camera
    /// holds the removed `Entity` object, frozen where it last was, until the server's next
    /// `set_camera` (it sends one the tick the spectated entity dies or leaves).
    last: Option<CameraEntitySnapshot>,
}

impl CameraEntity {
    /// `ClientPacketListener.handleSetCamera` -> `Minecraft.setCameraEntity`: the packet's
    /// entity becomes the camera when this level knows it (`packet.getEntity(level)`; `known`
    /// says whether it does), our own id gives the camera back to the player, and an unknown id
    /// changes nothing.
    pub fn set_camera(&mut self, id: EntityId, own: EntityId, known: bool) {
        if id == own {
            *self = CameraEntity::default();
        } else if known && self.entity != Some(id) {
            self.entity = Some(id);
            self.last = None;
        }
    }

    /// Re-reads the spectated entity from the store (a no-op while the camera is the player
    /// or after the entity was removed, which keeps the last snapshot).
    pub fn refresh(&mut self, store: &EntityStore) {
        if let Some(id) = self.entity {
            if let Some(s) = CameraEntitySnapshot::of(store, id) {
                self.last = Some(s);
            }
        }
    }

    /// The spectated entity's snapshot: `None` while the camera is the player (or before the
    /// first refresh saw the new camera entity, when the player stays the camera).
    pub fn snapshot(&self) -> Option<&CameraEntitySnapshot> {
        self.entity.and(self.last.as_ref())
    }
}

/// The camera's eye position and look at `partial` (vanilla `Camera.position()`, `yRot()`,
/// `xRot()` before the perspective moves it): the player's eye while it is the camera entity,
/// else the spectated entity's (`Camera.alignWithEntity`) with the camera's own eye height pair.
pub fn camera_pose(
    camera: &CameraEntity,
    player: &crate::player::LocalPlayer,
    partial: f32,
) -> (DVec3, f32, f32) {
    match camera.snapshot() {
        Some(c) => c.camera_at(partial, player.camera_eye_heights()),
        None => player.camera(partial),
    }
}

/// The camera entity as the render thread sees it: the snapshot and the camera's eye height
/// pair (`Camera.eyeHeightOld` / `eyeHeight`) to place the eye with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraEntityView {
    pub entity: CameraEntitySnapshot,
    pub eye_heights: (f32, f32),
}

/// The frame's camera eye and look before the perspective moves it (vanilla `Camera
/// .alignWithEntity`'s first step): the spectated entity's interpolated eye and view
/// rotation, or else the player's interpolated eye with the input layer's look plus the
/// server's rotation offset (the player turns at frame rate while it ticks at 20 Hz).
pub fn frame_eye(view: &ViewState, partial: f32, input_look: (f32, f32)) -> (DVec3, (f32, f32)) {
    match &view.camera_entity {
        Some(c) => {
            let (eye, yaw, pitch) = c.entity.camera_at(partial, c.eye_heights);
            (eye, (yaw, pitch))
        }
        None => (
            super::interpolate(view.prev_eye, view.eye, partial),
            (
                input_look.0 + view.look_offset.0,
                (input_look.1 + view.look_offset.1).clamp(-90.0, 90.0),
            ),
        ),
    }
}

/// The entity the world pass must not draw this frame (vanilla `LevelExtractor
/// .extractVisibleEntities`): the camera entity while the camera is not detached (first
/// person), nothing in the third-person perspectives. `local` is the player's own id.
pub fn hidden_entity(view: &ViewState, first_person: bool, local: EntityId) -> Option<EntityId> {
    hidden_camera_entity(view.camera_entity.as_ref(), first_person, local)
}

/// [`hidden_entity`] for the view's camera entity (`None`: the player is the camera).
fn hidden_camera_entity(
    camera: Option<&CameraEntityView>,
    first_person: bool,
    local: EntityId,
) -> Option<EntityId> {
    first_person.then(|| camera.map_or(local, |c| c.entity.id))
}

/// While another entity is the camera entity, the view's camera tilt and bob inputs become
/// that entity's (vanilla `Camera.extractRenderState` fills `entityRenderState` from the
/// camera entity; `GameRenderer.bobHurt` tilts only for a living one and `bobView` bobs only
/// for a player). The hurt direction is 0 (`LivingEntity.getHurtDir`; only the local player
/// learns its own). A spectated remote player's walk bob is not modelled: it does not bob.
pub(super) fn apply_camera_tilt(view: &mut ViewState) {
    let Some(c) = view.camera_entity else {
        return;
    };
    let tilt = c.entity.tilt.unwrap_or_default();
    view.hurt_ticks = tilt.hurt_time;
    view.hurt_dir = 0.0;
    view.dead = tilt.death_time.is_some();
    view.death_ticks = tilt.death_time.unwrap_or(0);
    view.walk_dist = 0.0;
    view.walk_dist_o = 0.0;
    view.bob = 0.0;
    view.bob_o = 0.0;
}

/// Whether the local player's own model is drawn (vanilla `LevelExtractor
/// .extractVisibleEntities` skips a `LocalPlayer` that is not the camera entity).
pub fn draws_local_player(view: &ViewState) -> bool {
    view.camera_entity.is_none()
}

#[cfg(test)]
mod tests;
