//! Registry-driven mob variants (26.3): cat, wolf, frog, pig, cow, chicken.
//!
//! Vanilla resolves these mobs' look from a *synchronized* variant registry
//! (`RegistryDataLoader.SYNCHRONIZED_REGISTRIES`): the entity's synched data carries a
//! `Holder<XVariant>` (a plain VarInt registry id on the wire, `ByteBufCodecs.holderRegistry`),
//! and the registry entry's NBT names the textures (and, for pig/cow/chicken, the model type:
//! normal/cold/warm — the temperature layers exist in the model dump as
//! `minecraft:cold_pig#main` and friends).
//!
//! The tables here are filled once at join from the server's `registry_data` (parsed by
//! `haste-client`'s `net::mob_variants`, like the painting variants). Unknown or mod-added
//! variant ids (an id with no entry, or an entry without the fields vanilla's have) fall back
//! to the default-variant row of the model table.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::entity::MetadataValue;
use haste_core::ids::RegistryIndex;
use haste_core::model::CubeModel;
use haste_core::Identifier;

use crate::metadata::Metadata;
use crate::model_dump;

/// Metadata index of the variant field per entity type, from each vanilla class's
/// `defineSynchedData` chain (`SynchedEntityData.defineId` numbers the super chain in order:
/// Entity 0-7, LivingEntity 8-14, Mob 15, AgeableMob 16-17, Animal none, then the class's own
/// fields in declaration order).
pub mod metadata_index {
    /// `Pig.DATA_VARIANT_ID`: boost time 18, variant 19 (Pig.java).
    pub const PIG: u8 = 19;
    /// `Cow.DATA_VARIANT_ID`: variant 18, sound variant 19 (Cow.java; AbstractCow adds none).
    pub const COW: u8 = 18;
    /// `Chicken.DATA_VARIANT_ID`: variant 18, sound variant 19 (Chicken.java).
    pub const CHICKEN: u8 = 18;
    /// `Frog.DATA_VARIANT_ID`: variant 18, tongue target 19 (Frog.java).
    pub const FROG: u8 = 18;
    /// `Cat.DATA_VARIANT_ID`: TamableAnimal 18-19 (flags, owner), variant 20, sound variant
    /// 21 (Cat.java).
    pub const CAT: u8 = 20;
    /// `Wolf.DATA_VARIANT_ID`: TamableAnimal 18-19, interested 20, collar 21, anger 22,
    /// variant 23 (Wolf.java).
    pub const WOLF: u8 = 23;
}

/// Entity metadata serializer ids for the `Holder<XVariant>` fields (the registration order
/// of `EntityDataSerializers`' static block; the protocol's metadata decoder keeps these as
/// `Other { type_id }`).
pub mod serializer_id {
    pub const CAT_VARIANT: i32 = 21;
    pub const COW_VARIANT: i32 = 23;
    pub const WOLF_VARIANT: i32 = 25;
    pub const FROG_VARIANT: i32 = 27;
    pub const PIG_VARIANT: i32 = 28;
    pub const CHICKEN_VARIANT: i32 = 30;
}

/// The model a variant's entry names (vanilla's `ModelAndTexture.model`, "normal" when the
/// field is absent — `ModelAndTexture.codec`'s `optionalFieldOf` default). Only pig, cow and
/// chicken variants carry a model; cat, wolf and frog always use their one layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariantModel {
    Normal,
    Cold,
    Warm,
}

impl VariantModel {
    /// The model string of a synced entry ("normal" is the codec's default when the field is
    /// absent). `None` for a model name we have no layer for (mods): the default look.
    pub fn parse(s: &str) -> Option<VariantModel> {
        match s {
            "normal" => Some(VariantModel::Normal),
            "cold" => Some(VariantModel::Cold),
            "warm" => Some(VariantModel::Warm),
            _ => None,
        }
    }
}

/// One resolved variant: the textures (asset ids, e.g. `minecraft:entity/pig/pig_cold`;
/// `haste-assets` maps them to `textures/<namespace>/<path>.png`) and the model type.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantAssets {
    /// The adult texture (`asset_id`; wolf's `assets.wild`).
    pub adult: Identifier,
    /// The baby texture (`baby_asset_id`; wolf's `baby_assets.wild`).
    pub baby: Identifier,
    /// Which temperature model layers the renderer bakes for this variant.
    pub model: VariantModel,
}

impl VariantAssets {
    /// One entry from its NBT fields (`asset_id`, optional `baby_asset_id` and `model`), with
    /// the asset ids exactly as the synced registry sent them. A frog has no baby form
    /// (tadpoles are their own entity), so its entries carry `asset_id` only and the baby
    /// falls back to the adult texture.
    pub fn from_assets(
        adult: &str,
        baby: Option<&str>,
        model: Option<&str>,
    ) -> Option<VariantAssets> {
        Some(VariantAssets {
            adult: Identifier::parse(adult).ok()?,
            baby: match baby {
                Some(b) => Identifier::parse(b).ok()?,
                None => Identifier::parse(adult).ok()?,
            },
            model: match model {
                Some(m) => VariantModel::parse(m)?,
                None => VariantModel::Normal,
            },
        })
    }
}

/// A single entry of the variant spec: how to read the variant out of one entity type's
/// metadata, and which synced registry the ids index.
struct VariantSpec {
    /// `minecraft:*` entity type name.
    entity: &'static str,
    /// Metadata index of the `Holder<XVariant>` field.
    index: u8,
    /// The metadata serializer id of the field (to tell variant fields from other `Other`
    /// values on the same index shape).
    serializer: i32,
    /// The synced registry name (`minecraft:pig_variant`).
    registry: &'static str,
}

impl VariantSpec {
    const fn new(
        entity: &'static str,
        index: u8,
        serializer: i32,
        registry: &'static str,
    ) -> VariantSpec {
        VariantSpec {
            entity,
            index,
            serializer,
            registry,
        }
    }
}

/// The six registry-driven variant mobs. Order follows vanilla's `EntityDataSerializers`
/// registration (cat, cow, wolf, frog, pig, chicken).
const SPECS: &[VariantSpec] = &[
    VariantSpec::new(
        "minecraft:cat",
        metadata_index::CAT,
        serializer_id::CAT_VARIANT,
        "minecraft:cat_variant",
    ),
    VariantSpec::new(
        "minecraft:cow",
        metadata_index::COW,
        serializer_id::COW_VARIANT,
        "minecraft:cow_variant",
    ),
    VariantSpec::new(
        "minecraft:wolf",
        metadata_index::WOLF,
        serializer_id::WOLF_VARIANT,
        "minecraft:wolf_variant",
    ),
    VariantSpec::new(
        "minecraft:frog",
        metadata_index::FROG,
        serializer_id::FROG_VARIANT,
        "minecraft:frog_variant",
    ),
    VariantSpec::new(
        "minecraft:pig",
        metadata_index::PIG,
        serializer_id::PIG_VARIANT,
        "minecraft:pig_variant",
    ),
    VariantSpec::new(
        "minecraft:chicken",
        metadata_index::CHICKEN,
        serializer_id::CHICKEN_VARIANT,
        "minecraft:chicken_variant",
    ),
];

/// The variant registries this module reads, in `EntityDataSerializers` registration order
/// (cat, cow, wolf, frog, pig, chicken). `haste-client` parses exactly these from the
/// server's `registry_data`.
pub const VARIANT_REGISTRIES: &[&str] = &[
    "minecraft:cat_variant",
    "minecraft:cow_variant",
    "minecraft:wolf_variant",
    "minecraft:frog_variant",
    "minecraft:pig_variant",
    "minecraft:chicken_variant",
];

/// The variant spec for an entity type name, when the type is one of the six.
fn spec_for(entity: &str) -> Option<&'static VariantSpec> {
    SPECS.iter().find(|s| s.entity == entity)
}

/// The variant spec for a registry kind, resolved through the entity-type names.
fn spec_for_kind(kind: RegistryIndex) -> Option<&'static VariantSpec> {
    spec_for(crate::registry::entity_type_name(kind)?.as_str())
}

/// The per-mob variant tables, set once at join. Each entry id maps to its resolved assets;
/// a missing entry means the id is unknown and the default row of the model table applies.
#[derive(Default)]
pub struct MobVariants {
    /// Registry id -> assets, per variant registry (the spec's `registry` name).
    tables: HashMap<&'static str, Vec<Option<VariantAssets>>>,
}

impl MobVariants {
    /// An empty table set (no server registries: every mob keeps its default look).
    pub fn empty() -> MobVariants {
        MobVariants::default()
    }

    /// Replaces one variant registry's table (entries in id order; `None` for an id whose
    /// entry is absent or did not parse — a known pack or a mod's shape: the default look).
    pub fn insert(&mut self, registry: &str, entries: Vec<Option<VariantAssets>>) {
        if let Some(spec) = SPECS.iter().find(|s| s.registry == registry) {
            self.tables.insert(spec.registry, entries);
        }
    }

    /// The assets of the variant an entity's metadata names. `None` when the type has no
    /// variants, the metadata has no variant field yet, the id is not in the synced table or
    /// the entry did not parse: callers keep the model table's default row.
    pub fn assets<'a>(
        &'a self,
        kind: RegistryIndex,
        metadata: &Metadata,
    ) -> Option<&'a VariantAssets> {
        let spec = spec_for_kind(kind)?;
        let id = variant_id(metadata, spec)?;
        self.tables.get(spec.registry)?.get(id as usize)?.as_ref()
    }
}

/// The variant registry id in an entity's metadata, or `None` when the field is absent or
/// malformed (`ByteBufCodecs.holderRegistry` writes a plain VarInt id, no offset).
fn variant_id(metadata: &Metadata, spec: &VariantSpec) -> Option<u32> {
    let entry = metadata.get(spec.index)?;
    let (type_id, raw) = match &entry.value {
        MetadataValue::Other { type_id, raw } => (*type_id, raw),
        _ => return None,
    };
    if type_id != spec.serializer {
        return None;
    }
    let (id, rest) = read_var_int(raw)?;
    if !rest.is_empty() || id < 0 {
        return None;
    }
    Some(id as u32)
}

/// Little-endian VarInt (the wire encoding the metadata decoder sliced into `raw`).
fn read_var_int(bytes: &[u8]) -> Option<(i32, &[u8])> {
    let mut value: u32 = 0;
    let mut shift = 0;
    for (i, b) in bytes.iter().enumerate() {
        value |= ((b & 0x7F) as u32) << shift;
        if b & 0x80 == 0 {
            return Some((value as i32, &bytes[i + 1..]));
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
    None
}

/// The model layers a variant uses: the temperature model's adult layer and its baby layer
/// (vanilla's `AdultAndBabyModelPair.getModel(isBaby)`).
///
/// Pig and chicken babies always use the shared `*_baby` layer whatever the variant
/// (`PigRenderer.bakeModels` maps both model types to `BabyPigModel(PIG_BABY)`;
/// `ChickenRenderer` likewise); cow babies have their own `cold_cow_baby`/`warm_cow_baby`.
/// Cat, wolf and frog have one layer for every variant.
fn layers_for(entity: &str, model: VariantModel) -> (&'static str, &'static str) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::entity::MetadataEntry;

    fn metadata(index: u8, type_id: i32, raw: &[u8]) -> Metadata {
        let mut m = Metadata::default();
        m.apply(&[MetadataEntry {
            index,
            value: MetadataValue::Other {
                type_id,
                raw: raw.to_vec().into(),
            },
        }]);
        m
    }

    /// The variant field's index is the synched-data chain's next slot after the super
    /// classes' fields (Entity 0-7, LivingEntity 8-14, Mob 15, AgeableMob 16-17).
    #[test]
    fn metadata_indices_follow_the_super_chain() {
        assert_eq!(metadata_index::PIG, 19); // boost time at 18
        assert_eq!(metadata_index::COW, 18);
        assert_eq!(metadata_index::CHICKEN, 18);
        assert_eq!(metadata_index::FROG, 18);
        assert_eq!(metadata_index::CAT, 20); // TamableAnimal 18-19
        assert_eq!(metadata_index::WOLF, 23); // TamableAnimal 18-19, wolf 20-22
    }

    /// A VarInt registry id is read from the field; a wrong serializer or a leftover byte
    /// means no variant (fall back to the default).
    #[test]
    fn variant_id_reads_the_varint() {
        let spec = spec_for("minecraft:pig").unwrap();
        let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[2]);
        assert_eq!(variant_id(&m, spec), Some(2));
        // A multi-byte VarInt (id 300 = 0xAC 0x02).
        let m = metadata(
            metadata_index::PIG,
            serializer_id::PIG_VARIANT,
            &[0xAC, 0x02],
        );
        assert_eq!(variant_id(&m, spec), Some(300));
        // Wrong serializer (the pig's sound variant would sit on another index).
        let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT + 1, &[0]);
        assert_eq!(variant_id(&m, spec), None);
        // Trailing bytes: not a well-formed holder id.
        let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[2, 0]);
        assert_eq!(variant_id(&m, spec), None);
    }

    /// Asset fields parse into the table convention; a model name we have no layer for
    /// (mods) resolves to nothing, so the default look applies.
    #[test]
    fn asset_parsing_and_model_fallback() {
        let cold = VariantAssets::from_assets(
            "minecraft:entity/pig/pig_cold",
            Some("minecraft:entity/pig/pig_cold_baby"),
            Some("cold"),
        )
        .unwrap();
        assert_eq!(
            cold,
            VariantAssets {
                adult: Identifier::minecraft("entity/pig/pig_cold"),
                baby: Identifier::minecraft("entity/pig/pig_cold_baby"),
                model: VariantModel::Cold,
            }
        );
        // No model field: the codec's default "normal". A frog has no baby texture: the baby
        // falls back to the adult's.
        let plain =
            VariantAssets::from_assets("minecraft:entity/frog/frog_temperate", None, None).unwrap();
        assert_eq!(plain.model, VariantModel::Normal);
        assert_eq!(
            plain.baby,
            Identifier::minecraft("entity/frog/frog_temperate")
        );
        // A mod's model name we do not know: no assets (default look).
        assert!(VariantAssets::from_assets(
            "mod:entity/pig",
            Some("mod:entity/pig_baby"),
            Some("modded")
        )
        .is_none());
    }

    /// The synced tables are keyed by registry id; unknown ids and absent entries fall back.
    #[test]
    fn table_lookup_falls_back_on_unknown_ids() {
        let mut variants = MobVariants::empty();
        variants.insert(
            "minecraft:pig_variant",
            vec![
                Some(
                    VariantAssets::from_assets(
                        "minecraft:entity/pig/pig_temperate",
                        Some("minecraft:entity/pig/pig_temperate_baby"),
                        None,
                    )
                    .unwrap(),
                ),
                None, // known pack: no NBT
                Some(
                    VariantAssets::from_assets(
                        "minecraft:entity/pig/pig_cold",
                        Some("minecraft:entity/pig/pig_cold_baby"),
                        Some("cold"),
                    )
                    .unwrap(),
                ),
            ],
        );
        // A registry no mob reads is ignored, not stored.
        variants.insert("minecraft:made_up", vec![None]);
        // kind is resolved through the real registries.json; without the cache the name map
        // is empty, so the lookups are checked only when it is present.
        let Some(names) = crate::registry::entity_type_names() else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        let kind = names
            .iter()
            .find(|(_, n)| n.as_str() == "minecraft:pig")
            .map(|(k, _)| *k)
            .unwrap();
        let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[2]);
        let a = variants.assets(kind, &m).unwrap();
        assert_eq!(a.adult, Identifier::minecraft("entity/pig/pig_cold"));
        assert_eq!(a.model, VariantModel::Cold);
        // Id 1 has no entry: default look.
        let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[1]);
        assert!(variants.assets(kind, &m).is_none());
        // Id out of range: default look.
        let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[9]);
        assert!(variants.assets(kind, &m).is_none());
        // Non-variant type: nothing.
        let zombie = names
            .iter()
            .find(|(_, n)| n.as_str() == "minecraft:zombie")
            .map(|(k, _)| *k)
            .unwrap();
        assert!(variants.assets(zombie, &m).is_none());
    }

    /// The temperature model layers exist in the dump; pig and chicken babies share one baby
    /// layer whatever the variant, cow babies do not.
    #[test]
    #[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
    fn temperature_layers_exist_in_the_dump() {
        let lib = model_dump::library().expect("entity model dump");
        for (entity, model) in [
            ("minecraft:pig", VariantModel::Normal),
            ("minecraft:pig", VariantModel::Cold),
            ("minecraft:cow", VariantModel::Normal),
            ("minecraft:cow", VariantModel::Warm),
            ("minecraft:cow", VariantModel::Cold),
            ("minecraft:chicken", VariantModel::Normal),
            ("minecraft:chicken", VariantModel::Cold),
        ] {
            let (adult, baby) = layers_for(entity, model);
            assert!(
                lib.get(adult, 1.0).is_some(),
                "{adult} missing from the dump"
            );
            assert!(lib.get(baby, 1.0).is_some(), "{baby} missing from the dump");
        }
    }

    /// With the dump present, a cold pig's variant model differs from the default one's and
    /// carries the variant texture; without it both are None and the store falls back.
    #[test]
    #[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
    fn variant_model_resolves_through_the_dump() {
        let Some(names) = crate::registry::entity_type_names() else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        let kind = names
            .iter()
            .find(|(_, n)| n.as_str() == "minecraft:pig")
            .map(|(k, _)| *k)
            .unwrap();
        let mut variants = MobVariants::empty();
        variants.insert(
            "minecraft:pig_variant",
            vec![Some(
                VariantAssets::from_assets(
                    "minecraft:entity/pig/pig_cold",
                    Some("minecraft:entity/pig/pig_cold_baby"),
                    Some("cold"),
                )
                .unwrap(),
            )],
        );
        let m = metadata(metadata_index::PIG, serializer_id::PIG_VARIANT, &[0]);
        let model = model_for(kind, &variants, &m, false).expect("cold pig model");
        assert_eq!(model.texture, Identifier::minecraft("entity/pig/pig_cold"));
        // The default (temperate) model differs.
        let plain = crate::registry::model_for_name("minecraft:pig").unwrap();
        assert_ne!(model.root, plain.root);
        // Baby: the shared baby layer with the variant's baby texture.
        let baby = model_for(kind, &variants, &m, true).expect("cold pig baby model");
        assert_eq!(
            baby.texture,
            Identifier::minecraft("entity/pig/pig_cold_baby")
        );
        // No variant in play: nothing, the store keeps the table's row.
        assert!(model_for(kind, &variants, &Metadata::default(), false).is_none());
    }
}
