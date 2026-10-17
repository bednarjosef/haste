//! The model of a registry variant: the temperature layer pair its entry names
//! (`AdultAndBabyModelPair`) with the variant's texture.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::ids::RegistryIndex;
use haste_core::model::CubeModel;

use crate::metadata::Metadata;
use crate::model_dump;

use super::synced_registry::{spec_for_kind, variant_id};
use super::{MobVariants, VariantModel};
/// The model layers a variant uses: the temperature model's adult layer and its baby layer
/// (vanilla's `AdultAndBabyModelPair.getModel(isBaby)`).
///
/// Pig and chicken babies always use the shared `*_baby` layer whatever the variant
/// (`PigRenderer.bakeModels` maps both model types to `BabyPigModel(PIG_BABY)`;
/// `ChickenRenderer` likewise); cow babies have their own `cold_cow_baby`/`warm_cow_baby`.
/// Cat, wolf and frog have one layer for every variant.
pub(super) fn layers_for(entity: &str, model: VariantModel) -> (&'static str, &'static str) {
    match (entity, model) {
        ("minecraft:pig", VariantModel::Cold) => {
            ("minecraft:cold_pig#main", "minecraft:pig_baby#main")
        }
        ("minecraft:pig", _) => ("minecraft:pig#main", "minecraft:pig_baby#main"),
        ("minecraft:cow", VariantModel::Cold) => {
            ("minecraft:cold_cow#main", "minecraft:cold_cow_baby#main")
        }
        ("minecraft:cow", VariantModel::Warm) => {
            ("minecraft:warm_cow#main", "minecraft:warm_cow_baby#main")
        }
        ("minecraft:cow", _) => ("minecraft:cow#main", "minecraft:cow_baby#main"),
        ("minecraft:chicken", VariantModel::Cold) => {
            ("minecraft:cold_chicken#main", "minecraft:chicken_baby#main")
        }
        ("minecraft:chicken", _) => ("minecraft:chicken#main", "minecraft:chicken_baby#main"),
        ("minecraft:cat", _) => ("minecraft:cat#main", "minecraft:cat_baby#main"),
        ("minecraft:wolf", _) => ("minecraft:wolf#main", "minecraft:wolf_baby#main"),
        _ => ("minecraft:frog#main", "minecraft:frog#main"),
    }
}

/// The per-variant model cache: `(kind, variant id, baby)` -> model. The layer models are
/// shared `Arc`s from the dump library; this only clones per distinct look, never per frame.
type ModelCache = Mutex<HashMap<(RegistryIndex, u32, bool), Option<Arc<CubeModel>>>>;

/// The variant model for an entity: the layer its variant's model type bakes, with the
/// variant's texture. `None` (callers fall back to the default row) when the dump is absent,
/// lacks the layer, or the entity has no variant in play. Results are cached per
/// (type, variant, baby); warm lookups are a map probe.
pub fn model_for(
    kind: RegistryIndex,
    variants: &MobVariants,
    metadata: &Metadata,
    baby: bool,
) -> Option<Arc<CubeModel>> {
    let spec = spec_for_kind(kind)?;
    let assets = variants.assets(kind, metadata)?;
    let id = variant_id(metadata, spec)?;
    let name = crate::registry::entity_type_name(kind)?;
    let key = (kind, id, baby);
    let cache: &ModelCache = model_cache();
    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }
    let (adult_layer, baby_layer) = layers_for(&name, assets.model);
    let (layer, texture) = if baby {
        (baby_layer, &assets.baby)
    } else {
        (adult_layer, &assets.adult)
    };
    let model = model_dump::library()?.get(layer, 1.0).map(|mut m| {
        if m.texture != *texture {
            let mut c = (*m).clone();
            c.texture = texture.clone();
            m = Arc::new(c);
        }
        m
    });
    cache.lock().unwrap().insert(key, model.clone());
    model
}

/// The process-wide variant model cache.
fn model_cache() -> &'static ModelCache {
    static CACHE: OnceLock<ModelCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
