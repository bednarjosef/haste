//! Whether an entity is a baby (`AgeableMob.isBaby` and the same-slot flags), for the baby
//! models and the walk swing (see `registry`). Moved out of `registry.rs` to keep that file
//! within the size limit; a pure move.

use haste_core::entity::MetadataValue;

/// The metadata index of `AgeableMob.DATA_BABY_ID` (and `Zombie`/`Zoglin`/`Piglin`'s own
/// `DATA_BABY_ID`, which lands on the same slot): `Entity` defines indices 0-7, `LivingEntity`
/// 8-14, `Mob` 15, and `AgeableMob` appends its boolean at 16 (Entity.java:323-330,
/// LivingEntity.java:323-331, Mob.java:270-273, AgeableMob.java:105-107 in the decompiled
/// source; each class's `defineSynchedData` calls `super` first, then appends).
pub const BABY_METADATA_INDEX: u8 = 16;

/// Whether the entity's metadata says it is a baby: `AgeableMob.DATA_BABY_ID` (index 16) or
/// the same-slot flags `Zombie`/`Zoglin`/`Piglin` define. Types without any baby flag are
/// never babies.
pub fn is_baby(metadata: &crate::metadata::Metadata) -> bool {
    matches!(
        metadata.get(BABY_METADATA_INDEX).map(|e| &e.value),
        Some(MetadataValue::Boolean(true))
    )
}
