//! The dragon fireball's entity-side mapping (`DragonFireballRenderer`): the renderer keeps
//! only the base `EntityRenderState`, so the decoration carries no data of its own.

use haste_core::RegistryIndex;

/// Whether a tracked entity is a `minecraft:dragon_fireball` (drawn through
/// `DragonFireballRenderer`, not the thrown-item sprite).
pub fn is_dragon_fireball(kind: RegistryIndex) -> bool {
    crate::registry::entity_type_name(kind) == Some("minecraft:dragon_fireball")
}
