//! The `minecraft:consumable` and `minecraft:food` item components.

/// An item's `minecraft:consumable` component (`world.item.component.Consumable`): how long the
/// item is used and the animation played while it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Consumable {
    /// `consume_seconds` (1.6 by default).
    pub consume_seconds: f32,
    /// `ItemUseAnimation` id (`animation`, `eat` by default).
    pub animation: i32,
}

impl Consumable {
    /// `Consumable.consumeTicks`: the use duration in ticks.
    pub fn consume_ticks(&self) -> i32 {
        (self.consume_seconds * 20.0) as i32
    }
}

/// Parses `minecraft:consumable` (`Consumable.CODEC`): `consume_seconds` and `animation` with
/// their defaults.
pub(crate) fn parse_consumable(v: &serde_json::Value) -> Consumable {
    Consumable {
        consume_seconds: v
            .get("consume_seconds")
            .and_then(|s| s.as_f64())
            .unwrap_or(1.6) as f32,
        animation: use_animation_id(v.get("animation").and_then(|a| a.as_str())),
    }
}

/// `ItemUseAnimation` name -> id.
fn use_animation_id(name: Option<&str>) -> i32 {
    match name {
        Some("none") => 0,
        Some("drink") => 2,
        Some("block") => 3,
        Some("bow") => 4,
        Some("trident") => 5,
        Some("crossbow") => 6,
        Some("spyglass") => 7,
        Some("toot_horn") => 8,
        Some("brush") => 9,
        Some("bundle") => 10,
        Some("spear") => 11,
        // `eat` and any unknown name are vanilla's default.
        _ => 1,
    }
}
