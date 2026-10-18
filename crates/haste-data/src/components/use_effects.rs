//! The `minecraft:use_effects` item component.

/// An item's `minecraft:use_effects` component (`world.item.component.UseEffects`): what
/// using the item does to the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UseEffects {
    /// `can_sprint` (false by default): using the item does not keep a sprint from starting
    /// (vanilla `LocalPlayer.isSlowDueToUsingItem` reads it).
    pub can_sprint: bool,
    /// `interact_vibrations` (true by default): using the item causes the game-event
    /// vibration (sculk) of the use.
    pub interact_vibrations: bool,
    /// `speed_multiplier` (0.2 by default): scales the movement input while using the item
    /// (vanilla `LocalPlayer.itemUseSpeedMultiplier`).
    pub speed_multiplier: f32,
}

impl UseEffects {
    /// `UseEffects.DEFAULT`, which every item's prototype carries
    /// (`DataComponents.COMMON_ITEM_COMPONENTS`).
    pub const DEFAULT: UseEffects = UseEffects {
        can_sprint: false,
        interact_vibrations: true,
        speed_multiplier: 0.2,
    };
}

/// Parses `minecraft:use_effects` (`UseEffects.CODEC`): `can_sprint`, `interact_vibrations`
/// and `speed_multiplier` with their defaults.
pub(crate) fn parse_use_effects(v: &serde_json::Value) -> UseEffects {
    UseEffects {
        can_sprint: v
            .get("can_sprint")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        interact_vibrations: v
            .get("interact_vibrations")
            .and_then(|b| b.as_bool())
            .unwrap_or(true),
        speed_multiplier: v
            .get("speed_multiplier")
            .and_then(|s| s.as_f64())
            .unwrap_or(0.2) as f32,
    }
}
