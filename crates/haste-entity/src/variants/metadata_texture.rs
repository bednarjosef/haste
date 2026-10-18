//! Metadata-driven variants (int fields in synched data, no registry): each renderer's
//! texture table and the model that carries the chosen texture.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::entity::MetadataValue;
use haste_core::ids::RegistryIndex;
use haste_core::model::CubeModel;
use haste_core::Identifier;

use crate::metadata::Metadata;

use super::metadata_index;
// ---------------------------------------------------------------------------
// Metadata-driven variants (int fields in synched data, no registry).
// ---------------------------------------------------------------------------

/// The texture a metadata-driven variant mob renders with: its renderer's texture table
/// indexed by the variant field. Everything here comes from the vanilla renderer named in
/// each `texture_for` arm; the model layer never changes with these variants (the same
/// `AgeableMobRenderer` pair draws them all), only the texture does.
///
/// `world_age` is the world time the bee's anger end time compares against
/// (`NeutralMob.isAngry`); callers that have no clock yet pass 0 (the bee then shows calm
/// for any end time in the past, which is right until the first `set_time`).
///
/// `None` (the model table's default row stands) when the type has no such variant or the
/// metadata has no field yet. Unknown ids use the renderer's own fallback
/// (`getOrDefault`-style) where vanilla has one, and the default texture where not.
pub fn texture_for(
    kind: RegistryIndex,
    metadata: &Metadata,
    baby: bool,
    custom_name: Option<&str>,
    world_age: i64,
) -> Option<Identifier> {
    let name = crate::registry::entity_type_name(kind)?;
    let int = |index: u8| match metadata.get(index).map(|e| &e.value) {
        Some(MetadataValue::VarInt(i)) => Some(*i),
        _ => None,
    };
    let byte = |index: u8| match metadata.get(index).map(|e| &e.value) {
        Some(MetadataValue::Byte(b)) => Some(*b),
        _ => None,
    };
    let tex = |adult: &str, baby_tex: &str| {
        Some(Identifier::minecraft(if baby { baby_tex } else { adult }))
    };
    match name {
        // AxolotlRenderer: axolotl_<variant name>.png / _baby.png, id clamped like
        // `ByIdMap.continuous(OutOfBoundsStrategy.ZERO)` (unknown -> LUCY).
        "minecraft:axolotl" => {
            let id = int(metadata_index::AXOLOTL)?;
            let variant = match id {
                0..=4 => ["lucy", "wild", "gold", "cyan", "blue"][id as usize],
                _ => "lucy",
            };
            Some(Identifier::minecraft(&format!(
                "entity/axolotl/axolotl_{variant}{}",
                if baby { "_baby" } else { "" }
            )))
        }
        // BeeRenderer: angry x nectar x baby, four textures each age. Nectar is bit 8 of the
        // bee's flags; angry is the anger end time past the world age
        // (`NeutralMob.isAngry`).
        "minecraft:bee" => {
            let nectar = byte(metadata_index::BEE_FLAGS).is_some_and(|b| b & 8 != 0);
            let angry = match metadata
                .get(metadata_index::BEE_ANGER_END)
                .map(|e| &e.value)
            {
                Some(MetadataValue::VarLong(end)) => *end > 0 && *end > world_age,
                _ => false,
            };
            let state = match (angry, nectar) {
                (true, true) => "_angry_nectar",
                (true, false) => "_angry",
                (false, true) => "_nectar",
                (false, false) => "",
            };
            Some(Identifier::minecraft(&format!(
                "entity/bee/bee{state}{}",
                if baby { "_baby" } else { "" }
            )))
        }
        // FoxRenderer: red/snow with a sleep texture (the sleep flag is bit 32 of the fox's
        // own flags byte at index 19).
        "minecraft:fox" => {
            let snow = int(metadata_index::FOX_TYPE)? == 1;
            let sleeping = byte(metadata_index::FOX_FLAGS).is_some_and(|b| b & 32 != 0);
            tex(
                &fox_texture(snow, sleeping, false),
                &fox_texture(snow, sleeping, true),
            )
        }
        // RabbitRenderer: brown/white/black/white_splotched/gold/salt, id 99 the killer
        // rabbit (caerbannog); a rabbit named "Toast" always shows the toast skin
        // (`MobRenderer.checkMagicName`).
        "minecraft:rabbit" => {
            if custom_name == Some("Toast") {
                return tex(
                    "entity/rabbit/rabbit_toast",
                    "entity/rabbit/rabbit_toast_baby",
                );
            }
            let id = int(metadata_index::RABBIT)?;
            let (adult, baby) = match id {
                1 => (
                    "entity/rabbit/rabbit_white",
                    "entity/rabbit/rabbit_white_baby",
                ),
                2 => (
                    "entity/rabbit/rabbit_black",
                    "entity/rabbit/rabbit_black_baby",
                ),
                3 => (
                    "entity/rabbit/rabbit_white_splotched",
                    "entity/rabbit/rabbit_white_splotched_baby",
                ),
                4 => (
                    "entity/rabbit/rabbit_gold",
                    "entity/rabbit/rabbit_gold_baby",
                ),
                5 => (
                    "entity/rabbit/rabbit_salt",
                    "entity/rabbit/rabbit_salt_baby",
                ),
                99 => (
                    "entity/rabbit/rabbit_caerbannog",
                    "entity/rabbit/rabbit_caerbannog_baby",
                ),
                // Sparse byId: unknown ids are the default BROWN.
                _ => (
                    "entity/rabbit/rabbit_brown",
                    "entity/rabbit/rabbit_brown_baby",
                ),
            };
            tex(adult, baby)
        }
        // HorseRenderer LOCATION_BY_VARIANT, id wrapped like `ByIdMap.continuous(WRAP)`.
        "minecraft:horse" => {
            let v = int(metadata_index::HORSE_TYPE_VARIANT)? & 0xff;
            let (adult, baby) = match v {
                1 => (
                    "entity/horse/horse_creamy",
                    "entity/horse/horse_creamy_baby",
                ),
                2 => (
                    "entity/horse/horse_chestnut",
                    "entity/horse/horse_chestnut_baby",
                ),
                3 => ("entity/horse/horse_brown", "entity/horse/horse_brown_baby"),
                4 => ("entity/horse/horse_black", "entity/horse/horse_black_baby"),
                5 => ("entity/horse/horse_gray", "entity/horse/horse_gray_baby"),
                6 => (
                    "entity/horse/horse_darkbrown",
                    "entity/horse/horse_darkbrown_baby",
                ),
                _ => ("entity/horse/horse_white", "entity/horse/horse_white_baby"),
            };
            tex(adult, baby)
        }
        // LlamaRenderer TEXTURES (same table for trader llamas: the renderer is shared).
        "minecraft:llama" | "minecraft:trader_llama" => {
            let v = int(metadata_index::LLAMA_VARIANT)?.rem_euclid(4);
            let (adult, baby) = match v {
                1 => ("entity/llama/llama_white", "entity/llama/llama_white_baby"),
                2 => ("entity/llama/llama_brown", "entity/llama/llama_brown_baby"),
                3 => ("entity/llama/llama_gray", "entity/llama/llama_gray_baby"),
                _ => (
                    "entity/llama/llama_creamy",
                    "entity/llama/llama_creamy_baby",
                ),
            };
            tex(adult, baby)
        }
        // PandaRenderer: the shown gene comes from the main and hidden genes
        // (`Gene.getVariantFromGenes`: a recessive main gene shows only when the hidden gene
        // matches, else NORMAL; a dominant main gene always shows).
        "minecraft:panda" => {
            let (main, hidden) = (
                byte(metadata_index::PANDA_MAIN_GENE)?,
                byte(metadata_index::PANDA_HIDDEN_GENE)?,
            );
            let (main, hidden) = (
                main.unsigned_abs().rem_euclid(7),
                hidden.unsigned_abs().rem_euclid(7),
            );
            // Recessive: brown (4) and weak (5).
            let shown = if main == 4 || main == 5 {
                if main == hidden {
                    main
                } else {
                    0
                }
            } else {
                main
            };
            let name = [
                "panda",
                "lazy_panda",
                "worried_panda",
                "playful_panda",
                "brown_panda",
                "weak_panda",
                "aggressive_panda",
            ][shown as usize];
            tex(
                &format!("entity/panda/{name}"),
                &format!("entity/panda/{name}_baby"),
            )
        }
        // ParrotRenderer.getVariantTexture.
        "minecraft:parrot" => {
            let v = int(metadata_index::PARROT)?.clamp(0, 4);
            tex(
                [
                    "entity/parrot/parrot_red_blue",
                    "entity/parrot/parrot_blue",
                    "entity/parrot/parrot_green",
                    "entity/parrot/parrot_yellow_blue",
                    "entity/parrot/parrot_grey",
                ][v as usize],
                [
                    "entity/parrot/parrot_red_blue",
                    "entity/parrot/parrot_blue",
                    "entity/parrot/parrot_green",
                    "entity/parrot/parrot_yellow_blue",
                    "entity/parrot/parrot_grey",
                ][v as usize],
            )
        }
        // MushroomCowRenderer TEXTURES (the mushroom block layer on top is an overlay,
        // pending the EntityInstance overlay interface).
        "minecraft:mooshroom" => {
            let brown = int(metadata_index::MOOSHROOM_TYPE)? == 1;
            tex("entity/cow/mooshroom_red", "entity/cow/mooshroom_red_baby")
                .filter(|_| !brown)
                .or_else(|| {
                    tex(
                        "entity/cow/mooshroom_brown",
                        "entity/cow/mooshroom_brown_baby",
                    )
                })
        }
        // StriderRenderer: suffocating (cold) swaps to the cold texture pair.
        "minecraft:strider" => {
            let cold = matches!(
                metadata
                    .get(metadata_index::STRIDER_SUFFOCATING)
                    .map(|e| &e.value),
                Some(MetadataValue::Boolean(true))
            );
            if cold {
                tex(
                    "entity/strider/strider_cold",
                    "entity/strider/strider_cold_baby",
                )
            } else {
                tex("entity/strider/strider", "entity/strider/strider_baby")
            }
        }
        _ => None,
    }
}

/// A fox's texture (`FoxRenderer.TEXTURES_BY_VARIANT`): snow/red, sleep/idle, baby/adult.
fn fox_texture(snow: bool, sleeping: bool, baby: bool) -> String {
    match (snow, sleeping, baby) {
        (false, false, false) => "entity/fox/fox".into(),
        (false, false, true) => "entity/fox/fox_baby".into(),
        (false, true, false) => "entity/fox/fox_sleep".into(),
        (false, true, true) => "entity/fox/fox_sleep_baby".into(),
        (true, false, false) => "entity/fox/fox_snow".into(),
        (true, false, true) => "entity/fox/fox_snow_baby".into(),
        (true, true, false) => "entity/fox/fox_snow_sleep".into(),
        (true, true, true) => "entity/fox/fox_snow_sleep_baby".into(),
    }
}

/// The metadata-driven model for an entity: the type's layer from the model table with the
/// variant texture. Cached per (type, texture, baby) — the texture set is small, so warm
/// lookups are a map probe. `None` when there is no variant in play (the default row).
/// `world_age` feeds the bee's anger end time (`texture_for`).
pub fn metadata_model_for(
    kind: RegistryIndex,
    metadata: &Metadata,
    baby: bool,
    custom_name: Option<&str>,
    world_age: i64,
) -> Option<Arc<CubeModel>> {
    let name = crate::registry::entity_type_name(kind)?;
    let texture = texture_for(kind, metadata, baby, custom_name, world_age)?;
    let key = (kind, texture.to_string(), baby);
    let cache: &TextureModelCache = texture_model_cache();
    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }
    // The layer comes from the model table's row for this type (adult or baby column).
    let model = (if baby {
        crate::registry::baby_model_for_name(name)
    } else {
        crate::registry::model_for_name(name)
    })
    .map(|mut m| {
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

type TextureModelCache = Mutex<HashMap<(RegistryIndex, String, bool), Option<Arc<CubeModel>>>>;

fn texture_model_cache() -> &'static TextureModelCache {
    static CACHE: OnceLock<TextureModelCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
