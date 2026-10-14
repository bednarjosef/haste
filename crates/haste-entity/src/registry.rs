//! Entity-type -> (model layer, texture) table, mirroring vanilla's entity renderers.
//!
//! One row per entity type: the `ModelLayers` constant its vanilla renderer bakes, the texture
//! path that renderer returns, and the root scale when vanilla's layer definition itself was
//! built with a `MeshTransformer.scaling(f)` (the dump cannot carry it; see `model_dump`).
//! Adult, default variant only for now: babies, pig/cow/chicken temperature variants, sheep
//! wool, villager professions and armour overlays are later tasks (the dump has all of them;
//! the rows simply point at the default layer and texture).
//!
//! Rows name the vanilla renderer they were read from (26.3 decompiled source,
//! `net.minecraft.client.renderer.entity`); entries without a row stay on the generic box.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::ids::RegistryIndex;
use haste_core::model::CubeModel;
use haste_core::Identifier;
use serde::Deserialize;

use crate::model_dump;

/// One table row: layer id (`minecraft:pig#main`), texture (a jar path under
/// `assets/minecraft/textures/`, without the prefix), and the layer's root scale.
struct Entry {
    layer: &'static str,
    texture: &'static str,
    root_scale: f32,
}

const fn row(layer: &'static str, texture: &'static str) -> Entry {
    Entry {
        layer,
        texture,
        root_scale: 1.0,
    }
}

const fn row_scaled(layer: &'static str, texture: &'static str, root_scale: f32) -> Entry {
    Entry {
        layer,
        texture,
        root_scale,
    }
}

/// The table. Textures and layers read from each vanilla renderer named in the comments
/// (`cache/source/26.3/client/net/minecraft/client/renderer/entity/`).
static TABLE: &[(&str, Entry)] = &[
    // AllayRenderer: ModelLayers.ALLAY, textures/entity/allay/allay.png
    (
        "minecraft:allay",
        row("minecraft:allay#main", "entity/allay/allay"),
    ),
    // ArmadilloRenderer: ModelLayers.ARMADILLO, textures/entity/armadillo/armadillo.png
    (
        "minecraft:armadillo",
        row("minecraft:armadillo#main", "entity/armadillo/armadillo"),
    ),
    // BatRenderer: ModelLayers.BAT, textures/entity/bat/bat.png
    ("minecraft:bat", row("minecraft:bat#main", "entity/bat/bat")),
    // BlazeRenderer: ModelLayers.BLAZE, textures/entity/blaze/blaze.png
    (
        "minecraft:blaze",
        row("minecraft:blaze#main", "entity/blaze/blaze"),
    ),
    // BoggedRenderer: ModelLayers.BOGGED, textures/entity/skeleton/bogged.png
    (
        "minecraft:bogged",
        row("minecraft:bogged#main", "entity/skeleton/bogged"),
    ),
    // BreezeRenderer: ModelLayers.BREEZE, textures/entity/breeze/breeze.png
    (
        "minecraft:breeze",
        row("minecraft:breeze#main", "entity/breeze/breeze"),
    ),
    // CaveSpiderRenderer: ModelLayers.CAVE_SPIDER (spider layer scaled 0.7 in LayerDefinitions),
    //   textures/entity/spider/cave_spider.png
    (
        "minecraft:cave_spider",
        row("minecraft:cave_spider#main", "entity/spider/cave_spider"),
    ),
    // ChickenRenderer: ModelLayers.CHICKEN (temperate variant), entity/chicken/chicken_temperate
    (
        "minecraft:chicken",
        row("minecraft:chicken#main", "entity/chicken/chicken_temperate"),
    ),
    // CodRenderer: ModelLayers.COD, textures/entity/fish/cod.png
    (
        "minecraft:cod",
        row("minecraft:cod#main", "entity/fish/cod"),
    ),
    // CowRenderer: ModelLayers.COW (temperate variant), entity/cow/cow_temperate
    (
        "minecraft:cow",
        row("minecraft:cow#main", "entity/cow/cow_temperate"),
    ),
    // CreeperRenderer: ModelLayers.CREEPER, textures/entity/creeper/creeper.png
    (
        "minecraft:creeper",
        row("minecraft:creeper#main", "entity/creeper/creeper"),
    ),
    // CreakingRenderer: ModelLayers.CREAKING, textures/entity/creaking/creaking.png
    (
        "minecraft:creaking",
        row("minecraft:creaking#main", "entity/creaking/creaking"),
    ),
    // DrownedRenderer: ModelLayers.DROWNED, textures/entity/zombie/drowned.png
    (
        "minecraft:drowned",
        row("minecraft:drowned#main", "entity/zombie/drowned"),
    ),
    // EndermanRenderer: ModelLayers.ENDERMAN, textures/entity/enderman/enderman.png
    (
        "minecraft:enderman",
        row("minecraft:enderman#main", "entity/enderman/enderman"),
    ),
    // EndermiteRenderer: ModelLayers.ENDERMITE, textures/entity/endermite/endermite.png
    (
        "minecraft:endermite",
        row("minecraft:endermite#main", "entity/endermite/endermite"),
    ),
    // EvokerRenderer: ModelLayers.EVOKER (illager layer, 0.9375 in LayerDefinitions),
    //   textures/entity/illager/evoker.png
    (
        "minecraft:evoker",
        row_scaled("minecraft:evoker#main", "entity/illager/evoker", 0.9375),
    ),
    // GhastRenderer: ModelLayers.GHAST (layer scaled 4.5), textures/entity/ghast/ghast.png
    (
        "minecraft:ghast",
        row_scaled("minecraft:ghast#main", "entity/ghast/ghast", 4.5),
    ),
    // GiantMobRenderer: ModelLayers.GIANT (humanoid layer scaled 6.0), textures/entity/zombie/zombie.png
    (
        "minecraft:giant",
        row_scaled("minecraft:giant#main", "entity/zombie/zombie", 6.0),
    ),
    // GlowSquidRenderer: ModelLayers.GLOW_SQUID, textures/entity/squid/glow_squid.png
    (
        "minecraft:glow_squid",
        row("minecraft:glow_squid#main", "entity/squid/glow_squid"),
    ),
    // GuardianRenderer: ModelLayers.GUARDIAN, textures/entity/guardian/guardian.png
    (
        "minecraft:guardian",
        row("minecraft:guardian#main", "entity/guardian/guardian"),
    ),
    // HappyGhastRenderer: ModelLayers.HAPPY_GHAST (layer scaled 4.0), textures/entity/ghast/happy_ghast.png
    (
        "minecraft:happy_ghast",
        row_scaled(
            "minecraft:happy_ghast#main",
            "entity/ghast/happy_ghast",
            4.0,
        ),
    ),
    // HoglinRenderer: ModelLayers.HOGLIN, textures/entity/hoglin/hoglin.png
    (
        "minecraft:hoglin",
        row("minecraft:hoglin#main", "entity/hoglin/hoglin"),
    ),
    // HuskRenderer: ModelLayers.HUSK (humanoid layer scaled 1.0625), textures/entity/zombie/husk.png
    (
        "minecraft:husk",
        row_scaled("minecraft:husk#main", "entity/zombie/husk", 1.0625),
    ),
    // IllusionerRenderer: ModelLayers.ILLUSIONER (illager layer, 0.9375), textures/entity/illager/illusioner.png
    (
        "minecraft:illusioner",
        row_scaled(
            "minecraft:illusioner#main",
            "entity/illager/illusioner",
            0.9375,
        ),
    ),
    // IronGolemRenderer: ModelLayers.IRON_GOLEM, textures/entity/iron_golem/iron_golem.png
    (
        "minecraft:iron_golem",
        row("minecraft:iron_golem#main", "entity/iron_golem/iron_golem"),
    ),
    // MagmaCubeRenderer: ModelLayers.MAGMA_CUBE, textures/entity/slime/magmacube.png
    (
        "minecraft:magma_cube",
        row("minecraft:magma_cube#main", "entity/slime/magmacube"),
    ),
    // MushroomCowRenderer: ModelLayers.MOOSHROOM (cow layer),
    //   textures/entity/cow/mooshroom_red.png (red is the default variant)
    (
        "minecraft:mooshroom",
        row("minecraft:mooshroom#main", "entity/cow/mooshroom_red"),
    ),
    // OcelotRenderer: ModelLayers.OCELOT, textures/entity/cat/ocelot.png
    (
        "minecraft:ocelot",
        row("minecraft:ocelot#main", "entity/cat/ocelot"),
    ),
    // ElderGuardianRenderer: ModelLayers.ELDER_GUARDIAN (guardian layer scaled 2.35),
    //   textures/entity/guardian/guardian_elder.png
    (
        "minecraft:elder_guardian",
        row_scaled(
            "minecraft:elder_guardian#main",
            "entity/guardian/guardian_elder",
            2.35,
        ),
    ),
    // ParrotRenderer: ModelLayers.PARROT, textures/entity/parrot/parrot_red_blue.png (default variant)
    (
        "minecraft:parrot",
        row("minecraft:parrot#main", "entity/parrot/parrot_red_blue"),
    ),
    // ParchedRenderer: ModelLayers.PARCHED (dual-body skeleton layer), textures/entity/skeleton/parched.png
    (
        "minecraft:parched",
        row("minecraft:parched#main", "entity/skeleton/parched"),
    ),
    // PhantomRenderer: ModelLayers.PHANTOM, textures/entity/phantom/phantom.png
    (
        "minecraft:phantom",
        row("minecraft:phantom#main", "entity/phantom/phantom"),
    ),
    // PigRenderer: ModelLayers.PIG (temperate variant), entity/pig/pig_temperate
    (
        "minecraft:pig",
        row("minecraft:pig#main", "entity/pig/pig_temperate"),
    ),
    // PiglinRenderer: ModelLayers.PIGLIN, textures/entity/piglin/piglin.png
    (
        "minecraft:piglin",
        row("minecraft:piglin#main", "entity/piglin/piglin"),
    ),
    // PillagerRenderer: ModelLayers.PILLAGER (illager layer, 0.9375), textures/entity/illager/pillager.png
    (
        "minecraft:pillager",
        row_scaled("minecraft:pillager#main", "entity/illager/pillager", 0.9375),
    ),
    // RavagerRenderer: ModelLayers.RAVAGER, textures/entity/illager/ravager.png
    (
        "minecraft:ravager",
        row("minecraft:ravager#main", "entity/illager/ravager"),
    ),
    // SheepRenderer: ModelLayers.SHEEP, textures/entity/sheep/sheep.png
    (
        "minecraft:sheep",
        row("minecraft:sheep#main", "entity/sheep/sheep"),
    ),
    // SilverfishRenderer: ModelLayers.SILVERFISH, textures/entity/silverfish/silverfish.png
    (
        "minecraft:silverfish",
        row("minecraft:silverfish#main", "entity/silverfish/silverfish"),
    ),
    // SkeletonRenderer: ModelLayers.SKELETON, textures/entity/skeleton/skeleton.png
    (
        "minecraft:skeleton",
        row("minecraft:skeleton#main", "entity/skeleton/skeleton"),
    ),
    // SlimeRenderer: ModelLayers.SLIME (inner body), textures/entity/slime/slime.png
    (
        "minecraft:slime",
        row("minecraft:slime#main", "entity/slime/slime"),
    ),
    // SnowGolemRenderer: ModelLayers.SNOW_GOLEM, textures/entity/snow_golem/snow_golem.png
    (
        "minecraft:snow_golem",
        row("minecraft:snow_golem#main", "entity/snow_golem/snow_golem"),
    ),
    // SpiderRenderer: ModelLayers.SPIDER, textures/entity/spider/spider.png
    (
        "minecraft:spider",
        row("minecraft:spider#main", "entity/spider/spider"),
    ),
    // SquidRenderer: ModelLayers.SQUID, textures/entity/squid/squid.png
    (
        "minecraft:squid",
        row("minecraft:squid#main", "entity/squid/squid"),
    ),
    // StrayRenderer: ModelLayers.STRAY, textures/entity/skeleton/stray.png
    (
        "minecraft:stray",
        row("minecraft:stray#main", "entity/skeleton/stray"),
    ),
    // StriderRenderer: ModelLayers.STRIDER, textures/entity/strider/strider.png
    (
        "minecraft:strider",
        row("minecraft:strider#main", "entity/strider/strider"),
    ),
    // VillagerRenderer: ModelLayers.VILLAGER (villager layer scaled 0.9375),
    //   textures/entity/villager/villager.png
    (
        "minecraft:villager",
        row_scaled(
            "minecraft:villager#main",
            "entity/villager/villager",
            0.9375,
        ),
    ),
    // VindicatorRenderer: ModelLayers.VINDICATOR (illager layer, 0.9375), textures/entity/illager/vindicator.png
    (
        "minecraft:vindicator",
        row_scaled(
            "minecraft:vindicator#main",
            "entity/illager/vindicator",
            0.9375,
        ),
    ),
    // WanderingTraderRenderer: ModelLayers.WANDERING_TRADER (villager layer scaled 0.9375),
    //   textures/entity/wandering_trader/wandering_trader.png
    (
        "minecraft:wandering_trader",
        row_scaled(
            "minecraft:wandering_trader#main",
            "entity/wandering_trader/wandering_trader",
            0.9375,
        ),
    ),
    // WitchRenderer: ModelLayers.WITCH (witch layer scaled 0.9375), textures/entity/witch/witch.png
    (
        "minecraft:witch",
        row_scaled("minecraft:witch#main", "entity/witch/witch", 0.9375),
    ),
    // WitherBossRenderer: ModelLayers.WITHER, textures/entity/wither/wither.png
    (
        "minecraft:wither",
        row("minecraft:wither#main", "entity/wither/wither"),
    ),
    // WitherSkeletonRenderer: ModelLayers.WITHER_SKELETON (skeleton layer scaled 1.2),
    //   textures/entity/skeleton/wither_skeleton.png
    (
        "minecraft:wither_skeleton",
        row_scaled(
            "minecraft:wither_skeleton#main",
            "entity/skeleton/wither_skeleton",
            1.2,
        ),
    ),
    // ZoglinRenderer: ModelLayers.ZOGLIN, textures/entity/hoglin/zoglin.png
    (
        "minecraft:zoglin",
        row("minecraft:zoglin#main", "entity/hoglin/zoglin"),
    ),
    // ZombieRenderer: ModelLayers.ZOMBIE, textures/entity/zombie/zombie.png
    (
        "minecraft:zombie",
        row("minecraft:zombie#main", "entity/zombie/zombie"),
    ),
    // ZombieVillagerRenderer: ModelLayers.ZOMBIE_VILLAGER, textures/entity/zombie_villager/zombie_villager.png
    (
        "minecraft:zombie_villager",
        row(
            "minecraft:zombie_villager#main",
            "entity/zombie_villager/zombie_villager",
        ),
    ),
    // ZombifiedPiglinRenderer: ModelLayers.ZOMBIFIED_PIGLIN, textures/entity/piglin/zombified_piglin.png
    (
        "minecraft:zombified_piglin",
        row(
            "minecraft:zombified_piglin#main",
            "entity/piglin/zombified_piglin",
        ),
    ),
];

/// The model for entity type `kind`, from the dump. `None` when the type is not in the table,
/// the registry name cannot be resolved, the dump is missing or the dump lacks the layer:
/// callers fall back to the dimension box. Results are cached per type (models stay shared
/// `Arc`s); warm lookups are a map probe, never an allocation.
pub fn model_for(kind: RegistryIndex) -> Option<Arc<CubeModel>> {
    model_entry(kind).map(|(_, m)| m)
}

/// The per-type lookup result: whether the model has a "head" part (for the head aim) and the
/// shared model. Cached per type; warm lookups are a map probe, never an allocation.
/// The cached per-type entry: the "head part exists" flag and the shared model.
type CachedEntry = Option<(bool, Arc<CubeModel>)>;

pub fn model_entry(kind: RegistryIndex) -> Option<(bool, Arc<CubeModel>)> {
    // Two-level cache: the name map is read once from the reports; models per type are filled
    // lazily (first spawn of that type) and shared from then on.
    static CACHE: OnceLock<Mutex<HashMap<RegistryIndex, CachedEntry>>> = OnceLock::new();
    static NAMES: OnceLock<HashMap<RegistryIndex, String>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let names = NAMES.get_or_init(|| load_entity_type_names().unwrap_or_default());
    if let Some(hit) = cache.lock().unwrap().get(&kind) {
        return hit.clone();
    }
    let name = names.get(&kind)?;
    let entry = model_for_name(name).map(|m| (has_head(&m.root), m));
    cache.lock().unwrap().insert(kind, entry.clone());
    entry
}

/// Whether the tree contains a part named "head" (computed once per type, not per frame).
fn has_head(part: &haste_core::model::ModelPart) -> bool {
    part.name == "head" || part.children.iter().any(has_head)
}

/// The table entry for `name`: (layer id, texture path, root scale) for tests and callers
/// that want the mapping without the dump.
pub fn table_entry(name: &str) -> Option<(&'static str, f32)> {
    let (_, e) = TABLE.iter().find(|(n, _)| *n == name)?;
    Some((e.layer, e.root_scale))
}

/// Every texture path in the table (unique, table order), for the jar-presence check the
/// renderer runs (a typo here would only show as placeholder boxes at runtime).
pub fn table_textures() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|(_, e)| e.texture)
}

/// `minecraft:entity_type` names by protocol id, from the data-generator reports
/// (`haste_core::paths::datagen_reports`, owned by world-and-data). haste-entity reads the one
/// registry it needs itself: haste-data's `Registries::parse` is not exported and pulling all
/// of `GameData` in here would duplicate the client's load. Only the entity ids the table
/// covers are kept.
fn load_entity_type_names() -> Option<HashMap<RegistryIndex, String>> {
    let path = haste_core::paths::datagen_reports().join("registries.json");
    let bytes = std::fs::read(path).ok()?;
    let registries: HashMap<&str, EntityTypeRegistryJson> = serde_json::from_slice(&bytes).ok()?;
    let reg = registries.get("minecraft:entity_type")?;
    let mut names = HashMap::with_capacity(reg.entries.len());
    for (name, e) in &reg.entries {
        if TABLE.iter().any(|(n, _)| n == name) {
            names.insert(RegistryIndex(e.protocol_id), (*name).to_owned());
        }
    }
    Some(names)
}

#[derive(Deserialize)]
struct EntityTypeRegistryJson {
    entries: HashMap<String, EntryId>,
}

#[derive(Deserialize)]
struct EntryId {
    protocol_id: u32,
}

/// The model for entity type `name` (a full registry id like `minecraft:pig`), from the dump.
/// `None` when the type is not in the table, the dump is missing, or the dump lacks the layer.
pub fn model_for_name(name: &str) -> Option<Arc<CubeModel>> {
    let (_, entry) = TABLE.iter().find(|(n, _)| *n == name)?;
    let lib = model_dump::library()?;
    let model = lib.get(entry.layer, entry.root_scale)?;
    // Replace the placeholder texture with the renderer's (the layer's model is shared, so a
    // clone is taken once per type here, then cached by `model_for`).
    let texture = Identifier::minecraft(entry.texture);
    if model.texture == texture {
        return Some(model);
    }
    let mut m = (*model).clone();
    m.texture = texture;
    Some(Arc::new(m))
}
