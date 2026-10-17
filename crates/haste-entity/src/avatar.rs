//! The player-only render state: the model customisation byte (`Avatar.isModelPartShown`,
//! `PlayerModelPart`), the outer layers it hides (`PlayerModel.setupAnim`), the cape's trailing
//! "cloak" position (`ClientAvatarState`) and the cape's swing (`AvatarRenderer.extractCapeState`
//! and `PlayerCapeModel.setupAnim`).
//!
//! Both the tracked players (`EntityStore::render_instances`) and the local player's own model
//! go through [`dress`], so the two never drift apart.
use glam::{DVec3, EulerRot, Quat, Vec3};
use haste_core::entity::MetadataValue;
use haste_core::model::{EntityInstance, EntityOverlay, PoseOverrides};

use crate::skin::PlayerSkin;

/// `Avatar.DATA_PLAYER_MODE_CUSTOMISATION`'s metadata index (`Entity`'s 8 slots, `LivingEntity`'s
/// 7, then `Avatar`'s main arm at 15 and this byte at 16).
pub const DATA_PLAYER_MODE_CUSTOMISATION: u8 = 16;

/// `PlayerModelPart`, by bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerModelPart {
    Cape = 0,
    Jacket = 1,
    LeftSleeve = 2,
    RightSleeve = 3,
    LeftPantsLeg = 4,
    RightPantsLeg = 5,
    Hat = 6,
}

/// The model customisation byte (the client information's displayed skin parts, synced as
/// entity data).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ModelParts(pub u8);

impl ModelParts {
    /// Every part shown (what Haste's client information sends).
    pub const ALL: ModelParts = ModelParts(0x7f);

    /// The byte from an avatar's metadata; `Avatar.defineSynchedData`'s default 0 (nothing
    /// shown) until the server sends it.
    pub fn of(metadata: &crate::metadata::Metadata) -> ModelParts {
        match metadata
            .get(DATA_PLAYER_MODE_CUSTOMISATION)
            .map(|e| &e.value)
        {
            Some(MetadataValue::Byte(b)) => ModelParts(*b as u8),
            _ => ModelParts(0),
        }
    }

    /// `Avatar.isModelPartShown`.
    pub fn shown(self, part: PlayerModelPart) -> bool {
        let mask = 1u8 << part as u8;
        self.0 & mask == mask
    }
}

/// `PlayerModel.setupAnim`: hides the outer layers whose part is not shown. Nothing is pushed
/// (so nothing allocated) when everything shows, the normal case.
pub fn hide_layers(parts: ModelParts, pose: &mut PoseOverrides) {
    for (part, name) in [
        (PlayerModelPart::Hat, "hat"),
        (PlayerModelPart::Jacket, "jacket"),
        (PlayerModelPart::LeftPantsLeg, "left_pants"),
        (PlayerModelPart::RightPantsLeg, "right_pants"),
        (PlayerModelPart::LeftSleeve, "left_sleeve"),
        (PlayerModelPart::RightSleeve, "right_sleeve"),
    ] {
        if !parts.shown(part) {
            pose.hidden.push(name.to_owned());
        }
    }
}

/// `ClientAvatarState`'s cloak: a point that trails the player by a quarter of the distance each
/// tick and snaps to it when more than 10 blocks away on an axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cloak {
    pub pos: DVec3,
    pub pos_o: DVec3,
}

impl Cloak {
    /// `ClientAvatarState.moveCloak`, called from `ClientAvatarState.tick` with the position at
    /// the start of the tick (`AbstractClientPlayer.tick` runs it before the movement).
    pub fn tick(&mut self, pos: DVec3) {
        self.pos_o = self.pos;
        let axis = |cloak: &mut f64, cloak_o: &mut f64, target: f64| {
            let d = target - *cloak;
            if (-10.0..=10.0).contains(&d) {
                *cloak += d * 0.25;
            } else {
                *cloak = target;
                *cloak_o = target;
            }
        };
        axis(&mut self.pos.x, &mut self.pos_o.x, pos.x);
        axis(&mut self.pos.y, &mut self.pos_o.y, pos.y);
        axis(&mut self.pos.z, &mut self.pos_o.z, pos.z);
    }

    /// `getInterpolatedCloakX/Y/Z`.
    pub fn at(&self, partial: f32) -> DVec3 {
        self.pos_o.lerp(self.pos, partial as f64)
    }
}

/// What `AvatarRenderer.extractCapeState` reads, already interpolated by the partial tick.
#[derive(Clone, Copy, Debug)]
pub struct CapeInput {
    /// The interpolated cloak position.
    pub cloak: DVec3,
    /// The interpolated entity position.
    pub pos: DVec3,
    /// The interpolated body yaw, degrees.
    pub body_yaw: f32,
    /// `AvatarRenderState.fallFlyingScale`.
    pub fall_flying_scale: f32,
    /// `ClientAvatarState.getInterpolatedBob` (0 for remote players off the ground).
    pub bob: f32,
    /// `ClientAvatarState.getInterpolatedWalkDistance` (remote players never add any).
    pub walk_dist: f32,
}

/// `AvatarRenderState`'s cape angles, degrees: flap, lean, sideways lean.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CapeState {
    pub flap: f32,
    pub lean: f32,
    pub lean2: f32,
}

/// `AvatarRenderer.extractCapeState`.
pub fn cape_state(i: &CapeInput) -> CapeState {
    let d = i.cloak - i.pos;
    let yaw = i.body_yaw.to_radians();
    let forward_x = yaw.sin() as f64;
    let forward_z = -yaw.cos() as f64;
    let flap = (d.y as f32 * 10.0).clamp(-6.0, 32.0);
    let lean = ((d.x * forward_x + d.z * forward_z) as f32 * 100.0 * (1.0 - i.fall_flying_scale))
        .clamp(0.0, 150.0);
    let lean2 = ((d.x * forward_z - d.z * forward_x) as f32 * 100.0).clamp(-20.0, 20.0);
    CapeState {
        flap: flap + (i.walk_dist * 6.0).sin() * 32.0 * i.bob,
        lean,
        lean2,
    }
}

/// The cape part's additive rotation for `PoseOverrides` (the renderer adds it to the part's
/// base rotation, (0, pi, 0) in the layer). `PlayerCapeModel.setupAnim` composes the base with
/// Y(-pi) X(6 + lean/2 + flap) Z(lean2/2) Y(180 - lean2/2) (`ModelPart.rotateBy`) and stores the
/// result as ZYX Euler angles; this returns those angles minus the base.
pub fn cape_rotation(s: &CapeState) -> Vec3 {
    let base = Quat::from_rotation_y(std::f32::consts::PI);
    let q = base
        * Quat::from_rotation_y(-std::f32::consts::PI)
        * Quat::from_rotation_x((6.0 + s.lean / 2.0 + s.flap).to_radians())
        * Quat::from_rotation_z((s.lean2 / 2.0).to_radians())
        * Quat::from_rotation_y((180.0 - s.lean2 / 2.0).to_radians());
    let (z, y, x) = q.to_euler(EulerRot::ZYX);
    Vec3::new(x, y - std::f32::consts::PI, z)
}

/// Dresses a player instance in its skin, the way `AvatarRenderer` draws it: the wide or slim
/// model with the skin texture, the outer layers the customisation byte hides, and the cape
/// layer when the skin has a cape and the cape part is shown (`CapeLayer.submit`; not while
/// invisible). `cape` is this frame's cape swing.
pub fn dress(inst: &mut EntityInstance, skin: &PlayerSkin, parts: ModelParts, cape: &CapeState) {
    inst.model = crate::models::humanoid(skin.model.is_slim());
    inst.skin = Some(skin.body.clone());
    hide_layers(parts, &mut inst.pose);
    if let Some(cape_model) = &skin.cape {
        if parts.shown(PlayerModelPart::Cape) && !inst.invisible {
            inst.pose
                .rotations
                .push(("cape".into(), cape_rotation(cape)));
            inst.overlays[0] = Some(EntityOverlay {
                model: cape_model.clone(),
                tint: [1.0; 3],
            });
        }
    }
}

/// [`dress`] for a tracked player (a `RemotePlayer`): its customisation byte from its entity
/// data and its cape swing from its own cloak. Remote players never walk through `move`, so
/// their walk distance stays 0 and the bob term drops out (`ClientAvatarState` only gains walk
/// distance in `LocalPlayer.move`).
pub(crate) fn dress_tracked(
    skin: &PlayerSkin,
    e: &crate::store::Entity,
    inst: &mut EntityInstance,
    partial: f32,
) {
    let cape = if skin.cape.is_some() {
        let t = e.fall_flying_ticks as f32 + partial;
        cape_state(&CapeInput {
            cloak: e.cloak.at(partial),
            pos: e.prev_pos.lerp(e.pos, partial as f64),
            body_yaw: inst.yaw,
            fall_flying_scale: (t * t / 100.0).clamp(0.0, 1.0),
            bob: 0.0,
            walk_dist: 0.0,
        })
    } else {
        CapeState::default()
    };
    dress(inst, skin, ModelParts::of(&e.metadata), &cape);
}
