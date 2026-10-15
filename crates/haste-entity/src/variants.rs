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
    /// `Axolotl.DATA_VARIANT`: variant 18, playing dead 19, from bucket 20 (Axolotl.java).
    pub const AXOLOTL: u8 = 18;
    /// `Rabbit.DATA_TYPE_ID`: the rabbit's variant 18 (Rabbit.java).
    pub const RABBIT: u8 = 18;
    /// `Horse.DATA_ID_TYPE_VARIANT`: AbstractHorse's flags 18, then the packed
    /// variant/markings int 19 (Horse.java: `variant = id & 0xFF`, `markings = id >> 8`).
    pub const HORSE_TYPE_VARIANT: u8 = 19;
    /// `Llama.DATA_VARIANT_ID`: AbstractHorse 18, AbstractChestedHorse's chest 19,
    /// strength 20, variant 21 (Llama.java).
    pub const LLAMA_VARIANT: u8 = 21;
    /// `Parrot.DATA_VARIANT_ID`: TamableAnimal 18-19, variant 20 (Parrot.java).
    pub const PARROT: u8 = 20;
    /// `Fox.DATA_TYPE_ID`: variant 18, flags 19, then the two trusted ids 20-21 (Fox.java's
    /// field declaration order — `defineId` numbers the fields, `defineSynchedData` only
    /// gives defaults).
    pub const FOX_TYPE: u8 = 18;
    /// `Fox.DATA_FLAGS_ID` (sitting 1, crouching 4, interested 8, pouncing 16,
    /// sleeping 32, faceplanted 64, defending 128).
    pub const FOX_FLAGS: u8 = 19;
    /// `Panda.MAIN_GENE_ID`: unhappy 18, sneeze 19, eat 20, main gene 21, hidden gene 22,
    /// flags 23, eat counter 24 (Panda.java).
    pub const PANDA_MAIN_GENE: u8 = 21;
    /// `Panda.HIDDEN_GENE_ID`.
    pub const PANDA_HIDDEN_GENE: u8 = 22;
    /// `AbstractCubeMob.ID_SIZE` (slime, magma cube): AgeableMob 16-17, size 18.
    pub const CUBE_SIZE: u8 = 18;
    /// `Bee.DATA_FLAGS_ID`: flags 18 (nectar 8), anger end time 19.
    pub const BEE_FLAGS: u8 = 18;
    /// `Bee.DATA_ANGER_END_TIME` (game-time tick the anger ends, -1 when calm).
    pub const BEE_ANGER_END: u8 = 19;
    /// `Strider.DATA_SUFFOCATING`: boost time 18, suffocating (cold) 19.
    pub const STRIDER_SUFFOCATING: u8 = 19;
    /// `MushroomCow.DATA_TYPE`: variant 18 (red 0, brown 1; AbstractCow adds nothing).
    pub const MOOSHROOM_TYPE: u8 = 18;
    /// `Sheep.DATA_WOOL_ID`: AgeableMob 16-17, then the packed wool byte 18 (colour =
    /// `byte & 15`, sheared = `byte & 16`).
    pub const SHEEP_WOOL: u8 = 18;
    /// `Horse.DATA_ID_TYPE_VARIANT`'s markings nibble: `id >> 8` (Markings.NONE 0, WHITE 1,
    /// WHITE_FIELD 2, WHITE_DOTS 3, BLACK_DOTS 4).
    pub const HORSE_MARKINGS_SHIFT: u32 = 8;
    /// `Cat.DATA_COLLAR_COLOR`: variant 20, sound 21, lying 22, relax 23, collar 24; the
    /// collar shows when the cat is tamed (TamableAnimal flags 18, bit 4).
    pub const CAT_COLLAR: u8 = 24;
    /// `Wolf.DATA_COLLAR_COLOR`: interested 20, collar 21, anger 22, variant 23 (tamed =
    /// TamableAnimal flags 18, bit 4).
    pub const WOLF_COLLAR: u8 = 21;
    /// `Shulker.DATA_COLOR_ID`: attach face 16, peek 17, colour byte 18 (16 = none).
    pub const SHULKER_COLOR: u8 = 18;
    /// `TropicalFish.DATA_ID_TYPE_VARIANT`: AbstractFish from-bucket 16, then the packed
    /// variant 17 (pattern `id & 0xFFFF`, base colour `id >> 16`, pattern colour `id >> 24`).
    pub const TROPICAL_FISH_VARIANT: u8 = 17;
    /// `TamableAnimal.DATA_FLAGS_ID` (tamed = bit 4, sitting = bit 1).
    pub const TAME_FLAGS: u8 = 18;
    /// Tamed bit of [`TAME_FLAGS`].
    pub const TAME_FLAG_TAMED: i8 = 4;
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
    match name.as_str() {
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

/// The rest-pose scale of a slime or magma cube (`AbstractCubeMobRenderer.applySizeAndSquish`
/// with no squish: uniform `size`), or 1.0 for every other type. The squish animation's
/// non-uniform wobble is part of the animation task, not the variant.
///
/// Cheap on the frame path: the metadata probe comes first, so every non-cube entity (the
/// common case) returns without a registry lookup.
pub fn scale_for(kind: RegistryIndex, metadata: &Metadata) -> f32 {
    let size = match metadata.get(metadata_index::CUBE_SIZE).map(|e| &e.value) {
        Some(MetadataValue::VarInt(i)) if *i > 0 => *i,
        _ => return 1.0,
    };
    let Some(name) = crate::registry::entity_type_name(kind) else {
        return 1.0;
    };
    if !matches!(name.as_str(), "minecraft:slime" | "minecraft:magma_cube") {
        return 1.0;
    }
    size as f32
}

/// `DyeColor.getTextureDiffuseColor` (the third constructor argument), linear RGB in 0..1:
/// the tint vanilla multiplies into dyed overlay layers. Order is the dye id (white 0 ..
/// black 15).
pub const DYE_COLORS: [[f32; 3]; 16] = {
    // The vanilla constants are sRGB 0xRRGGBB; the vertex colour is a linear multiplier
    // sampled against an sRGB atlas, so the raw 8-bit channels are what vanilla passes on.
    let hex = [
        0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
        0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
    ];
    let mut out = [[0.0f32; 3]; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = [
            ((hex[i] >> 16) & 0xFF) as f32 / 255.0,
            ((hex[i] >> 8) & 0xFF) as f32 / 255.0,
            (hex[i] & 0xFF) as f32 / 255.0,
        ];
        i += 1;
    }
    out
};

/// The dye colour's linear tint (see [`DYE_COLORS`]); ids wrap like `DyeColor.byId`
/// (`ByIdMap.continuous` with the zero strategy, so any id outside 0..15 is white).
pub fn dye_tint(id: i32) -> [f32; 3] {
    DYE_COLORS[id.rem_euclid(16) as usize]
}

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
    match name.as_str() {
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

/// The tropical fish's base model: the shape its packed variant names (`Base.SMALL` uses
/// `tropical_fish_small` with `tropical_a`, `Base.LARGE` the large shape with `tropical_b`)
/// — `TropicalFishRenderer.submit` swaps the model with the pattern's base. The base colour
/// tint (`getModelTint`) is not applied yet: the base draw has no tint slot, a later
/// EntityInstance change; the pattern overlay carries its own colour today.
pub fn tropical_fish_model(metadata: &Metadata) -> Option<Arc<CubeModel>> {
    let packed = match metadata
        .get(metadata_index::TROPICAL_FISH_VARIANT)
        .map(|e| &e.value)
    {
        Some(MetadataValue::VarInt(i)) => *i,
        _ => return None,
    };
    let large = packed >> 8 & 0xFF == 1;
    let (layer, texture) = if large {
        (
            "minecraft:tropical_fish_large#main",
            "entity/fish/tropical_b",
        )
    } else {
        (
            "minecraft:tropical_fish_small#main",
            "entity/fish/tropical_a",
        )
    };
    dump_layer_with_texture(layer, texture)
}

/// The base texture of a shulker (`ShulkerRenderer.getTextureLocation`): the dye's sheet
/// sprite `entity/shulker/shulker_<name>`, or the plain `shulker` when the colour byte is
/// 16 (or absent).
pub fn shulker_texture(metadata: &Metadata) -> Option<Identifier> {
    let color = match metadata
        .get(metadata_index::SHULKER_COLOR)
        .map(|e| &e.value)
    {
        Some(MetadataValue::Byte(b)) => *b,
        _ => 16,
    };
    if color == 16 || !(0..=15).contains(&color) {
        return None;
    }
    const NAMES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    Some(Identifier::minecraft(&format!(
        "entity/shulker/shulker_{}",
        NAMES[color as usize]
    )))
}

/// `TropicalFishPatternLayer`'s texture per packed pattern id (`base | index << 8`).
fn tropical_pattern_texture(pattern: i32) -> String {
    let index = pattern & 0xFF;
    let large = pattern >> 8 & 1 == 1;
    let (letter, index) = if large { ("b", index) } else { ("a", index) };
    format!("entity/fish/tropical_{letter}_pattern_{}", index + 1)
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
fn dump_layer_with_texture(layer: &str, texture: &str) -> Option<Arc<CubeModel>> {
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
        crate::registry::baby_model_for_name(&name)
            .or_else(|| crate::registry::model_for_name(&name))?
    } else {
        crate::registry::model_for_name(&name)?
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
        crate::registry::baby_model_for_name(&name)
    } else {
        crate::registry::model_for_name(&name)
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

/// Test helper: apply another metadata set's entries on top.
#[cfg(test)]
trait ApplyExtra {
    fn apply_extra(self, other: Metadata) -> Metadata;
}
#[cfg(test)]
impl ApplyExtra for Metadata {
    fn apply_extra(mut self, other: Metadata) -> Metadata {
        use haste_core::entity::MetadataEntry;
        let entries: Vec<MetadataEntry> = other.iter().cloned().collect();
        self.apply(&entries);
        self
    }
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

    fn int_metadata(entries: &[(u8, i32)]) -> Metadata {
        let mut m = Metadata::default();
        m.apply(
            &entries
                .iter()
                .map(|(i, v)| MetadataEntry {
                    index: *i,
                    value: MetadataValue::VarInt(*v),
                })
                .collect::<Vec<_>>(),
        );
        m
    }

    fn byte_metadata(entries: &[(u8, i8)]) -> Metadata {
        let mut m = Metadata::default();
        m.apply(
            &entries
                .iter()
                .map(|(i, v)| MetadataEntry {
                    index: *i,
                    value: MetadataValue::Byte(*v),
                })
                .collect::<Vec<_>>(),
        );
        m
    }

    fn bool_metadata(entries: &[(u8, bool)]) -> Metadata {
        let mut m = Metadata::default();
        m.apply(
            &entries
                .iter()
                .map(|(i, v)| MetadataEntry {
                    index: *i,
                    value: MetadataValue::Boolean(*v),
                })
                .collect::<Vec<_>>(),
        );
        m
    }

    fn long_metadata(entries: &[(u8, i64)]) -> Metadata {
        let mut m = Metadata::default();
        m.apply(
            &entries
                .iter()
                .map(|(i, v)| MetadataEntry {
                    index: *i,
                    value: MetadataValue::VarLong(*v),
                })
                .collect::<Vec<_>>(),
        );
        m
    }

    /// The kind of an entity type name, through the real registries.json (None without the
    /// cache: these tests are skipped there).
    fn kind_of(name: &str) -> Option<RegistryIndex> {
        crate::registry::entity_type_names()?
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(k, _)| *k)
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
        assert_eq!(metadata_index::AXOLOTL, 18);
        assert_eq!(metadata_index::RABBIT, 18);
        assert_eq!(metadata_index::HORSE_TYPE_VARIANT, 19); // AbstractHorse flags 18
        assert_eq!(metadata_index::LLAMA_VARIANT, 21); // flags 18, chest 19, strength 20
        assert_eq!(metadata_index::PARROT, 20); // TamableAnimal 18-19
        assert_eq!(metadata_index::FOX_TYPE, 18); // then flags 19, trusted 20-21
        assert_eq!(metadata_index::FOX_FLAGS, 19);
        assert_eq!(metadata_index::PANDA_MAIN_GENE, 21); // unhappy 18, sneeze 19, eat 20
        assert_eq!(metadata_index::PANDA_HIDDEN_GENE, 22);
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

    /// The metadata-driven variant textures: each renderer's own table, including the
    /// fallbacks vanilla applies for unknown ids (and the killer rabbit's 99). Needs
    /// registries.json for the kind lookup.
    #[test]
    #[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
    fn metadata_variant_textures_match_the_renderers() {
        let cases: Vec<(&str, Metadata, bool, Option<&str>, &str)> = vec![
            // Axolotl: variant 3 (cyan), and an unknown id falls back to LUCY.
            (
                "minecraft:axolotl",
                int_metadata(&[(metadata_index::AXOLOTL, 3)]),
                false,
                None,
                "entity/axolotl/axolotl_cyan",
            ),
            (
                "minecraft:axolotl",
                int_metadata(&[(metadata_index::AXOLOTL, 9)]),
                true,
                None,
                "entity/axolotl/axolotl_lucy_baby",
            ),
            // Fox: snow variant sleeping (flags bit 32), baby.
            (
                "minecraft:fox",
                int_metadata(&[(metadata_index::FOX_TYPE, 1)])
                    .apply_extra(byte_metadata(&[(metadata_index::FOX_FLAGS, 32)])),
                true,
                None,
                "entity/fox/fox_snow_sleep_baby",
            ),
            // Rabbit: the killer rabbit (99), Toast's magic name, and the default.
            (
                "minecraft:rabbit",
                int_metadata(&[(metadata_index::RABBIT, 99)]),
                false,
                None,
                "entity/rabbit/rabbit_caerbannog",
            ),
            (
                "minecraft:rabbit",
                int_metadata(&[(metadata_index::RABBIT, 0)]),
                false,
                Some("Toast"),
                "entity/rabbit/rabbit_toast",
            ),
            (
                "minecraft:rabbit",
                int_metadata(&[(metadata_index::RABBIT, 42)]),
                true,
                None,
                "entity/rabbit/rabbit_brown_baby",
            ),
            // Horse: variant 4 (black).
            (
                "minecraft:horse",
                int_metadata(&[(metadata_index::HORSE_TYPE_VARIANT, 4)]),
                false,
                None,
                "entity/horse/horse_black",
            ),
            // Llama: variant 3 (gray), baby.
            (
                "minecraft:llama",
                int_metadata(&[(metadata_index::LLAMA_VARIANT, 3)]),
                true,
                None,
                "entity/llama/llama_gray_baby",
            ),
            // Parrot: variant 2 (green).
            (
                "minecraft:parrot",
                int_metadata(&[(metadata_index::PARROT, 2)]),
                false,
                None,
                "entity/parrot/parrot_green",
            ),
            // Panda: main brown (4) with hidden brown shows brown; with a different hidden
            // gene it shows normal.
            (
                "minecraft:panda",
                byte_metadata(&[
                    (metadata_index::PANDA_MAIN_GENE, 4),
                    (metadata_index::PANDA_HIDDEN_GENE, 4),
                ]),
                false,
                None,
                "entity/panda/brown_panda",
            ),
            (
                "minecraft:panda",
                byte_metadata(&[
                    (metadata_index::PANDA_MAIN_GENE, 4),
                    (metadata_index::PANDA_HIDDEN_GENE, 0),
                ]),
                true,
                None,
                "entity/panda/panda_baby",
            ),
            // Mooshroom: variant 1 (brown).
            (
                "minecraft:mooshroom",
                int_metadata(&[(metadata_index::MOOSHROOM_TYPE, 1)]),
                true,
                None,
                "entity/cow/mooshroom_brown_baby",
            ),
            // Strider: suffocating (cold) boolean at 19.
            (
                "minecraft:strider",
                Metadata::default().apply_extra(bool_metadata(&[(
                    metadata_index::STRIDER_SUFFOCATING,
                    true,
                )])),
                false,
                None,
                "entity/strider/strider_cold",
            ),
        ];
        for (name, meta, baby, custom, want) in cases {
            let Some(kind) = kind_of(name) else {
                eprintln!("skipped: {name} not in registries.json");
                return;
            };
            let got = texture_for(kind, &meta, baby, custom, 0).unwrap();
            assert_eq!(
                got,
                Identifier::minecraft(want),
                "{name} baby={baby} custom={custom:?}"
            );
        }
        // A type with no metadata variant: nothing.
        if let Some(pig) = kind_of("minecraft:pig") {
            assert!(texture_for(
                pig,
                &int_metadata(&[(metadata_index::RABBIT, 1)]),
                false,
                None,
                0
            )
            .is_none());
        }
    }

    /// The bee's texture: nectar (flags bit 8) x anger (end time past the world age) x baby
    /// (`BeeRenderer.getTextureLocation`); the anger end time in the past means calm.
    #[test]
    #[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
    fn bee_textures_angry_nectar_and_age() {
        let Some(bee) = kind_of("minecraft:bee") else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        let bee_meta = |flags: i8, anger_end: i64| {
            bool_metadata(&[(0, false)])
                .apply_extra(long_metadata(&[(metadata_index::BEE_ANGER_END, anger_end)]))
                .apply_extra(byte_metadata(&[(metadata_index::BEE_FLAGS, flags)]))
        };
        // Calm, no nectar.
        assert_eq!(
            texture_for(bee, &bee_meta(0, -1), false, None, 100).unwrap(),
            Identifier::minecraft("entity/bee/bee")
        );
        // Nectar only.
        assert_eq!(
            texture_for(bee, &bee_meta(8, -1), false, None, 100).unwrap(),
            Identifier::minecraft("entity/bee/bee_nectar")
        );
        // Angry (end time 200 > age 100), nectar, baby.
        assert_eq!(
            texture_for(bee, &bee_meta(8, 200), true, None, 100).unwrap(),
            Identifier::minecraft("entity/bee/bee_angry_nectar_baby")
        );
        // The anger already ended: calm.
        assert_eq!(
            texture_for(bee, &bee_meta(0, 50), false, None, 100).unwrap(),
            Identifier::minecraft("entity/bee/bee")
        );
        // Angry without nectar.
        assert_eq!(
            texture_for(bee, &bee_meta(0, 200), false, None, 100).unwrap(),
            Identifier::minecraft("entity/bee/bee_angry")
        );
    }

    /// The slime and magma cube scale: `AbstractCubeMob.ID_SIZE` at index 18, everything
    /// else 1.0.
    #[test]
    #[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
    fn cube_size_scales_the_model() {
        let Some(slime) = kind_of("minecraft:slime") else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        let size = |i: i32| int_metadata(&[(metadata_index::CUBE_SIZE, i)]);
        assert_eq!(scale_for(slime, &Metadata::default()), 1.0);
        assert_eq!(scale_for(slime, &size(4)), 4.0);
        // Non-positive sizes render at the minimum (vanilla never sends them).
        assert_eq!(scale_for(slime, &size(0)), 1.0);
        if let Some(pig) = kind_of("minecraft:pig") {
            assert_eq!(scale_for(pig, &size(4)), 1.0);
        }
    }

    /// A strider that is not suffocating keeps the plain texture (the boolean defaults
    /// false when absent).
    #[test]
    #[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
    fn strider_plain_when_not_suffocating() {
        let Some(strider) = kind_of("minecraft:strider") else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        assert_eq!(
            texture_for(strider, &Metadata::default(), false, None, 0).unwrap(),
            Identifier::minecraft("entity/strider/strider")
        );
    }

    /// `DyeColor.getTextureDiffuseColor` for a few dyes, as linear 0..1 triples
    /// (white, orange, black, red — the exact vanilla constants, channel / 255).
    #[test]
    fn dye_tints_match_vanilla() {
        assert_eq!(dye_tint(0), [249.0 / 255.0, 1.0, 254.0 / 255.0]); // white 0xF9FFFE
        assert_eq!(dye_tint(1), [249.0 / 255.0, 128.0 / 255.0, 29.0 / 255.0]); // orange 0xF9801D
        assert_eq!(dye_tint(15), [29.0 / 255.0, 29.0 / 255.0, 33.0 / 255.0]); // black 0x1D1D21
        assert_eq!(dye_tint(14), [176.0 / 255.0, 46.0 / 255.0, 38.0 / 255.0]); // red 0xB02E26
                                                                               // Out-of-range ids wrap to a real dye like `DyeColor.byId`.
        assert_eq!(dye_tint(16), dye_tint(0));
        assert_eq!(dye_tint(-1), dye_tint(15));
    }

    /// The sheep's overlays from the packed wool byte: colour in the low nibble, sheared
    /// bit 16; the undercoat only for non-white adults (SheepWoolUndercoatLayer).
    #[test]
    #[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
    fn sheep_overlays_from_the_wool_byte() {
        let Some(sheep) = kind_of("minecraft:sheep") else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        // Wool byte absent: no overlays (nothing to draw yet).
        assert!(overlays_for(sheep, &Metadata::default(), false)
            .iter()
            .all(|o| o.is_none()));
        // Red wool (colour 14): the wool layer and the undercoat, both tinted red.
        let red = byte_metadata(&[(metadata_index::SHEEP_WOOL, 14)]);
        let [wool, undercoat] = overlays_for(sheep, &red, false);
        let wool = wool.expect("red wool layer");
        assert_eq!(
            wool.model.texture,
            Identifier::minecraft("entity/sheep/sheep_wool")
        );
        assert_eq!(wool.tint, dye_tint(14));
        let undercoat = undercoat.expect("undercoat for non-white wool");
        assert_eq!(
            undercoat.model.texture,
            Identifier::minecraft("entity/sheep/sheep_wool_undercoat")
        );
        // White adult: the wool but no undercoat (the same white would be invisible).
        let white = byte_metadata(&[(metadata_index::SHEEP_WOOL, 0)]);
        let [wool, undercoat] = overlays_for(sheep, &white, false);
        assert!(wool.is_some());
        assert!(undercoat.is_none());
        // Sheared (bit 16): no wool, but the undercoat keeps a non-white sheep visible.
        let sheared_red = byte_metadata(&[(metadata_index::SHEEP_WOOL, 14 | 16)]);
        let [wool, undercoat] = overlays_for(sheep, &sheared_red, false);
        assert!(wool.is_none());
        assert!(undercoat.is_some());
        // A baby: wool only.
        let [wool, undercoat] = overlays_for(sheep, &red, true);
        assert!(wool.is_some());
        assert!(undercoat.is_none());
    }

    /// Collars show only for tamed mobs (TamableAnimal flags bit 4), with the collar dye's
    /// tint; the cat uses its collar layer, the wolf its own base model (WolfCollarLayer
    /// re-draws the parent model).
    #[test]
    #[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
    fn collars_only_when_tamed() {
        let tamed =
            || byte_metadata(&[(metadata_index::TAME_FLAGS, metadata_index::TAME_FLAG_TAMED)]);
        let Some(cat) = kind_of("minecraft:cat") else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        // Untamed: no collar.
        assert!(overlays_for(cat, &Metadata::default(), false)
            .iter()
            .all(|o| o.is_none()));
        // Tamed with a blue collar (11): the cat collar layer, tinted blue.
        let blue = tamed().apply_extra(int_metadata(&[(metadata_index::CAT_COLLAR, 11)]));
        let [collar, none] = overlays_for(cat, &blue, false);
        let collar = collar.expect("tamed cat collar");
        assert_eq!(
            collar.model.texture,
            Identifier::minecraft("entity/cat/cat_collar")
        );
        assert_eq!(collar.tint, dye_tint(11));
        assert!(none.is_none());
        // The wolf collar re-draws the wolf's own shape with the collar texture.
        let Some(wolf) = kind_of("minecraft:wolf") else {
            eprintln!("skipped: minecraft:wolf not in the registry");
            return;
        };
        let lime = tamed().apply_extra(int_metadata(&[(metadata_index::WOLF_COLLAR, 5)]));
        let [collar, _] = overlays_for(wolf, &lime, false);
        let collar = collar.expect("tamed wolf collar");
        assert_eq!(
            collar.model.texture,
            Identifier::minecraft("entity/wolf/wolf_collar")
        );
        assert_eq!(collar.tint, dye_tint(5));
    }

    /// The tropical fish: the packed variant picks the shape (small/large), the pattern
    /// texture and both dye tints; the shulker's colour byte picks its texture.
    #[test]
    #[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
    fn tropical_fish_and_shulker_from_packed_metadata() {
        // Packed variant: pattern flopper (LARGE base 1, index 0 -> 0x100), base colour
        // magenta (2), pattern colour yellow (4).
        let packed = 0x100 | 2 << 16 | 4 << 24;
        let fish_meta = int_metadata(&[(metadata_index::TROPICAL_FISH_VARIANT, packed)]);
        let model = tropical_fish_model(&fish_meta).expect("large fish model");
        assert_eq!(
            model.texture,
            Identifier::minecraft("entity/fish/tropical_b")
        );
        let Some(fish) = kind_of("minecraft:tropical_fish") else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        let [pattern, none] = overlays_for(fish, &fish_meta, false);
        let pattern = pattern.expect("pattern overlay");
        assert_eq!(
            pattern.model.texture,
            Identifier::minecraft("entity/fish/tropical_b_pattern_1")
        );
        assert_eq!(pattern.tint, dye_tint(4));
        assert!(none.is_none());
        // A small-shape pattern (kob: base 0, index 0) uses the small layer and tropical_a.
        let kob = int_metadata(&[(metadata_index::TROPICAL_FISH_VARIANT, 0)]);
        let model = tropical_fish_model(&kob).expect("small fish model");
        assert_eq!(
            model.texture,
            Identifier::minecraft("entity/fish/tropical_a")
        );
        // The shulker's colour byte: 16 (or absent) is the plain shulker, 0..15 the dyed
        // sprite.
        assert_eq!(shulker_texture(&Metadata::default()), None);
        assert_eq!(
            shulker_texture(&byte_metadata(&[(metadata_index::SHULKER_COLOR, 16)])),
            None
        );
        assert_eq!(
            shulker_texture(&byte_metadata(&[(metadata_index::SHULKER_COLOR, 9)])),
            Some(Identifier::minecraft("entity/shulker/shulker_cyan"))
        );
    }

    /// Horse markings: the same horse shape re-drawn with the markings texture, no tint;
    /// NONE (0) draws nothing.
    #[test]
    #[ignore = "needs cache/oracle/26.3 (registries.json + model dump; HASTE_CACHE overrides)"]
    fn horse_markings_layer() {
        let Some(horse) = kind_of("minecraft:horse") else {
            eprintln!("skipped: no registries.json in the cache");
            return;
        };
        // No variant int: nothing (also what Markings.NONE gives).
        assert!(overlays_for(horse, &Metadata::default(), false)
            .iter()
            .all(|o| o.is_none()));
        // Variant 4 (black coat) with black dots markings (4 << 8).
        let meta = int_metadata(&[(metadata_index::HORSE_TYPE_VARIANT, 4 | 4 << 8)]);
        let [markings, none] = overlays_for(horse, &meta, false);
        let markings = markings.expect("markings layer");
        assert_eq!(
            markings.model.texture,
            Identifier::minecraft("entity/horse/horse_markings_blackdots")
        );
        assert_eq!(markings.tint, [1.0; 3]);
        assert!(none.is_none());
    }
}
