//! The synced data `StuckInBodyLayer`, `ParrotOnShoulderLayer` read off a living entity: the
//! arrows and stingers stuck in its body (`LivingEntity.DATA_ARROW_COUNT` and
//! `DATA_STINGER_COUNT`, indices 12 and 13) and, for a player, the parrots on its shoulders
//! (`Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT`, indices 19 and 20). The pose inputs the
//! layers take from the render state (head angles, walk cycle, crouching) ride along.

use std::sync::{Arc, OnceLock};

use haste_core::entity::MetadataValue;
use haste_core::model::CubeModel;
use haste_core::EntityId;

use crate::metadata::Metadata;
use crate::model_dump;
use crate::store::{Entity, EntityStore};
use crate::walk_swing::wrap_degrees;

/// `BeeStingerModel`'s layer id in the model library.
const STINGER_MODEL: &str = "minecraft:bee_stinger#main";
/// `ParrotModel`'s layer id in the model library.
const PARROT_MODEL: &str = "minecraft:parrot#main";

/// `LivingEntity.DATA_ARROW_COUNT`.
const ARROW_COUNT: u8 = 12;
/// `LivingEntity.DATA_STINGER_COUNT`.
const STINGER_COUNT: u8 = 13;
/// `Player.DATA_SHOULDER_PARROT_LEFT` / `_RIGHT`.
const SHOULDER_PARROT: [u8; 2] = [19, 20];
/// The serializer id of `EntityDataSerializers.OPTIONAL_UNSIGNED_INT` in the 26.3 registration
/// order. Haste's table names this id `OPTIONAL_VAR_INT` (same wire form), so the value is kept
/// as its raw bytes and read here.
const OPTIONAL_UNSIGNED_INT: i32 = 19;

/// What one living entity shows on its body this frame, with the render-state inputs the
/// stuck items and shoulder parrots are posed from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuckState {
    pub id: EntityId,
    pub arrows: u32,
    pub stingers: u32,
    /// The parrot variant (0..=4) on the left and right shoulder, `None` when empty.
    pub parrots: [Option<u32>; 2],
    pub crouching: bool,
    /// The head's yaw relative to the body, in radians (`netHeadYaw`), and its pitch.
    pub head_yaw: f32,
    pub head_pitch: f32,
    /// The walk cycle's position and speed (`AvatarRenderState.walkAnimationPos` / `Speed`).
    pub walk_pos: f32,
    pub walk_speed: f32,
}

/// The two models the stuck layers draw besides the body's own: `BeeStingerModel` and
/// `ParrotModel`, at their own pixel scale. `None` when the model dump lacks one.
#[derive(Clone, Debug, Default)]
pub struct StuckModels {
    pub stinger: Option<Arc<CubeModel>>,
    pub parrot: Option<Arc<CubeModel>>,
}

/// The stuck layers' models, resolved once from the model library (cheap to call per frame:
/// it clones two `Arc`s).
pub fn stuck_models() -> StuckModels {
    static MODELS: OnceLock<StuckModels> = OnceLock::new();
    MODELS
        .get_or_init(|| {
            let lib = model_dump::library();
            StuckModels {
                stinger: lib.and_then(|l| l.get(STINGER_MODEL, 1.0)),
                parrot: lib.and_then(|l| l.get(PARROT_MODEL, 1.0)),
            }
        })
        .clone()
}

impl EntityStore {
    /// Fills `out` with the stuck state of every living entity that shows one, sorted by id
    /// (the renderer looks entities up by binary search). Reuses `out`'s buffer.
    pub fn stuck_states(&self, partial: f32, out: &mut Vec<StuckState>) {
        out.clear();
        for e in self.slots.iter().flatten() {
            if !e.is_living {
                continue;
            }
            let state = stuck_state(e, partial);
            if state.arrows > 0 || state.stingers > 0 || state.parrots != [None, None] {
                out.push(state);
            }
        }
        out.sort_unstable_by_key(|s| s.id.0);
    }
}

fn stuck_state(e: &Entity, partial: f32) -> StuckState {
    let m = &e.metadata;
    let parrots = if e.is_player_type {
        SHOULDER_PARROT.map(|index| optional_unsigned_int(m, index))
    } else {
        [None, None]
    };
    StuckState {
        id: e.id,
        arrows: var_int(m, ARROW_COUNT),
        stingers: var_int(m, STINGER_COUNT),
        parrots,
        crouching: m.shared.crouching,
        head_yaw: wrap_degrees(e.head_yaw_at(partial) - e.yaw_at(partial)).to_radians(),
        head_pitch: e.pitch_at(partial).to_radians(),
        walk_pos: e.walk.position(partial),
        walk_speed: e.walk.speed(partial),
    }
}

/// A VarInt entry, zero when absent (vanilla's default for both counts).
pub(crate) fn var_int(m: &Metadata, index: u8) -> u32 {
    match m.get(index).map(|e| &e.value) {
        Some(MetadataValue::VarInt(v)) => (*v).max(0) as u32,
        _ => 0,
    }
}

/// An `OPTIONAL_UNSIGNED_INT` entry: zero on the wire is empty, `v + 1` is the value `v`.
pub(crate) fn optional_unsigned_int(m: &Metadata, index: u8) -> Option<u32> {
    match m.get(index).map(|e| &e.value) {
        Some(MetadataValue::Other { type_id, raw }) if *type_id == OPTIONAL_UNSIGNED_INT => {
            let v = var_int_bytes(raw)?;
            v.checked_sub(1)
        }
        _ => None,
    }
}

/// Reads one VarInt from the front of `raw`; `None` when it is truncated or longer than five
/// bytes.
pub(crate) fn var_int_bytes(raw: &[u8]) -> Option<u32> {
    let mut v = 0u32;
    for (i, b) in raw.iter().take(5).enumerate() {
        v |= ((b & 0x7f) as u32) << (7 * i);
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}
