//! Powder snow: the predicted freeze ticks and the `Hud` freeze outline's texture.
use super::FREEZE_TICKS;
use haste_protocol::haste_core::Identifier;
/// Vanilla's `Hud.POWDER_SNOW_OUTLINE_LOCATION`.
pub(super) fn powder_snow_outline() -> Identifier {
    Identifier::minecraft("textures/misc/powder_snow_outline.png")
}

/// Whether `block` is powder snow (`PowderSnowBlock` applies the `FREEZE` inside-block effect).
pub(super) fn is_powder_snow_block(
    block: &Identifier,
    blocks: &haste_data::BlockStates,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> bool {
    if block.namespace() == "minecraft" && block.path() == "powder_snow" {
        return true;
    }
    // `BlockTags.POWDER_SNOW` (`minecraft:powder_snow`) covers it and any modded snow.
    match (tags, blocks.block_index(block)) {
        (Some(tags), Some(index)) => tags.contains(&Identifier::minecraft("powder_snow"), index.0),
        _ => false,
    }
}

/// One tick of the local player's freeze ticks, as the client predicts them: powder snow's
/// `FREEZE` adds one (`InsideBlockEffectType.FREEZE`, capped at `getTicksRequiredToFreeze`),
/// fire's `CLEAR_FREEZE` zeroes them, and outside powder snow the server's rule eases them out
/// by two per tick (`LivingEntity.aiStep`). The server never sends the owner their own
/// `set_entity_data` (`ChunkMap.TrackedEntity.updatePlayer` skips self), so Haste predicts it.
pub fn tick_frozen(current: i32, in_powder_snow: bool, in_fire: bool) -> i32 {
    if in_fire {
        0
    } else if in_powder_snow {
        (current + 1).min(FREEZE_TICKS)
    } else {
        (current - 2).max(0)
    }
}

/// Vanilla's `Entity.getPercentFrozen`.
pub fn percent_frozen(ticks_frozen: i32) -> f32 {
    (ticks_frozen.min(FREEZE_TICKS) as f32) / FREEZE_TICKS as f32
}
