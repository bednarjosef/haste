//! Player attributes and pose dimensions. Defaults and their sources are listed in `NOTES.md`.
use serde::{Deserialize, Serialize};

/// Body pose as far as physics cares (subset of vanilla's 18 poses).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Pose {
    #[default]
    Standing,
    Crouching,
    Swimming,
    /// Gliding (`Player.getDesiredPose` returns `Pose.FALL_FLYING` while the shared flag 7 is
    /// set): 0.6 x 0.6, eye height 0.4 (`Avatar`'s pose dimensions).
    FallFlying,
}

impl Pose {
    /// Serialized pose name as in `entity_dimensions.json` and the traces.
    pub fn name(self) -> &'static str {
        match self {
            Pose::Standing => "standing",
            Pose::Crouching => "crouching",
            Pose::Swimming => "swimming",
            Pose::FallFlying => "fall_flying",
        }
    }
    pub fn from_name(s: &str) -> Option<Pose> {
        Some(match s {
            "standing" => Pose::Standing,
            "crouching" => Pose::Crouching,
            "swimming" => Pose::Swimming,
            "fall_flying" => Pose::FallFlying,
            _ => return None,
        })
    }
}

/// Attribute values the tick needs. `Default` is the vanilla player with no modifiers.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct PlayerAttributes {
    /// `minecraft:movement_speed` base value without the sprint modifier (wiki "Attribute": player 0.1).
    pub movement_speed: f64,
    /// `minecraft:jump_strength` (wiki "Attribute": 0.42).
    pub jump_strength: f64,
    /// `minecraft:gravity` (wiki "Attribute": 0.08).
    pub gravity: f64,
    /// `minecraft:step_height` (wiki "Attribute": 0.6).
    pub step_height: f64,
    /// `minecraft:sneaking_speed` (wiki "Attribute": 0.3).
    pub sneaking_speed: f64,
    /// Creative flight speed (player ability, not an attribute). 0.05: derived so that the
    /// steady flight speed matches the wiki's ~10.92 m/s ("Flying"); see NOTES.md.
    pub flying_speed: f32,
    /// Standing box (entity_dimensions.json "standing": 0.6 x 1.8).
    pub width: f32,
    pub height: f32,
    /// entity_dimensions.json "crouching": height 1.5.
    pub crouching_height: f32,
    /// entity_dimensions.json "swimming": height 0.6.
    pub swimming_height: f32,
    /// `minecraft:scale` (wiki "Attribute": 1.0); multiplies the box and the eye height.
    pub scale: f32,
}

impl Default for PlayerAttributes {
    fn default() -> Self {
        Self {
            movement_speed: 0.1,
            jump_strength: 0.42,
            gravity: 0.08,
            step_height: 0.6,
            sneaking_speed: 0.3,
            flying_speed: 0.05,
            width: 0.6,
            height: 1.8,
            crouching_height: 1.5,
            swimming_height: 0.6,
            scale: 1.0,
        }
    }
}

impl PlayerAttributes {
    /// Eye height per pose (entity_dimensions.json: standing 1.62, crouching 1.27, swimming 0.4,
    /// fall_flying 0.4). Scaled by the `scale` attribute (in f32, as entity dimensions are floats).
    pub fn eye_height_for(&self, pose: Pose) -> f64 {
        let eye: f32 = match pose {
            Pose::Standing => 1.62,
            Pose::Crouching => 1.27,
            Pose::Swimming => 0.4,
            Pose::FallFlying => 0.4,
        };
        (eye * self.scale) as f64
    }

    /// Box height for a pose, scaled by the `scale` attribute. The fall-flying pose is the same
    /// 0.6 x 0.6 as swimming (`Avatar`'s pose dimensions), so it shares `swimming_height`.
    pub fn height_for(&self, pose: Pose) -> f64 {
        let h = match pose {
            Pose::Standing => self.height,
            Pose::Crouching => self.crouching_height,
            Pose::Swimming | Pose::FallFlying => self.swimming_height,
        };
        (h * self.scale) as f64
    }

    /// Box width, scaled by the `scale` attribute.
    pub fn box_width(&self) -> f64 {
        (self.width * self.scale) as f64
    }
}
