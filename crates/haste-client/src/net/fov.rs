// The FOV modifier of the camera tick (vanilla `AbstractClientPlayer.getFieldOfViewModifier`):
// which held items change it while in use, and the target value it smooths towards.

use haste_core::{Identifier, RegistryIndex};
use haste_data::GameData;

/// An item whose use affects the FOV (vanilla's `getFieldOfViewModifier` only looks at the
/// bow and the spyglass; every other use item leaves the modifier alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseItemKind {
    Bow,
    Spyglass,
}

impl UseItemKind {
    /// Classifies the held stack by `minecraft:item` registry id; `Other` for everything
    /// else (and whenever the game data is not at hand, e.g. the recording test fakes).
    pub fn classify(data: Option<&GameData>, item: RegistryIndex) -> Option<Self> {
        let data = data?;
        let id = |name: &str| {
            data.registries
                .index("minecraft:item", &Identifier::minecraft(name))
        };
        if Some(item) == id("bow") {
            Some(Self::Bow)
        } else if Some(item) == id("spyglass") {
            Some(Self::Spyglass)
        } else {
            None
        }
    }
}

/// `AbstractClientPlayer.getFieldOfViewModifier` (vanilla): the per-tick FOV modifier from
/// the movement speed against the walking speed, the flying bonus, the bow draw and the
/// spyglass, scaled by the `fovEffectScale` option. `movement_speed` is the attribute value
/// *with* the sprinting modifier (vanilla reads the attribute with it).
pub fn fov_modifier_target(
    flying: bool,
    movement_speed: f64,
    walking_speed: f32,
    use_item: Option<UseItemKind>,
    use_ticks: u32,
    first_person: bool,
    effect_scale: f32,
) -> f32 {
    let mut modifier: f64 = 1.0;
    if flying {
        modifier *= 1.1;
    }
    if walking_speed != 0.0 {
        let speed_factor = movement_speed / walking_speed as f64;
        modifier *= (speed_factor + 1.0) / 2.0;
    }
    if let Some(kind) = use_item {
        if kind == UseItemKind::Bow {
            let scale = (use_ticks as f32 / 20.0).min(1.0) as f64;
            modifier *= 1.0 - scale * scale * 0.15;
        } else if first_person && kind == UseItemKind::Spyglass {
            // Vanilla returns before the effect-scale lerp: scoping is a full 0.1.
            return 0.1;
        }
    }
    (1.0 + (modifier - 1.0) * effect_scale as f64) as f32
}
