//! The overlay layers of an entity: sheep wool and undercoat, horse markings, cat and
//! wolf collars, the tropical fish pattern.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::entity::MetadataValue;
use haste_core::ids::RegistryIndex;
use haste_core::model::CubeModel;
use haste_core::Identifier;

use crate::metadata::Metadata;
use crate::model_dump;

use super::tropical_fish::tropical_pattern_texture;
use super::{dye_tint, metadata_index};
/// The overlay layers of an entity (vanilla's `RenderLayer`s that draw a second model with
/// the same transform): sheep wool and its undercoat, horse markings, tamed cat/wolf
/// collars, the tropical fish pattern. The *base* texture changes for a shulker's colour,
/// which is not an overlay — see [`shulker_texture`].
///
/// Results are cached per (type, the inputs that pick them: wool byte, markings, collar
/// colour, packed fish variant); warm lookups are a map probe, and the returned array is
/// copied into the `EntityInstance` (two `Option<Arc>` clones, no allocation beyond that).
pub fn overlays_for(
    kind: RegistryIndex,
    metadata: &Metadata,
    baby: bool,
) -> [Option<haste_core::model::EntityOverlay>; 2] {
    // The overlay mobs all carry their picker in one of five metadata slots; probing those
    // first (a short Vec scan each) keeps entities without overlays — almost all of them —
    // off the registry-name lookup entirely.
    let has_picker = metadata.get(metadata_index::SHEEP_WOOL).is_some()
        || metadata.get(metadata_index::HORSE_TYPE_VARIANT).is_some()
        || metadata
            .get(metadata_index::TROPICAL_FISH_VARIANT)
            .is_some()
        || metadata.get(metadata_index::CAT_COLLAR).is_some()
        || metadata.get(metadata_index::WOLF_COLLAR).is_some();
    if !has_picker {
        return Default::default();
    }
    let int = |index: u8| match metadata.get(index).map(|e| &e.value) {
        Some(MetadataValue::VarInt(i)) => Some(*i),
        _ => None,
    };
    let byte = |index: u8| match metadata.get(index).map(|e| &e.value) {
        Some(MetadataValue::Byte(b)) => Some(*b),
        _ => None,
    };
    let tamed =
        byte(metadata_index::TAME_FLAGS).is_some_and(|b| b & metadata_index::TAME_FLAG_TAMED != 0);
    let Some(name) = crate::registry::entity_type_name(kind) else {
        return Default::default();
    };
    match name {
        // SheepWoolLayer + SheepWoolUndercoatLayer: the wool byte packs the colour (low
        // nibble) and the sheared bit (16); sheared sheep drop the wool layer, and the
        // undercoat hides for white adults (it would be invisible under white wool).
        "minecraft:sheep" => {
            let Some(wool) = byte(metadata_index::SHEEP_WOOL) else {
                return Default::default();
            };
            let color = wool & 15;
            let sheared = wool & 16 != 0;
            let tint = dye_tint(color as i32);
            let mut out: [Option<haste_core::model::EntityOverlay>; 2] = Default::default();
            if !sheared {
                out[0] =
                    wool_layer(baby).map(|model| haste_core::model::EntityOverlay { model, tint });
            }
            // `SheepWoolUndercoatLayer`: adults, and only when the wool is not already
            // white (the same tint would be invisible); a sheared sheep keeps it.
            if !baby && color != 0 {
                out[1] = undercoat_layer(baby)
                    .map(|model| haste_core::model::EntityOverlay { model, tint });
            }
            out
        }
        // HorseMarkingLayer: the base horse model again with a markings texture, no tint
        // (NONE draws nothing).
        "minecraft:horse" => {
            let Some(v) = int(metadata_index::HORSE_TYPE_VARIANT) else {
                return Default::default();
            };
            let markings = (v >> metadata_index::HORSE_MARKINGS_SHIFT) & 0xFF;
            let (adult, baby_tex) = match markings {
                1 => (
                    "entity/horse/horse_markings_white",
                    "entity/horse/horse_markings_white_baby",
                ),
                2 => (
                    "entity/horse/horse_markings_whitefield",
                    "entity/horse/horse_markings_whitefield_baby",
                ),
                3 => (
                    "entity/horse/horse_markings_whitedots",
                    "entity/horse/horse_markings_whitedots_baby",
                ),
                4 => (
                    "entity/horse/horse_markings_blackdots",
                    "entity/horse/horse_markings_blackdots_baby",
                ),
                _ => return Default::default(),
            };
            let model = parent_model_with_texture(kind, baby, if baby { baby_tex } else { adult });
            [
                model.map(|model| haste_core::model::EntityOverlay {
                    model,
                    tint: [1.0; 3],
                }),
                None,
            ]
        }
        // CatCollarLayer: cat#collar layer, the collar dye's tint, tamed cats only.
        "minecraft:cat" if tamed => {
            let color = int(metadata_index::CAT_COLLAR).unwrap_or(14); // DyeColor.RED
            let layer = if baby {
                "minecraft:cat_baby#collar"
            } else {
                "minecraft:cat#collar"
            };
            let model = dump_layer_with_texture(layer, &collar_texture("cat", baby));
            [
                model.map(|model| haste_core::model::EntityOverlay {
                    model,
                    tint: dye_tint(color),
                }),
                None,
            ]
        }
        // WolfCollarLayer: the base wolf model with the collar texture, tamed wolves only.
        "minecraft:wolf" if tamed => {
            let color = int(metadata_index::WOLF_COLLAR).unwrap_or(14); // DyeColor.RED
            let model = parent_model_with_texture(kind, baby, &collar_texture("wolf", baby));
            [
                model.map(|model| haste_core::model::EntityOverlay {
                    model,
                    tint: dye_tint(color),
                }),
                None,
            ]
        }
        // TropicalFishPatternLayer: the pattern layer of the fish's shape, the pattern
        // colour's tint. The base model also switches shape (small/large) with the pattern's
        // base and takes the base colour's tint — handled in `tropical_fish_model`.
        "minecraft:tropical_fish" => {
            let Some(packed) = int(metadata_index::TROPICAL_FISH_VARIANT) else {
                return Default::default();
            };
            let pattern = packed & 0xFFFF;
            let large = pattern >> 8 & 1 == 1;
            let texture = tropical_pattern_texture(pattern);
            let layer = if large {
                "minecraft:tropical_fish_large#pattern"
            } else {
                "minecraft:tropical_fish_small#pattern"
            };
            let pattern_color = dye_tint(packed >> 24 & 0xFF);
            let model = dump_layer_with_texture(layer, &texture);
            [
                model.map(|model| haste_core::model::EntityOverlay {
                    model,
                    tint: pattern_color,
                }),
                None,
            ]
        }
        _ => Default::default(),
    }
}

/// `textures/entity/<kind>/<kind>_collar[_baby].png`.
fn collar_texture(kind: &str, baby: bool) -> String {
    format!(
        "entity/{kind}/{kind}_collar{}",
        if baby { "_baby" } else { "" }
    )
}

/// The sheep wool layer with its (untinted) texture; the tint goes on the overlay.
fn wool_layer(baby: bool) -> Option<Arc<CubeModel>> {
    let layer = if baby {
        "minecraft:sheep_baby#wool"
    } else {
        "minecraft:sheep#wool"
    };
    let texture = if baby {
        "entity/sheep/sheep_wool_baby"
    } else {
        "entity/sheep/sheep_wool"
    };
    dump_layer_with_texture(layer, texture)
}

/// The sheep undercoat layer (adults only in vanilla; babies never get it).
fn undercoat_layer(baby: bool) -> Option<Arc<CubeModel>> {
    if baby {
        return None;
    }
    dump_layer_with_texture(
        "minecraft:sheep#wool_undercoat",
        "entity/sheep/sheep_wool_undercoat",
    )
}

/// A layer from the dump with a texture swapped in, cached per (layer, texture): the cache
/// every overlay resolution shares.
pub(super) fn dump_layer_with_texture(layer: &str, texture: &str) -> Option<Arc<CubeModel>> {
    let cache: &OverlayModelCache = overlay_model_cache();
    let key = (layer.to_owned(), texture.to_owned());
    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }
    let model = model_dump::library()?.get(layer, 1.0).map(|mut m| {
        let texture = Identifier::minecraft(texture);
        if m.texture != texture {
            let mut c = (*m).clone();
            c.texture = texture.clone();
            m = Arc::new(c);
        }
        m
    });
    cache.lock().unwrap().insert(key, model.clone());
    model
}

/// The horse markings / wolf collar model: the parent model re-drawn with another texture
/// (both vanilla layers use `getParentModel()`, so the shape is the base model's, adult or
/// baby column of the model table as `baby` picks).
fn parent_model_with_texture(
    kind: RegistryIndex,
    baby: bool,
    texture: &str,
) -> Option<Arc<CubeModel>> {
    let name = crate::registry::entity_type_name(kind)?;
    let model = if baby {
        crate::registry::baby_model_for_name(name)
            .or_else(|| crate::registry::model_for_name(name))?
    } else {
        crate::registry::model_for_name(name)?
    };
    let texture = Identifier::minecraft(texture);
    if model.texture == texture {
        return Some(model);
    }
    let mut c = (*model).clone();
    c.texture = texture;
    Some(Arc::new(c))
}

type OverlayModelCache = Mutex<HashMap<(String, String), Option<Arc<CubeModel>>>>;

fn overlay_model_cache() -> &'static OverlayModelCache {
    static CACHE: OnceLock<OverlayModelCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
