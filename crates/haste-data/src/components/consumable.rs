//! The `minecraft:consumable` and `minecraft:food` item components.

use haste_core::Identifier;

/// An item's `minecraft:consumable` component (`world.item.component.Consumable`): how long
/// the item is used, the animation played while it is, the sound and particles the use
/// emits.
#[derive(Clone, Debug, PartialEq)]
pub struct Consumable {
    /// `consume_seconds` (1.6 by default).
    pub consume_seconds: f32,
    /// `ItemUseAnimation` id (`animation`, `eat` by default).
    pub animation: i32,
    /// `sound`: the sound event `Consumable.emitParticlesAndSounds` plays while using and
    /// on finish (`entity.generic.eat` by default).
    pub sound: Identifier,
    /// `has_consume_particles`: whether the use spawns item particles (true by default).
    pub has_consume_particles: bool,
}

impl Consumable {
    /// `Consumable.consumeTicks`: the use duration in ticks.
    pub fn consume_ticks(&self) -> i32 {
        (self.consume_seconds * 20.0) as i32
    }

    /// `Consumables.defaultFood`: the eat animation, the generic eat sound, particles on.
    pub fn default_food() -> Consumable {
        Consumable {
            consume_seconds: 1.6,
            animation: 1,
            sound: generic_eat(),
            has_consume_particles: true,
        }
    }

    /// `Consumables.defaultDrink`: the drink animation, the generic drink sound, no
    /// particles.
    pub fn default_drink() -> Consumable {
        Consumable {
            animation: 2,
            sound: Identifier::minecraft("entity.generic.drink"),
            has_consume_particles: false,
            ..Self::default_food()
        }
    }
}

/// The default consume sound (`SoundEvents.GENERIC_EAT`).
pub(crate) fn generic_eat() -> Identifier {
    Identifier::minecraft("entity.generic.eat")
}

/// Parses `minecraft:consumable` (`Consumable.CODEC`): `consume_seconds`, `animation`,
/// `sound` and `has_consume_particles` with their defaults (the report names only what
/// differs: bread's whole component is `{}`).
pub(crate) fn parse_consumable(v: &serde_json::Value) -> Consumable {
    Consumable {
        consume_seconds: v
            .get("consume_seconds")
            .and_then(|s| s.as_f64())
            .unwrap_or(1.6) as f32,
        animation: use_animation_id(v.get("animation").and_then(|a| a.as_str())),
        sound: v
            .get("sound")
            .and_then(parse_sound_id)
            .unwrap_or_else(generic_eat),
        has_consume_particles: v
            .get("has_consume_particles")
            .and_then(|b| b.as_bool())
            .unwrap_or(true),
    }
}

/// A `sound` field: a registry id string (`minecraft:entity.generic.eat`), or an inline
/// sound object naming `sound_id` (the shape `SoundEvent.DIRECT_CODEC` writes).
fn parse_sound_id(v: &serde_json::Value) -> Option<Identifier> {
    let name = match v {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Object(o) => o.get("sound_id")?.as_str()?,
        _ => return None,
    };
    // Namespaced ids parse as they are; a bare path is `minecraft:`'s (the default
    // namespace vanilla's `Identifier.parse` assumes).
    Identifier::parse(name)
        .or_else(|_| Identifier::parse(&format!("minecraft:{name}")))
        .ok()
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
