//! Thrown projectiles drawn as their item (`ThrownItemRenderer`): snowballs, eggs, ender
//! pearls, eyes of ender, splash and lingering potions, experience bottles and the two
//! fireballs. Vanilla registers each type in `EntityRenderers` with a `ThrownItemRenderer`
//! (its scale and full-bright arguments here); which item it draws is the entity's synched
//! stack (`ThrowableItemProjectile.DATA_ITEM_STACK`, the first slot after Entity's eight)
//! or, when the server did not send one, the type's default item
//! (`ThrowableItemProjectile.getDefaultItem` and friends).

use std::collections::HashMap;
use std::sync::OnceLock;

use haste_core::entity::MetadataValue;
use haste_core::model::{BodyOp, BodyTransform, ThrownItemDecoration};
use haste_core::RegistryIndex;

use crate::metadata::Metadata;

/// `FireworkRocketEntity.DATA_SHOT_AT_ANGLE`'s synched slot.
const SHOT_AT_ANGLE_INDEX: u8 = 10;

/// One thrown-item entity type: `EntityRenderers`' registration of it — the
/// `ThrownItemRenderer` constructor's `scale` and `fullBright` — plus the item its class
/// defaults to, drawn when the synched stack does not say otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrownItem {
    pub scale: f32,
    pub full_bright: bool,
    default_item: &'static str,
}

/// `ThrownItemRenderer(context)` — scale 1, not full-bright (`EntityRenderers`' default).
const fn plain(default_item: &'static str) -> ThrownItem {
    ThrownItem {
        scale: 1.0,
        full_bright: false,
        default_item,
    }
}

/// `new ThrownItemRenderer<>(context, scale, fullBright)` for the three types that pass
/// their own arguments.
const fn scaled(default_item: &'static str, scale: f32) -> ThrownItem {
    ThrownItem {
        scale,
        full_bright: true,
        default_item,
    }
}

/// The thrown item of an entity type name, `None` for every type vanilla draws otherwise.
pub fn variant_for_name(name: &str) -> Option<ThrownItem> {
    Some(match name {
        // EntityRenderers: EGG, ENDER_PEARL, EXPERIENCE_BOTTLE, LINGERING_POTION, SNOWBALL,
        // SPLASH_POTION -> plain ThrownItemRenderer; getDefaultItem each class.
        "minecraft:egg" => plain("minecraft:egg"),
        "minecraft:ender_pearl" => plain("minecraft:ender_pearl"),
        "minecraft:experience_bottle" => plain("minecraft:experience_bottle"),
        "minecraft:lingering_potion" => plain("minecraft:lingering_potion"),
        "minecraft:snowball" => plain("minecraft:snowball"),
        "minecraft:splash_potion" => plain("minecraft:splash_potion"),
        // EYE_OF_ENDER: (context, 1.0F, true), EyeOfEnder.getDefaultItem = ENDER_EYE.
        "minecraft:eye_of_ender" => scaled("minecraft:ender_eye", 1.0),
        // FireworkEntityRenderer: the camera-facing item at scale 1, not full-bright, the
        // same transform as ThrownItemRenderer. Its rocket item is synched (slot 8) and
        // always sent; a shot-at-angle rocket is turned by `flight_turn`.
        "minecraft:firework_rocket" => plain("minecraft:firework_rocket"),
        // FIREBALL: (context, 3.0F, true), Fireball.getDefaultItem = FIRE_CHARGE.
        "minecraft:fireball" => scaled("minecraft:fire_charge", 3.0),
        // SMALL_FIREBALL: (context, 0.75F, true), SmallFireball inherits FIRE_CHARGE.
        "minecraft:small_fireball" => scaled("minecraft:fire_charge", 0.75),
        _ => return None,
    })
}

/// The thrown item of a tracked entity's type.
pub fn variant_for_kind(kind: RegistryIndex) -> Option<ThrownItem> {
    crate::registry::entity_type_name(kind).and_then(variant_for_name)
}

/// Whether an entity type trails smoke every tick
/// (`AbstractHurtingProjectile.createParticleTrail`/`getTrailParticle`: SMOKE for the
/// fireball family and the wither skull; the dragon fireball trails dragon breath and the
/// wind charges nothing).
pub fn has_smoke_trail(kind: RegistryIndex) -> bool {
    matches!(
        crate::registry::entity_type_name(kind),
        Some("minecraft:fireball")
            | Some("minecraft:small_fireball")
            | Some("minecraft:wither_skull")
    )
}

impl crate::store::EntityStore {
    /// The current positions of the entities that trail smoke
    /// (`AbstractHurtingProjectile.createParticleTrail`), for the client's per-tick particle
    /// spawn — no allocation, one position per trailing entity.
    pub fn smoke_trail_positions(&self) -> impl Iterator<Item = haste_core::DVec3> + '_ {
        self.slots
            .iter()
            .flatten()
            .filter(|slot| has_smoke_trail(slot.kind))
            .map(|slot| slot.pos)
    }
}

/// Whether an entity type is a firework rocket (`FireworkRocketEntity`), which trails its
/// `FIREWORK` particles while it flies.
pub fn is_firework_rocket(kind: RegistryIndex) -> bool {
    crate::registry::entity_type_name(kind) == Some("minecraft:firework_rocket")
}

/// The render state `ThrownItemRenderer.extractRenderState` collects: the item to draw
/// (`ItemSupplier.getItem`, the synched stack or the type's default), with the renderer's
/// scale and full-bright. `None` when no item resolves (an unknown default item draws
/// nothing rather than guessing; vanilla's is always a known vanilla item).
pub fn decoration(t: &ThrownItem, metadata: &Metadata) -> Option<ThrownItemDecoration> {
    Some(ThrownItemDecoration {
        item: item_of(metadata).or_else(|| default_item_index(t.default_item))?,
        scale: t.scale,
        full_bright: t.full_bright,
    })
}

/// The entity's synched stack (`DATA_ITEM_STACK`, slot 8): a non-empty stack's item. The
/// server only sends the slot when it differs from the default, so `None` usually means
/// "use the type's default item" (an emptied stack, which vanilla never sends — the slot is
/// defined from the type's item — falls back to it too).
fn item_of(metadata: &Metadata) -> Option<RegistryIndex> {
    match metadata.get(8).map(|e| &e.value) {
        Some(MetadataValue::ItemStack(s)) if !s.is_empty() => s.item,
        _ => None,
    }
}

/// `FireworkRocketEntity.isShotAtAngle` (`DATA_SHOT_AT_ANGLE`, the boolean at index 10, after
/// the item at 8 and the attached target at 9): a rocket shot from a crossbow or at an angle
/// is drawn turned to its flight.
pub fn shot_at_angle(metadata: &Metadata) -> bool {
    matches!(
        metadata.get(SHOT_AT_ANGLE_INDEX).map(|e| &e.value),
        Some(MetadataValue::Boolean(true))
    )
}

/// `FireworkEntityRenderer.submit`: a rocket shot at an angle turns about Z 180, Y 180, X 90
/// after the camera orientation. The turn goes on the instance's whole-model body transform
/// (`EntityInstance.body`, applied before the camera orientation by the thrown-item renderer),
/// so the ops are pushed in vanilla's order. Every other thrown item keeps its empty body.
pub fn flight_turn(t: &ThrownItem, metadata: &Metadata, body: &mut BodyTransform) {
    if t.default_item == "minecraft:firework_rocket" && shot_at_angle(metadata) {
        body.push(BodyOp::RotateZ(std::f32::consts::PI));
        body.push(BodyOp::RotateY(std::f32::consts::PI));
        body.push(BodyOp::RotateX(std::f32::consts::FRAC_PI_2));
    }
}

/// A default item's registry index, from the data-generator report's `minecraft:item`
/// registry (read once; the same file the client's item names come from). A missing report
/// or an unknown name logs once and means the projectile draws nothing until one appears.
fn default_item_index(name: &str) -> Option<RegistryIndex> {
    item_indices()
        .and_then(|m| m.get(name).copied())
        .or_else(|| {
            static WARNED: OnceLock<bool> = OnceLock::new();
            WARNED.get_or_init(|| {
                tracing::warn!("thrown item: default item {name} not in the item registry");
                true
            });
            None
        })
}

/// `minecraft:item` names by protocol id, from `registries.json` (haste-data's
/// `Registries::parse` is crate-private; the same one-file read `registry.rs` does for
/// entity types).
pub(crate) fn item_indices() -> Option<&'static HashMap<&'static str, RegistryIndex>> {
    static ITEMS: OnceLock<Option<HashMap<&'static str, RegistryIndex>>> = OnceLock::new();
    ITEMS
        .get_or_init(|| {
            let path = haste_core::paths::datagen_reports().join("registries.json");
            let bytes = std::fs::read(&path).ok()?;
            #[derive(serde::Deserialize)]
            struct ItemRegistryJson {
                entries: HashMap<String, Id>,
            }
            #[derive(serde::Deserialize)]
            struct Id {
                protocol_id: u32,
            }
            let registries: HashMap<&str, ItemRegistryJson> =
                serde_json::from_slice(&bytes).ok()?;
            let reg = registries.get("minecraft:item")?;
            let mut items = HashMap::with_capacity(reg.entries.len());
            for (name, e) in &reg.entries {
                items.insert(
                    Box::leak((*name).to_owned().into_boxed_str()) as &'static str,
                    RegistryIndex(e.protocol_id),
                );
            }
            Some(items)
        })
        .as_ref()
}
