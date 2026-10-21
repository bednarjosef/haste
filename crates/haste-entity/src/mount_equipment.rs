//! The mounts' equipment layers: vanilla `HorseRenderer`'s `HorseArmorLayer` (horse body armour),
//! `WolfArmorLayer` (wolf body armour and its crack overlay) and the `SaddleLayer`s of the
//! horse family, pig, strider, camel and nautilus. Each layer is the dumped model of its vanilla
//! layer, drawn with the base model's pose; the equipment asset's layer type (the name its
//! textures are keyed by) comes with it. An entity carries no kind of its own for this, so the
//! layer is picked by the base model's texture path (`entity/horse/horse_white`, ...).
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::model::{CubeModel, EntityOverlay, EquipmentSlot};
use haste_core::Identifier;
use haste_data::ItemComponents;

use crate::model_dump;

/// A mount's equipment layer: the dumped geometry and the equipment layer type (the serialized
/// name of `EquipmentClientInfo.LayerType`) whose textures the worn item's asset holds.
#[derive(Clone)]
pub struct MountLayer {
    pub model: Arc<CubeModel>,
    pub layer_type: &'static str,
}

/// The equipment layer a mount draws for a worn slot, given its base model's texture path. `None`
/// for a slot the mount has no layer for (zombie and skeleton horses take no body armour here),
/// and for babies, which are never armoured or saddled.
pub fn layer_for(texture: &str, slot: EquipmentSlot) -> Option<MountLayer> {
    let (layer, layer_type) = layer_names(texture, slot)?;
    Some(MountLayer {
        model: layer_model(layer, None)?,
        layer_type,
    })
}

/// The dumped layer name and the equipment layer type a mount's slot draws with.
pub(crate) fn layer_names(
    texture: &str,
    slot: EquipmentSlot,
) -> Option<(&'static str, &'static str)> {
    if is_baby(texture) {
        return None;
    }
    match slot {
        // `HorseArmorLayer` (`HORSE_BODY`) and `WolfArmorLayer` (`WOLF_BODY`).
        EquipmentSlot::Body if is_horse(texture) => {
            Some(("minecraft:horse_armor#main", "horse_body"))
        }
        EquipmentSlot::Body if is_wolf(texture) => Some(("minecraft:wolf_armor#main", "wolf_body")),
        // The saddle layers: each mount's own saddle geometry and layer type.
        EquipmentSlot::Saddle => Some(match texture {
            "entity/horse/donkey" => ("minecraft:donkey#saddle", "donkey_saddle"),
            "entity/horse/mule" => ("minecraft:mule#saddle", "mule_saddle"),
            "entity/camel/camel" => ("minecraft:camel#saddle", "camel_saddle"),
            "entity/camel/camel_husk" => ("minecraft:camel_husk#saddle", "camel_husk_saddle"),
            "entity/nautilus/nautilus" => ("minecraft:nautilus#saddle", "nautilus_saddle"),
            "entity/pig/pig_temperate" | "entity/pig/pig_warm" | "entity/pig/pig_cold" => {
                ("minecraft:pig#saddle", "pig_saddle")
            }
            "entity/strider/strider" | "entity/strider/strider_cold" => {
                ("minecraft:strider#saddle", "strider_saddle")
            }
            _ if is_horse(texture) => ("minecraft:horse#saddle", "horse_saddle"),
            _ => return None,
        }),
        _ => None,
    }
}

/// The wolf armour's crack overlay (vanilla `WolfArmorLayer.maybeRenderCracks`): the armour's
/// geometry re-drawn with the crack texture for the worn armour's damage. `None` for anything
/// but an adult wolf wearing damaged armour, and for armour not yet cracked. Drawn after the
/// armour, at the armour's pose.
pub fn crack_overlay(
    texture: &str,
    equipment: &[(u8, haste_core::entity::ItemStackData)],
    components: &ItemComponents,
) -> Option<EntityOverlay> {
    if !is_wolf(texture) || is_baby(texture) {
        return None;
    }
    let (_, stack) = equipment
        .iter()
        .find(|(wire, _)| EquipmentSlot::from_wire(*wire) == Some(EquipmentSlot::Body))?;
    let item = stack.item?;
    let max = components.max_damage(item)?;
    let damage = components.damage(&stack.components_raw).unwrap_or(0);
    let texture = crackiness_texture(cracked_fraction(max, damage)?)?;
    let model = layer_model("minecraft:wolf_armor#main", Some(texture))?;
    Some(EntityOverlay {
        model,
        tint: [1.0; 3],
    })
}

/// Whether a model is a wolf armour crack overlay (its texture is one of the crack textures),
/// which the armour pass draws itself, after the armour.
pub fn is_crack_overlay(model: &CubeModel) -> bool {
    model
        .texture
        .path()
        .starts_with("entity/wolf/wolf_armor_crackiness")
}

/// `Crackiness.byDamage`'s remaining fraction: `(maxDamage - damage) / maxDamage`. `None` for
/// an item that takes no damage (no max).
pub(crate) fn cracked_fraction(max: u32, damage: u32) -> Option<f32> {
    if max == 0 {
        return None;
    }
    Some((max as i64 - damage as i64) as f32 / max as f32)
}

/// `Crackiness.WOLF_ARMOR.byFraction` as the crack texture it selects (`WolfArmorLayer`'s
/// `ARMOR_CRACK_LOCATIONS`): high below 0.32, medium below 0.69, low below 0.95, none above.
pub(crate) fn crackiness_texture(fraction: f32) -> Option<&'static str> {
    if fraction < 0.32 {
        Some("entity/wolf/wolf_armor_crackiness_high")
    } else if fraction < 0.69 {
        Some("entity/wolf/wolf_armor_crackiness_medium")
    } else if fraction < 0.95 {
        Some("entity/wolf/wolf_armor_crackiness_low")
    } else {
        None
    }
}

fn is_baby(texture: &str) -> bool {
    texture.ends_with("_baby")
}

/// The horse's base textures (`horse_white`, `horse_brown`, ...); the markings overlays are not
/// base textures.
fn is_horse(texture: &str) -> bool {
    texture.starts_with("entity/horse/horse_")
}

/// The wolf's base textures (variants of `wolf`); the collar overlay is not one.
fn is_wolf(texture: &str) -> bool {
    texture.starts_with("entity/wolf/wolf") && !texture.contains("collar")
}

type LayerCache = Mutex<HashMap<(&'static str, Option<&'static str>), Option<Arc<CubeModel>>>>;

/// A dumped layer's geometry, with its texture swapped for `texture` when given. The dump hands
/// out a fresh copy per call, so each (layer, texture) pair is built once and shared; a frame
/// only takes the lock and clones the `Arc`.
pub fn layer_model(layer: &'static str, texture: Option<&'static str>) -> Option<Arc<CubeModel>> {
    static CACHE: OnceLock<LayerCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (layer, texture);
    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }
    let model = model_dump::library()?.get(layer, 1.0).map(|mut m| {
        if let Some(texture) = texture.map(Identifier::minecraft) {
            if m.texture != texture {
                let mut c = (*m).clone();
                c.texture = texture;
                m = Arc::new(c);
            }
        }
        m
    });
    cache.lock().unwrap().insert(key, model.clone());
    model
}
