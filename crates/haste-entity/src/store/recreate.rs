//! A tracked entity built from its spawn packet: vanilla `EntityType.create` followed by
//! `Entity.recreateFromPacket`. Moved out of `store.rs` to keep that file within the size
//! limit; a pure move.

use haste_core::entity::{EntitySpawn, Pose};

use super::Entity;
use crate::dims::EntityDimensions;
use crate::lerp::LerpState;
use crate::metadata::Metadata;
use crate::walk_swing::{SwingState, WalkAnimationState};

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
            // `Display.createInterpolationHandler`: a zero-step window until the synched
            // teleport duration sets one.
            lerp: crate::display::lerp_state(s.kind, LerpState::new(s.pos, s.yaw, s.pitch)),
            new_minecart: None,
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
            fangs: Default::default(),
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
            display: crate::display::DisplayState::for_kind(s.kind).map(Box::new),
        }
    }
}
