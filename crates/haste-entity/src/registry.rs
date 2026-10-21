//! Entity-type -> (model layer, texture) table, mirroring vanilla's entity renderers.
//!
//! One row per entity type: the `ModelLayers` constant its vanilla renderer bakes, the texture
//! path that renderer returns, and the root scale when vanilla's layer definition itself was
//! built with a `MeshTransformer.scaling(f)` (the dump cannot carry it; see `model_dump`).
//! Adult default variant first, then the baby layer/texture vanilla's `AgeableMobRenderer`
//! picks from `AgeableMob.DATA_BABY_ID` (metadata index 16; see `baby_metadata_index`).
//!
//! Rows name the vanilla renderer they were read from (26.3 decompiled source,
//! `net.minecraft.client.renderer.entity`); entries without a row stay on the generic box.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::entity::MetadataValue;
use haste_core::ids::RegistryIndex;
use haste_core::model::CubeModel;
use haste_core::Identifier;
use serde::Deserialize;

use crate::model_dump;

/// The renderers' shadow radius and strength by entity type (`EntityRenderer.shadowRadius`).
mod shadow;
pub use shadow::{shadow_radius_for_name, shadow_strength_for_name};

mod decoration_kind;
pub use decoration_kind::{decoration_kind, DecorationKind};

mod row;
use row::{row, row_baby, row_scaled, Entry};

/// The table. Textures and layers read from each vanilla renderer named in the comments
/// (`cache/source/26.3/client/net/minecraft/client/renderer/entity/`).
static TABLE: &[(&str, Entry)] = &[
    // AllayRenderer: ModelLayers.ALLAY, textures/entity/allay/allay.png
    (
        "minecraft:allay",
        row("minecraft:allay#main", "entity/allay/allay"),
    ),
    // ArmadilloRenderer: ModelLayers.ARMADILLO / ARMADILLO_BABY,
    //   textures/entity/armadillo/armadillo.png / armadillo_baby.png
    (
        "minecraft:armadillo",
        row_baby(
            "minecraft:armadillo#main",
            "entity/armadillo/armadillo",
            "minecraft:armadillo_baby#main",
            "entity/armadillo/armadillo_baby",
        ),
    ),
    // AxolotlRenderer: ModelLayers.AXOLOTL / AXOLOTL_BABY, textures/entity/axolotl/axolotl_lucy.png
    //   (default variant LUCY, the first enum constant; AxolotlRenderer builds textures from
    //   the variant name: axolotl_<name>.png / axolotl_<name>_baby.png)
    (
        "minecraft:axolotl",
        row_baby(
            "minecraft:axolotl#main",
            "entity/axolotl/axolotl_lucy",
            "minecraft:axolotl_baby#main",
            "entity/axolotl/axolotl_lucy_baby",
        ),
    ),
    // BatRenderer: ModelLayers.BAT, textures/entity/bat/bat.png
    ("minecraft:bat", row("minecraft:bat#main", "entity/bat/bat")),
    // BeeRenderer: ModelLayers.BEE / BEE_BABY, textures/entity/bee/bee.png / bee_baby.png
    //   (angry/nectar combinations are state textures, later task)
    (
        "minecraft:bee",
        row_baby(
            "minecraft:bee#main",
            "entity/bee/bee",
            "minecraft:bee_baby#main",
            "entity/bee/bee_baby",
        ),
    ),
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
    // CamelRenderer: ModelLayers.CAMEL / CAMEL_BABY, textures/entity/camel/camel.png / camel_baby.png
    (
        "minecraft:camel",
        row_baby(
            "minecraft:camel#main",
            "entity/camel/camel",
            "minecraft:camel_baby#main",
            "entity/camel/camel_baby",
        ),
    ),
    // CamelHuskRenderer: ModelLayers.CAMEL (adult only), textures/entity/camel/camel_husk.png
    (
        "minecraft:camel_husk",
        row("minecraft:camel#main", "entity/camel/camel_husk"),
    ),
    // CatRenderer: ModelLayers.CAT / CAT_BABY, texture from the CatVariant registry
    //   (default variant BLACK: entity/cat/cat_black.png / cat_black_baby.png; per-variant later)
    (
        "minecraft:cat",
        row_baby(
            "minecraft:cat#main",
            "entity/cat/cat_black",
            "minecraft:cat_baby#main",
            "entity/cat/cat_black_baby",
        ),
    ),
    // CaveSpiderRenderer: ModelLayers.CAVE_SPIDER (spider layer scaled 0.7 in LayerDefinitions),
    //   textures/entity/spider/cave_spider.png
    (
        "minecraft:cave_spider",
        row("minecraft:cave_spider#main", "entity/spider/cave_spider"),
    ),
    // ChickenRenderer: ModelLayers.CHICKEN / CHICKEN_BABY (temperate variant),
    //   entity/chicken/chicken_temperate / _baby (baby uses the shared chicken baby layer)
    (
        "minecraft:chicken",
        row_baby(
            "minecraft:chicken#main",
            "entity/chicken/chicken_temperate",
            "minecraft:chicken_baby#main",
            "entity/chicken/chicken_temperate_baby",
        ),
    ),
    // CodRenderer: ModelLayers.COD, textures/entity/fish/cod.png
    (
        "minecraft:cod",
        row("minecraft:cod#main", "entity/fish/cod"),
    ),
    // CopperGolemRenderer: ModelLayers.COPPER_GOLEM,
    //   textures/entity/copper_golem/copper_golem.png (CopperGolemOxidationLevels.UNAFFECTED)
    (
        "minecraft:copper_golem",
        row(
            "minecraft:copper_golem#main",
            "entity/copper_golem/copper_golem",
        ),
    ),
    // CowRenderer: ModelLayers.COW / COW_BABY (temperate variant), entity/cow/cow_temperate / _baby
    (
        "minecraft:cow",
        row_baby(
            "minecraft:cow#main",
            "entity/cow/cow_temperate",
            "minecraft:cow_baby#main",
            "entity/cow/cow_temperate_baby",
        ),
    ),
    // CreakingRenderer: ModelLayers.CREAKING, textures/entity/creaking/creaking.png
    (
        "minecraft:creaking",
        row("minecraft:creaking#main", "entity/creaking/creaking"),
    ),
    // CreeperRenderer: ModelLayers.CREEPER, textures/entity/creeper/creeper.png
    (
        "minecraft:creeper",
        row("minecraft:creeper#main", "entity/creeper/creeper"),
    ),
    // DolphinRenderer: ModelLayers.DOLPHIN / DOLPHIN_BABY,
    //   textures/entity/dolphin/dolphin.png / dolphin_baby.png
    (
        "minecraft:dolphin",
        row_baby(
            "minecraft:dolphin#main",
            "entity/dolphin/dolphin",
            "minecraft:dolphin_baby#main",
            "entity/dolphin/dolphin_baby",
        ),
    ),
    // DonkeyRenderer: ModelLayers.DONKEY / DONKEY_BABY,
    //   textures/entity/horse/donkey.png / donkey_baby.png
    (
        "minecraft:donkey",
        row_baby(
            "minecraft:donkey#main",
            "entity/horse/donkey",
            "minecraft:donkey_baby#main",
            "entity/horse/donkey_baby",
        ),
    ),
    // DrownedRenderer: ModelLayers.DROWNED / DROWNED_BABY,
    //   textures/entity/zombie/drowned.png / drowned_baby.png
    (
        "minecraft:drowned",
        row_baby(
            "minecraft:drowned#main",
            "entity/zombie/drowned",
            "minecraft:drowned_baby#main",
            "entity/zombie/drowned_baby",
        ),
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
    // EnderDragonRenderer: ModelLayers.ENDER_DRAGON, textures/entity/enderdragon/dragon.png
    //   (the whole-body historical-rotation setup and beam/eyes passes are later tasks; the
    //   dumped rest pose is drawn at the entity's yaw)
    (
        "minecraft:ender_dragon",
        row("minecraft:ender_dragon#main", "entity/enderdragon/dragon"),
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
    // FoxRenderer: ModelLayers.FOX / FOX_BABY, textures/entity/fox/fox.png / fox_baby.png
    //   (red variant is the default; snow/sleep textures later)
    (
        "minecraft:fox",
        row_baby(
            "minecraft:fox#main",
            "entity/fox/fox",
            "minecraft:fox_baby#main",
            "entity/fox/fox_baby",
        ),
    ),
    // FrogRenderer: ModelLayers.FROG, texture from the FrogVariant registry (default
    //   TEMPERATE: entity/frog/frog_temperate.png; per-variant later)
    (
        "minecraft:frog",
        row("minecraft:frog#main", "entity/frog/frog_temperate"),
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
    // GlowSquidRenderer: ModelLayers.GLOW_SQUID / GLOW_SQUID_BABY,
    //   textures/entity/squid/glow_squid.png / glow_squid_baby.png
    (
        "minecraft:glow_squid",
        row_baby(
            "minecraft:glow_squid#main",
            "entity/squid/glow_squid",
            "minecraft:glow_squid_baby#main",
            "entity/squid/glow_squid_baby",
        ),
    ),
    // GoatRenderer: ModelLayers.GOAT / GOAT_BABY, textures/entity/goat/goat.png / goat_baby.png
    (
        "minecraft:goat",
        row_baby(
            "minecraft:goat#main",
            "entity/goat/goat",
            "minecraft:goat_baby#main",
            "entity/goat/goat_baby",
        ),
    ),
    // GuardianRenderer: ModelLayers.GUARDIAN, textures/entity/guardian/guardian.png
    (
        "minecraft:guardian",
        row("minecraft:guardian#main", "entity/guardian/guardian"),
    ),
    // HappyGhastRenderer: ModelLayers.HAPPY_GHAST / HAPPY_GHAST_BABY (layer scaled 4.0),
    //   textures/entity/ghast/happy_ghast.png / happy_ghast_baby.png
    (
        "minecraft:happy_ghast",
        Entry {
            layer: "minecraft:happy_ghast#main",
            texture: "entity/ghast/happy_ghast",
            root_scale: 4.0,
            baby: Some((
                "minecraft:happy_ghast_baby#main",
                "entity/ghast/happy_ghast_baby",
            )),
        },
    ),
    // HoglinRenderer: ModelLayers.HOGLIN / HOGLIN_BABY,
    //   textures/entity/hoglin/hoglin.png / hoglin_baby.png
    (
        "minecraft:hoglin",
        row_baby(
            "minecraft:hoglin#main",
            "entity/hoglin/hoglin",
            "minecraft:hoglin_baby#main",
            "entity/hoglin/hoglin_baby",
        ),
    ),
    // HorseRenderer: ModelLayers.HORSE / HORSE_BABY,
    //   textures/entity/horse/horse_white.png / horse_white_baby.png (variant 0 = WHITE)
    (
        "minecraft:horse",
        row_baby(
            "minecraft:horse#main",
            "entity/horse/horse_white",
            "minecraft:horse_baby#main",
            "entity/horse/horse_white_baby",
        ),
    ),
    // HuskRenderer: ModelLayers.HUSK / HUSK_BABY (humanoid layer scaled 1.0625),
    //   textures/entity/zombie/husk.png / husk_baby.png
    (
        "minecraft:husk",
        Entry {
            layer: "minecraft:husk#main",
            texture: "entity/zombie/husk",
            root_scale: 1.0625,
            baby: Some(("minecraft:husk_baby#main", "entity/zombie/husk_baby")),
        },
    ),
    // IllusionerRenderer: ModelLayers.ILLUSIONER (illager layer, 0.9375),
    //   textures/entity/illager/illusioner.png
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
    // LlamaRenderer: ModelLayers.LLAMA / LLAMA_BABY,
    //   textures/entity/llama/llama_creamy.png / llama_creamy_baby.png (default variant CREAMY)
    (
        "minecraft:llama",
        row_baby(
            "minecraft:llama#main",
            "entity/llama/llama_creamy",
            "minecraft:llama_baby#main",
            "entity/llama/llama_creamy_baby",
        ),
    ),
    // MagmaCubeRenderer: ModelLayers.MAGMA_CUBE, textures/entity/slime/magmacube.png
    //   (size scaling from metadata is a later task)
    (
        "minecraft:magma_cube",
        row("minecraft:magma_cube#main", "entity/slime/magmacube"),
    ),
    // MooshroomRenderer: ModelLayers.MOOSHROOM / MOOSHROOM_BABY (cow layer),
    //   textures/entity/cow/mooshroom_red.png / mooshroom_red_baby.png (default variant RED)
    (
        "minecraft:mooshroom",
        row_baby(
            "minecraft:mooshroom#main",
            "entity/cow/mooshroom_red",
            "minecraft:mooshroom_baby#main",
            "entity/cow/mooshroom_red_baby",
        ),
    ),
    // MuleRenderer: ModelLayers.MULE / MULE_BABY,
    //   textures/entity/horse/mule.png / mule_baby.png
    (
        "minecraft:mule",
        row_baby(
            "minecraft:mule#main",
            "entity/horse/mule",
            "minecraft:mule_baby#main",
            "entity/horse/mule_baby",
        ),
    ),
    // NautilusRenderer: ModelLayers.NAUTILUS / NAUTILUS_BABY,
    //   textures/entity/nautilus/nautilus.png / nautilus_baby.png
    (
        "minecraft:nautilus",
        row_baby(
            "minecraft:nautilus#main",
            "entity/nautilus/nautilus",
            "minecraft:nautilus_baby#main",
            "entity/nautilus/nautilus_baby",
        ),
    ),
    // OcelotRenderer: ModelLayers.OCELOT / OCELOT_BABY,
    //   textures/entity/cat/ocelot.png / ocelot_baby.png
    (
        "minecraft:ocelot",
        row_baby(
            "minecraft:ocelot#main",
            "entity/cat/ocelot",
            "minecraft:ocelot_baby#main",
            "entity/cat/ocelot_baby",
        ),
    ),
    // PandaRenderer: ModelLayers.PANDA / PANDA_BABY,
    //   textures/entity/panda/panda.png / panda_baby.png (normal gene)
    (
        "minecraft:panda",
        row_baby(
            "minecraft:panda#main",
            "entity/panda/panda",
            "minecraft:panda_baby#main",
            "entity/panda/panda_baby",
        ),
    ),
    // ParchedRenderer: ModelLayers.PARCHED (dual-body skeleton layer),
    //   textures/entity/skeleton/parched.png
    (
        "minecraft:parched",
        row("minecraft:parched#main", "entity/skeleton/parched"),
    ),
    // ParrotRenderer: ModelLayers.PARROT, textures/entity/parrot/parrot_red_blue.png
    //   (default variant RED_BLUE; the other four variants later)
    (
        "minecraft:parrot",
        row("minecraft:parrot#main", "entity/parrot/parrot_red_blue"),
    ),
    // PhantomRenderer: ModelLayers.PHANTOM, textures/entity/phantom/phantom.png
    //   (phantom-size scaling is a later task)
    (
        "minecraft:phantom",
        row("minecraft:phantom#main", "entity/phantom/phantom"),
    ),
    // PigRenderer: ModelLayers.PIG / PIG_BABY (temperate variant),
    //   entity/pig/pig_temperate / pig_temperate_baby
    (
        "minecraft:pig",
        row_baby(
            "minecraft:pig#main",
            "entity/pig/pig_temperate",
            "minecraft:pig_baby#main",
            "entity/pig/pig_temperate_baby",
        ),
    ),
    // PiglinRenderer: ModelLayers.PIGLIN / PIGLIN_BABY,
    //   textures/entity/piglin/piglin.png / piglin_baby.png
    (
        "minecraft:piglin",
        row_baby(
            "minecraft:piglin#main",
            "entity/piglin/piglin",
            "minecraft:piglin_baby#main",
            "entity/piglin/piglin_baby",
        ),
    ),
    // PiglinRenderer (isBrute): ModelLayers.PIGLIN_BRUTE (no baby model in vanilla),
    //   textures/entity/piglin/piglin_brute.png
    (
        "minecraft:piglin_brute",
        row("minecraft:piglin_brute#main", "entity/piglin/piglin_brute"),
    ),
    // PillagerRenderer: ModelLayers.PILLAGER (illager layer, 0.9375),
    //   textures/entity/illager/pillager.png
    (
        "minecraft:pillager",
        row_scaled("minecraft:pillager#main", "entity/illager/pillager", 0.9375),
    ),
    // PolarBearRenderer: ModelLayers.POLAR_BEAR / POLAR_BEAR_BABY,
    //   textures/entity/bear/polarbear.png / polarbear_baby.png
    (
        "minecraft:polar_bear",
        row_baby(
            "minecraft:polar_bear#main",
            "entity/bear/polarbear",
            "minecraft:polar_bear_baby#main",
            "entity/bear/polarbear_baby",
        ),
    ),
    // PufferfishRenderer: ModelLayers.PUFFERFISH_BIG (default puff state 0 uses the small
    //   layer; the dumped PUFFERFISH_SMALL is picked here), textures/entity/fish/pufferfish.png
    (
        "minecraft:pufferfish",
        row("minecraft:pufferfish_small#main", "entity/fish/pufferfish"),
    ),
    // RabbitRenderer: ModelLayers.RABBIT / RABBIT_BABY,
    //   textures/entity/rabbit/rabbit_brown.png / rabbit_brown_baby.png (default variant BROWN)
    (
        "minecraft:rabbit",
        row_baby(
            "minecraft:rabbit#main",
            "entity/rabbit/rabbit_brown",
            "minecraft:rabbit_baby#main",
            "entity/rabbit/rabbit_brown_baby",
        ),
    ),
    // RavagerRenderer: ModelLayers.RAVAGER, textures/entity/illager/ravager.png
    (
        "minecraft:ravager",
        row("minecraft:ravager#main", "entity/illager/ravager"),
    ),
    // SalmonRenderer: ModelLayers.SALMON (medium is the default variant),
    //   textures/entity/fish/salmon.png
    (
        "minecraft:salmon",
        row("minecraft:salmon#main", "entity/fish/salmon"),
    ),
    // SheepRenderer: ModelLayers.SHEEP / SHEEP_BABY,
    //   textures/entity/sheep/sheep.png / sheep_baby.png (wool layer later)
    (
        "minecraft:sheep",
        row_baby(
            "minecraft:sheep#main",
            "entity/sheep/sheep",
            "minecraft:sheep_baby#main",
            "entity/sheep/sheep_baby",
        ),
    ),
    // ShulkerRenderer: ModelLayers.SHULKER, textures/entity/shulker/shulker.png
    //   (uncoloured default; dyed variants later)
    (
        "minecraft:shulker",
        row("minecraft:shulker#main", "entity/shulker/shulker"),
    ),
    // SilverfishRenderer: ModelLayers.SILVERFISH, textures/entity/silverfish/silverfish.png
    (
        "minecraft:silverfish",
        row("minecraft:silverfish#main", "entity/silverfish/silverfish"),
    ),
    // SkeletonHorseRenderer (UndeadHorseRenderer.Type.SKELETON):
    //   ModelLayers.SKELETON_HORSE / SKELETON_HORSE_BABY,
    //   textures/entity/horse/horse_skeleton.png / horse_skeleton_baby.png
    (
        "minecraft:skeleton_horse",
        row_baby(
            "minecraft:skeleton_horse#main",
            "entity/horse/horse_skeleton",
            "minecraft:skeleton_horse_baby#main",
            "entity/horse/horse_skeleton_baby",
        ),
    ),
    // SkeletonRenderer: ModelLayers.SKELETON, textures/entity/skeleton/skeleton.png
    (
        "minecraft:skeleton",
        row("minecraft:skeleton#main", "entity/skeleton/skeleton"),
    ),
    // SlimeRenderer: ModelLayers.SLIME (inner body), textures/entity/slime/slime.png
    //   (size scaling from metadata is a later task)
    (
        "minecraft:slime",
        row("minecraft:slime#main", "entity/slime/slime"),
    ),
    // SnifferRenderer: ModelLayers.SNIFFER / SNIFFER_BABY,
    //   textures/entity/sniffer/sniffer.png / snifflet.png (the baby texture is "snifflet")
    (
        "minecraft:sniffer",
        row_baby(
            "minecraft:sniffer#main",
            "entity/sniffer/sniffer",
            "minecraft:sniffer_baby#main",
            "entity/sniffer/snifflet",
        ),
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
    // SquidRenderer: ModelLayers.SQUID / SQUID_BABY,
    //   textures/entity/squid/squid.png / squid_baby.png
    (
        "minecraft:squid",
        row_baby(
            "minecraft:squid#main",
            "entity/squid/squid",
            "minecraft:squid_baby#main",
            "entity/squid/squid_baby",
        ),
    ),
    // StrayRenderer: ModelLayers.STRAY, textures/entity/skeleton/stray.png
    (
        "minecraft:stray",
        row("minecraft:stray#main", "entity/skeleton/stray"),
    ),
    // StriderRenderer: ModelLayers.STRIDER / STRIDER_BABY,
    //   textures/entity/strider/strider.png / strider_baby.png (cold/suffocating later)
    (
        "minecraft:strider",
        row_baby(
            "minecraft:strider#main",
            "entity/strider/strider",
            "minecraft:strider_baby#main",
            "entity/strider/strider_baby",
        ),
    ),
    // SulfurCubeRenderer: ModelLayers.SULFUR_CUBE,
    //   textures/entity/sulfur_cube/sulfur_cube_outer.png
    (
        "minecraft:sulfur_cube",
        row(
            "minecraft:sulfur_cube#main",
            "entity/sulfur_cube/sulfur_cube_outer",
        ),
    ),
    // TadpoleRenderer: ModelLayers.TADPOLE, textures/entity/tadpole/tadpole.png
    (
        "minecraft:tadpole",
        row("minecraft:tadpole#main", "entity/tadpole/tadpole"),
    ),
    // TraderLlamaRenderer: ModelLayers.TRADER_LLAMA / TRADER_LLAMA_BABY,
    //   textures/entity/llama/llama_creamy.png / llama_creamy_baby.png (creamy is the default)
    (
        "minecraft:trader_llama",
        row_baby(
            "minecraft:trader_llama#main",
            "entity/llama/llama_creamy",
            "minecraft:trader_llama_baby#main",
            "entity/llama/llama_creamy_baby",
        ),
    ),
    // TropicalFishRenderer: ModelLayers.TROPICAL_FISH_SMALL (KOB pattern is the small base),
    //   textures/entity/fish/tropical_a.png (pattern layer later)
    (
        "minecraft:tropical_fish",
        row(
            "minecraft:tropical_fish_small#main",
            "entity/fish/tropical_a",
        ),
    ),
    // TurtleRenderer: ModelLayers.TURTLE / TURTLE_BABY,
    //   textures/entity/turtle/turtle.png / turtle_baby.png
    (
        "minecraft:turtle",
        row_baby(
            "minecraft:turtle#main",
            "entity/turtle/turtle",
            "minecraft:turtle_baby#main",
            "entity/turtle/turtle_baby",
        ),
    ),
    // VexRenderer: ModelLayers.VEX, textures/entity/illager/vex.png (charging later)
    (
        "minecraft:vex",
        row("minecraft:vex#main", "entity/illager/vex"),
    ),
    // VillagerRenderer: ModelLayers.VILLAGER / VILLAGER_BABY (villager layer scaled 0.9375),
    //   textures/entity/villager/villager.png / villager_baby.png (profession overlay later)
    (
        "minecraft:villager",
        Entry {
            layer: "minecraft:villager#main",
            texture: "entity/villager/villager",
            root_scale: 0.9375,
            baby: Some((
                "minecraft:villager_baby#main",
                "entity/villager/villager_baby",
            )),
        },
    ),
    // VindicatorRenderer: ModelLayers.VINDICATOR (illager layer, 0.9375),
    //   textures/entity/illager/vindicator.png
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
    // WardenRenderer: ModelLayers.WARDEN, textures/entity/warden/warden.png
    //   (emissive layers later)
    (
        "minecraft:warden",
        row("minecraft:warden#main", "entity/warden/warden"),
    ),
    // WitchRenderer: ModelLayers.WITCH (witch layer scaled 0.9375),
    //   textures/entity/witch/witch.png
    (
        "minecraft:witch",
        row_scaled("minecraft:witch#main", "entity/witch/witch", 0.9375),
    ),
    // WitherBossRenderer: ModelLayers.WITHER, textures/entity/wither/wither.png
    //   (vanilla's scale() applies 2.0 on top of the pose stack; folded into the root scale)
    (
        "minecraft:wither",
        row_scaled("minecraft:wither#main", "entity/wither/wither", 2.0),
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
    // WolfRenderer: ModelLayers.WOLF / WOLF_BABY, texture from the WolfVariant registry
    //   (default PALE: entity/wolf/wolf.png / wolf_baby.png, wild texture; per-variant and
    //   tame/angry textures later)
    (
        "minecraft:wolf",
        row_baby(
            "minecraft:wolf#main",
            "entity/wolf/wolf",
            "minecraft:wolf_baby#main",
            "entity/wolf/wolf_baby",
        ),
    ),
    // ZoglinRenderer: ModelLayers.ZOGLIN / ZOGLIN_BABY,
    //   textures/entity/hoglin/zoglin.png / zoglin_baby.png
    (
        "minecraft:zoglin",
        row_baby(
            "minecraft:zoglin#main",
            "entity/hoglin/zoglin",
            "minecraft:zoglin_baby#main",
            "entity/hoglin/zoglin_baby",
        ),
    ),
    // ZombieHorseRenderer (UndeadHorseRenderer.Type.ZOMBIE):
    //   ModelLayers.ZOMBIE_HORSE / ZOMBIE_HORSE_BABY,
    //   textures/entity/horse/horse_zombie.png / horse_zombie_baby.png
    (
        "minecraft:zombie_horse",
        row_baby(
            "minecraft:zombie_horse#main",
            "entity/horse/horse_zombie",
            "minecraft:zombie_horse_baby#main",
            "entity/horse/horse_zombie_baby",
        ),
    ),
    // ZombieNautilusRenderer: ModelLayers.ZOMBIE_NAUTILUS,
    //   texture from the ZombieNautilusVariant registry (default normal variant)
    (
        "minecraft:zombie_nautilus",
        row(
            "minecraft:zombie_nautilus#main",
            "entity/nautilus/zombie_nautilus",
        ),
    ),
    // ZombieRenderer: ModelLayers.ZOMBIE / ZOMBIE_BABY,
    //   textures/entity/zombie/zombie.png / zombie_baby.png
    (
        "minecraft:zombie",
        row_baby(
            "minecraft:zombie#main",
            "entity/zombie/zombie",
            "minecraft:zombie_baby#main",
            "entity/zombie/zombie_baby",
        ),
    ),
    // ZombieVillagerRenderer: ModelLayers.ZOMBIE_VILLAGER / ZOMBIE_VILLAGER_BABY,
    //   textures/entity/zombie_villager/zombie_villager.png / zombie_villager_baby.png
    (
        "minecraft:zombie_villager",
        row_baby(
            "minecraft:zombie_villager#main",
            "entity/zombie_villager/zombie_villager",
            "minecraft:zombie_villager_baby#main",
            "entity/zombie_villager/zombie_villager_baby",
        ),
    ),
    // ZombifiedPiglinRenderer: ModelLayers.ZOMBIFIED_PIGLIN / ZOMBIFIED_PIGLIN_BABY,
    //   textures/entity/piglin/zombified_piglin.png / zombified_piglin_baby.png
    (
        "minecraft:zombified_piglin",
        row_baby(
            "minecraft:zombified_piglin#main",
            "entity/piglin/zombified_piglin",
            "minecraft:zombified_piglin_baby#main",
            "entity/piglin/zombified_piglin_baby",
        ),
    ),
];

/// The metadata index of `AgeableMob.DATA_BABY_ID` (and `Zombie`/`Zoglin`/`Piglin`'s own
/// `DATA_BABY_ID`, which lands on the same slot): `Entity` defines indices 0-7, `LivingEntity`
/// 8-14, `Mob` 15, and `AgeableMob` appends its boolean at 16 (Entity.java:323-330,
/// LivingEntity.java:323-331, Mob.java:270-273, AgeableMob.java:105-107 in the decompiled
/// source; each class's `defineSynchedData` calls `super` first, then appends).
pub const BABY_METADATA_INDEX: u8 = 16;

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
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(&kind) {
        return hit.clone();
    }
    let name = entity_type_names()?.get(&kind).copied()?;
    let entry = model_for_name(name).map(|m| (has_head(&m.root), m));
    cache.lock().unwrap().insert(kind, entry.clone());
    entry
}

/// The `minecraft:entity_type` name for `kind` (shared map, read once from the reports; the
/// strings are leaked with the map, so this is a copy-free `&'static str`).
pub fn entity_type_name(kind: RegistryIndex) -> Option<&'static str> {
    entity_type_names()?.get(&kind).copied()
}

/// The table-covered entity type names by protocol id (loaded lazily, once per process).
pub fn entity_type_names() -> Option<&'static HashMap<RegistryIndex, &'static str>> {
    static NAMES: OnceLock<Option<HashMap<RegistryIndex, &'static str>>> = OnceLock::new();
    NAMES.get_or_init(load_entity_type_names).as_ref()
}

/// Whether the tree contains a part named "head" (computed once per type, not per frame).
fn has_head(part: &haste_core::model::ModelPart) -> bool {
    part.id == haste_core::parts::HEAD || part.children.iter().any(has_head)
}

/// The table entry for `name`: (layer id, root scale) for tests and callers
/// that want the mapping without the dump.
pub fn table_entry(name: &str) -> Option<(&'static str, f32)> {
    let (_, e) = TABLE.iter().find(|(n, _)| *n == name)?;
    Some((e.layer, e.root_scale))
}

/// Every texture path in the table (unique, table order), for the jar-presence check the
/// renderer runs (a typo here would only show as placeholder boxes at runtime).
pub fn table_textures() -> impl Iterator<Item = &'static str> {
    fn entry_textures(e: &Entry) -> [&'static str; 2] {
        match e.baby {
            Some((_, baby_texture)) => [e.texture, baby_texture],
            None => [e.texture, ""],
        }
    }
    TABLE
        .iter()
        .flat_map(|(_, e)| entry_textures(e))
        .filter(|t| !t.is_empty())
}

/// `minecraft:entity_type` names by protocol id, from the data-generator reports
/// (`haste_core::paths::datagen_reports`, owned by world-and-data). haste-entity reads the one
/// registry it needs itself: haste-data's `Registries::parse` is not exported and pulling all
/// of `GameData` in here would duplicate the client's load. Every entity type is kept (the
/// decoration kinds need names the model table does not cover); `model_entry` filters.
fn load_entity_type_names() -> Option<HashMap<RegistryIndex, &'static str>> {
    let path = haste_core::paths::datagen_reports().join("registries.json");
    let bytes = std::fs::read(path).ok()?;
    let registries: HashMap<&str, EntityTypeRegistryJson> = serde_json::from_slice(&bytes).ok()?;
    let reg = registries.get("minecraft:entity_type")?;
    let mut names = HashMap::with_capacity(reg.entries.len());
    for (name, e) in &reg.entries {
        names.insert(
            RegistryIndex(e.protocol_id),
            Box::leak((*name).to_owned().into_boxed_str()) as &'static str,
        );
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

/// The model for entity type `name` when the entity is a baby, from the row's baby layer
/// (vanilla's `AgeableMobRenderer.submit` picks `babyModel` when `state.isBaby`), falling back
/// to the adult model when the row has none (vanilla types without a baby layer) or the dump
/// lacks the layer. Not cached per type: the baby flag is per entity, but the underlying layer
/// models are shared `Arc`s inside the library.
pub fn baby_model_for_name(name: &str) -> Option<Arc<CubeModel>> {
    let (_, entry) = TABLE.iter().find(|(n, _)| *n == name)?;
    let (baby_layer, baby_texture) = entry.baby?;
    let lib = model_dump::library()?;
    let model = lib.get(baby_layer, entry.root_scale)?;
    let texture = Identifier::minecraft(baby_texture);
    if model.texture == texture {
        return Some(model);
    }
    let mut m = (*model).clone();
    m.texture = texture;
    Some(Arc::new(m))
}

/// Whether the entity's metadata says it is a baby: `AgeableMob.DATA_BABY_ID` (index 16) or
/// the same-slot flags `Zombie`/`Zoglin`/`Piglin` define. Types without any baby flag are
/// never babies.
pub fn is_baby(metadata: &crate::metadata::Metadata) -> bool {
    matches!(
        metadata.get(BABY_METADATA_INDEX).map(|e| &e.value),
        Some(MetadataValue::Boolean(true))
    )
}

/// The `EntityAttachment.NAME_TAG` anchor above the entity's feet: the `AT_HEIGHT` fallback is
/// `Entity.getBbHeight()`; the sniffer's type overrides it to 2.05
/// (`EntityTypes.SNIFFER.nameTagOffset`). `submitNameTag` adds 0.5 on top of this.
pub fn name_tag_offset_for_name(name: &str, bb_height: f32) -> f32 {
    match name {
        "minecraft:sniffer" => 2.05,
        _ => bb_height,
    }
}
