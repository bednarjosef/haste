//! The shadow each entity renderer casts: `EntityRenderer.shadowRadius` and
//! `shadowStrength` by entity type. Moved out of `registry.rs` to keep that file within the
//! size limit; a pure move.

/// The vanilla `EntityRenderer.shadowRadius` for an entity type: the value its renderer
/// constructor passes to `LivingEntityRenderer`/`MobRenderer`, or the base it sets directly
/// (`EntityRenderer.shadowRadius` defaults to 0.0, so types absent here cast no shadow). Read
/// from 26.3 `net.minecraft.client.renderer.entity` (the class named per arm); a table-covered
/// living type not named falls back to the usual mob 0.5. `store::render_instances` multiplies
/// it by the baby/living scale, like `LivingEntityRenderer.getShadowRadius`.
pub fn shadow_radius_for_name(name: &str) -> f32 {
    let r = match name {
        // AllayRenderer 0.4, ArmadilloRenderer 0.7, AxolotlRenderer 0.5
        "minecraft:allay" => 0.4,
        "minecraft:armadillo" => 0.7,
        "minecraft:axolotl" => 0.5,
        // BatRenderer 0.25, BeeRenderer 0.4, BlazeRenderer 0.5, BreezeRenderer 0.5
        "minecraft:bat" => 0.25,
        "minecraft:bee" => 0.4,
        "minecraft:blaze" => 0.5,
        "minecraft:breeze" => 0.5,
        // CamelRenderer / CamelHuskRenderer 0.7, CatRenderer 0.4, CaveSpiderRenderer 0.56
        "minecraft:camel" | "minecraft:camel_husk" => 0.7,
        "minecraft:cat" => 0.4,
        "minecraft:cave_spider" => 0.56,
        // ChickenRenderer 0.3, CodRenderer 0.3, CopperGolemRenderer 0.5, CowRenderer 0.7
        "minecraft:chicken" => 0.3,
        "minecraft:cod" => 0.3,
        "minecraft:copper_golem" => 0.5,
        "minecraft:cow" | "minecraft:mooshroom" => 0.7,
        // CreakingRenderer 0.6, CreeperRenderer 0.5
        "minecraft:creaking" => 0.6,
        "minecraft:creeper" => 0.5,
        // DolphinRenderer 0.7, Donkey/Mule 0.75 (AbstractHorseRenderer)
        "minecraft:dolphin" => 0.7,
        "minecraft:donkey" | "minecraft:mule" => 0.75,
        // DrownedRenderer / HuskRenderer / ZombieRenderer 0.5 (AbstractZombieRenderer)
        "minecraft:drowned"
        | "minecraft:husk"
        | "minecraft:zombie"
        | "minecraft:zombie_villager" => 0.5,
        // EnderDragonRenderer 0.5, EndermanRenderer 0.5, EndermiteRenderer 0.3
        "minecraft:ender_dragon" => 0.5,
        "minecraft:enderman" => 0.5,
        "minecraft:endermite" => 0.3,
        // EvokerRenderer / IllusionerRenderer / PillagerRenderer / VindicatorRenderer 0.5
        "minecraft:evoker"
        | "minecraft:illusioner"
        | "minecraft:pillager"
        | "minecraft:vindicator" => 0.5,
        // FoxRenderer 0.4, FrogRenderer 0.3, GhastRenderer 1.5
        "minecraft:fox" => 0.4,
        "minecraft:frog" => 0.3,
        "minecraft:ghast" => 1.5,
        // GoatRenderer 0.7, GuardianRenderer/ElderGuardian 0.5, Hoglin/Zoglin 0.7
        "minecraft:goat" => 0.7,
        "minecraft:guardian" | "minecraft:elder_guardian" => 0.5,
        "minecraft:hoglin" | "minecraft:zoglin" => 0.7,
        // Horse family 0.75 (AbstractHorseRenderer)
        "minecraft:horse" | "minecraft:skeleton_horse" | "minecraft:zombie_horse" => 0.75,
        // IronGolemRenderer 0.7, Llama/TraderLlama 0.7, Nautilus/ZombieNautilus 0.7
        "minecraft:iron_golem" => 0.7,
        "minecraft:llama" | "minecraft:trader_llama" => 0.7,
        "minecraft:nautilus" | "minecraft:zombie_nautilus" => 0.7,
        // OcelotRenderer 0.4, PandaRenderer 0.9, ParrotRenderer 0.3, PhantomRenderer 0.75
        "minecraft:ocelot" => 0.4,
        "minecraft:panda" => 0.9,
        "minecraft:parrot" => 0.3,
        "minecraft:phantom" => 0.75,
        // PigRenderer 0.7, PolarBearRenderer 0.9, PufferfishRenderer 0.2
        "minecraft:pig" => 0.7,
        "minecraft:polar_bear" => 0.9,
        "minecraft:pufferfish" => 0.2,
        // RabbitRenderer 0.3, RavagerRenderer 1.1, SalmonRenderer 0.4
        "minecraft:rabbit" => 0.3,
        "minecraft:ravager" => 1.1,
        "minecraft:salmon" => 0.4,
        // SheepRenderer 0.7, ShulkerRenderer 0.0, SilverfishRenderer 0.3
        "minecraft:sheep" => 0.7,
        "minecraft:shulker" => 0.0,
        "minecraft:silverfish" => 0.3,
        // Skeleton family 0.5 (AbstractSkeletonRenderer), Slime/MagmaCube 0.25, Sniffer 1.1
        "minecraft:skeleton"
        | "minecraft:stray"
        | "minecraft:bogged"
        | "minecraft:wither_skeleton" => 0.5,
        "minecraft:slime" | "minecraft:magma_cube" => 0.25,
        "minecraft:sniffer" => 1.1,
        // SnowGolemRenderer 0.5, SpiderRenderer 0.8, Squid/GlowSquid 0.7, Strider 0.5
        "minecraft:snow_golem" => 0.5,
        "minecraft:spider" => 0.8,
        "minecraft:squid" | "minecraft:glow_squid" => 0.7,
        "minecraft:strider" => 0.5,
        // TadpoleRenderer 0.14, TropicalFishRenderer 0.15, TurtleRenderer 0.7
        "minecraft:tadpole" => 0.14,
        "minecraft:tropical_fish" => 0.15,
        "minecraft:turtle" => 0.7,
        // VexRenderer 0.3, Villager/WanderingTrader 0.5, WardenRenderer 0.9
        "minecraft:vex" => 0.3,
        "minecraft:villager" | "minecraft:wandering_trader" => 0.5,
        "minecraft:warden" => 0.9,
        // WitherBossRenderer 1.0, WolfRenderer 0.5, WitchRenderer 0.5
        "minecraft:wither" => 1.0,
        "minecraft:wolf" => 0.5,
        "minecraft:witch" => 0.5,
        // ArmorStandRenderer / ShulkerRenderer pass 0.0; the rest of the table (living types
        // not named above) use the usual 0.5.
        "minecraft:armor_stand" => 0.0,
        // ThrownTridentRenderer never sets the radius (the base 0.0): a table row, no shadow.
        "minecraft:trident" => 0.0,
        // Non-living renderers that set the base radius directly: ItemEntityRenderer 0.15,
        // ExperienceOrbRenderer 0.15, AbstractBoatRenderer 0.8, AbstractMinecartRenderer 0.7,
        // TntRenderer 0.5, FallingBlockRenderer 0.5, EndCrystalRenderer 0.5.
        "minecraft:item" | "minecraft:experience_orb" => 0.15,
        // Every type the boat table names (all 22 vanilla boats, chest boats and rafts).
        _ if crate::boat::entry_for_name(name).is_some() => 0.8,
        "minecraft:minecart"
        | "minecraft:chest_minecart"
        | "minecraft:furnace_minecart"
        | "minecraft:hopper_minecart"
        | "minecraft:tnt_minecart"
        | "minecraft:command_block_minecart" => 0.7,
        "minecraft:tnt" | "minecraft:falling_block" | "minecraft:end_crystal" => 0.5,
        _ if super::TABLE.iter().any(|(n, _)| *n == name) => 0.5,
        _ => 0.0,
    };
    r
}

/// The vanilla `EntityRenderer.shadowStrength` for an entity type: 0.75 for items and
/// experience orbs (`ItemEntityRenderer`/`ExperienceOrbRenderer`), 1.0 otherwise.
pub fn shadow_strength_for_name(name: &str) -> f32 {
    match name {
        "minecraft:item" | "minecraft:experience_orb" => 0.75,
        _ => 1.0,
    }
}
