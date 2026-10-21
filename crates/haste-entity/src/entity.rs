//! `Entity`'s constructor and its per-entity reads (yaw, pitch, body and head yaw between ticks).
//! The struct itself lives in `store.rs` next to the slots that hold it.

use haste_core::entity::{EntitySpawn, Pose};
use haste_core::DVec3;

use crate::dims::EntityDimensions;
use crate::lerp::LerpState;
use crate::metadata::Metadata;
use crate::store::{Entity, DEATH_TICKS};
use crate::walk_swing::{lerp, rot_lerp, wrap_degrees, SwingState, WalkAnimationState};

impl Entity {
    pub(crate) fn new(s: &EntitySpawn, dims: &EntityDimensions) -> Entity {
        let type_name = crate::registry::entity_type_name(s.kind);
        let living = if s.kind.0 == 0 {
            true
        } else {
            match type_name {
                None => true,
                Some(name) => {
                    crate::registry::table_entry(name).is_some()
                        || name == "minecraft:player"
                        || name == "minecraft:mannequin"
                        || name == "minecraft:armor_stand"
                }
            }
        };
        // `ItemEntity`: simulated client-side; `set_metadata` confirms it at index 8.
        let is_item = type_name == Some("minecraft:item");
        let is_tnt = type_name == Some("minecraft:tnt");
        // The `AbstractBoat` family (vanilla's or a mod's): the client ticks its paddle and
        // bubble-column state (`crate::boat`).
        let is_boat = type_name.is_some_and(crate::boat::is_boat_family);
        Entity {
            id: s.id,
            uuid: s.uuid,
            kind: s.kind,
            pos: s.pos,
            prev_pos: s.pos,
            data: s.data,
            yaw: s.yaw,
            pitch: s.pitch,
            prev_yaw: s.yaw,
            prev_pitch: s.pitch,
            head_yaw: s.head_yaw,
            velocity: s.velocity,
            on_ground: false,
            pose: Pose::Standing,
            dimensions: dims.get(s.kind, Pose::Standing),
            metadata: Metadata::default(),
            equipment: Vec::new(),
            lerp: LerpState::new(s.pos, s.yaw, s.pitch),
            nametag: None,
            is_player_type: false,
            walk: WalkAnimationState::default(),
            // `LivingEntity.recreateFromPacket`: the body starts facing the packet's head yaw
            // (`yBodyRot = yHeadRot = getYHeadRot()`, the `O` copies the same), not its yaw.
            body_yaw: if living { s.head_yaw } else { s.yaw },
            prev_body_yaw: if living { s.head_yaw } else { s.yaw },
            prev_head_yaw: s.head_yaw,
            lerp_head_yaw: s.head_yaw,
            lerp_head_steps: 0,
            swing: SwingState::default(),
            swim_amount: 0.0,
            swim_amount_o: 0.0,
            fall_flying_ticks: 0,
            tick_count: 0,
            flap: 0.0,
            o_flap: 0.0,
            flap_speed: 0.0,
            o_flap_speed: 0.0,
            flapping: 1.0,
            hurt_ticks: 0,
            dead: false,
            swell: 0,
            old_swell: 0,
            attack_ticks: 0,
            last_hit: -1000,
            item: None,
            is_item,
            is_boat,
            boat: crate::boat::BoatState::default(),
            age: 0,
            cloak: crate::avatar::Cloak::default(),
            is_living: living,
            is_tnt,
            using_item: crate::using_item::UsingItem::default(),
            shake_time: 0,
            arrow_in_ground: false,
            in_ground_time: 0,
            spectral: type_name == Some("minecraft:spectral_arrow"),
        }
    }

    /// `LivingEntity.isDeadOrDying` became true: `deathTime` starts counting on the next tick
    /// (`hurt_ticks` runs the 20-tick death counter down; see `animation::death_time`). A
    /// second trigger (event 3 after the health metadata, or the reverse) changes nothing.
    pub(crate) fn start_death(&mut self) {
        if !self.dead {
            self.dead = true;
            self.hurt_ticks = DEATH_TICKS;
        }
    }

    /// `Entity.getYRot(partialTicks)`: `Mth.rotLerp(partial, yRotO, yRot)`.
    pub fn yaw_at(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.yaw
        } else {
            rot_lerp(partial, self.prev_yaw, self.yaw)
        }
    }

    /// `Entity.getXRot(partialTicks)`: `Mth.lerp(partial, xRotO, xRot)`.
    pub fn pitch_at(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.pitch
        } else {
            lerp(partial, self.prev_pitch, self.pitch)
        }
    }

    /// The present (pos, yaw, pitch): what `LerpState` keeps for the part a packet lacks.
    pub(crate) fn current(&self) -> (DVec3, f32, f32) {
        (self.pos, self.yaw, self.pitch)
    }

    /// Body yaw between the last two ticks (`Mth.rotLerp(partial, yBodyRotO, yBodyRot)`).
    pub fn body_yaw_at(&self, partial: f32) -> f32 {
        self.prev_body_yaw + partial * wrap_degrees(self.body_yaw - self.prev_body_yaw)
    }

    /// Head yaw between the last two ticks (`Mth.rotLerp(partial, yHeadRotO, yHeadRot)`).
    pub fn head_yaw_at(&self, partial: f32) -> f32 {
        self.prev_head_yaw + partial * wrap_degrees(self.head_yaw - self.prev_head_yaw)
    }
}
