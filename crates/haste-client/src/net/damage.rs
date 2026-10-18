//! Damage feedback: what the client does when the server says a living entity took
//! damage (`damage_event`, vanilla `ClientPacketListener.handleDamageEvent` ->
//! `LivingEntity.handleDamageEvent`) and when the local player's health changed
//! (`set_health` -> `LocalPlayer.hurtTo`).
//!
//! Vanilla's own split is kept: the hurt sound is chosen by the *victim's* type
//! (`LivingEntity.getHurtSound`, overridden per entity class; players take the damage
//! type's `effects` sound, `Player.getHurtSound` -> `DamageEffects.sound`), played at
//! the victim with `getSoundVolume()` and the fixed hurt pitch
//! `(rand - rand) * 0.2 + 1` (`handleDamageEvent` does not use `getVoicePitch`, so
//! babies pitch the same as adults). `hurtTo` sets the local player's `hurtTime` (the
//! camera tilt, `GameRenderer.bobHurt`) and `damageCooldownTime` (the HUD heart blink's
//! input, `Hud.extractPlayerHealth`), also set by `handleDamageEvent` for any damage.
//!
//! The per-type sound and volume tables follow vanilla's class overrides; they list
//! the *default* variant of each type (adult, out of water, not scared): the
//! state-dependent variants vanilla picks from synced data (a shulker's closed hurt,
//! a screaming goat, a drowned in water, a wolf's armor taking the hit) are a known
//! deviation, listed in the report. A type the tables do not know (a mod's) plays
//! vanilla's living-entity default, `entity.generic.hurt`, logged once per type.

use haste_audio::random::LegacyRandom;
use haste_protocol::SyncedRegistries;

use crate::sound::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// `SoundSource.PLAYERS` (the index in vanilla's `SoundSource` order).
const CATEGORY_PLAYERS: i32 = 7;
/// `SoundSource.HOSTILE` (`Monster.getSoundSource`).
const CATEGORY_HOSTILE: i32 = 5;
/// `SoundSource.NEUTRAL` (`Entity.getSoundSource`'s default).
const CATEGORY_NEUTRAL: i32 = 6;
/// Vanilla `LivingEntity.getHurtSound`: the default hurt sound of a living entity.
const GENERIC_HURT: &str = "minecraft:entity.generic.hurt";

/// The hurt sound event of a damage type's `effects` field (`DamageEffects.sound`,
/// what a hurt *player* plays): the synced `minecraft:damage_type` entry's id, or
/// `"hurt"` when the entry or the field is absent (vanilla's codec default).
pub fn damage_effects_id(source_type: i32, regs: &SyncedRegistries) -> String {
    let registry = haste_core::Identifier::minecraft("damage_type");
    let index = usize::try_from(source_type).ok();
    index
        .and_then(|i| regs.entry_nbt(&registry, i))
        .and_then(|nbt| nbt.get_string("effects").map(str::to_owned))
        .unwrap_or_else(|| "hurt".to_owned())
}

/// The hurt sound of a damage type's effects for a player (`DamageEffects` -> the
/// registered `SoundEvents.PLAYER_HURT*` event).
pub fn player_hurt_sound(effects: &str) -> &'static str {
    match effects {
        "drowning" => "minecraft:entity.player.hurt_drown",
        "burning" => "minecraft:entity.player.hurt_on_fire",
        "poking" => "minecraft:entity.player.hurt_sweet_berry_bush",
        "freezing" => "minecraft:entity.player.hurt_freeze",
        // "hurt", "thorns" and anything a mod synced that we do not know.
        _ => "minecraft:entity.player.hurt",
    }
}

/// The hurt sound event of an entity type: vanilla's `getHurtSound` overrides, keyed
/// by `minecraft:entity_type` name. `None` = not in the table (plays the generic
/// hurt, vanilla's `LivingEntity` base, logged once). Players are not in it: their
/// sound comes from the damage type's effects.
pub(crate) fn mob_hurt_sound(name: &str) -> Option<&'static str> {
    let path = name.strip_prefix("minecraft:").unwrap_or(name);
    let sound = match path {
        "allay" => "entity.allay.hurt",
        "armor_stand" => "entity.armor_stand.hit",
        "armadillo" => "entity.armadillo.hurt",
        "axolotl" => "entity.axolotl.hurt",
        "bat" => "entity.bat.hurt",
        "bee" => "entity.bee.hurt",
        "blaze" => "entity.blaze.hurt",
        "bogged" => "entity.bogged.hurt",
        "breeze" => "entity.breeze.hurt",
        "camel" => "entity.camel.hurt",
        "camel_husk" => "entity.camel_husk.hurt",
        // The variant sound sets (cow, pig, chicken, cat, wolf) all start from the
        // classic set; a variant pick needs the synced variant field (not mirrored).
        "cat" => "entity.cat.hurt",
        "cave_spider" => "entity.spider.hurt", // Spider's override, inherited
        "chicken" => "entity.chicken.hurt",
        "cod" => "entity.cod.hurt",
        "copper_golem" => "entity.copper_golem.hurt", // unweathered level
        "cow" => "entity.cow.hurt",
        "creeper" => "entity.creeper.hurt",
        "creaking" => "entity.hostile.hurt", // Monster's, unless heart-bound
        "dolphin" => "entity.dolphin.hurt",
        "donkey" => "entity.donkey.hurt",
        "drowned" => "entity.drowned.hurt", // the land variant
        "elder_guardian" => "entity.elder_guardian.hurt",
        "ender_dragon" => "entity.ender_dragon.hurt",
        "enderman" => "entity.enderman.hurt",
        "endermite" => "entity.endermite.hurt",
        "evoker" => "entity.evoker.hurt",
        "fox" => "entity.fox.hurt",
        "frog" => "entity.frog.hurt",
        "ghast" => "entity.ghast.hurt",
        "giant" => "entity.hostile.hurt", // Monster's base
        "glow_squid" => "entity.glow_squid.hurt",
        "goat" => "entity.goat.hurt", // the non-screaming variant
        "guardian" => "entity.guardian.hurt_land", // out of water
        "happy_ghast" => "entity.happy_ghast.hurt",
        "hoglin" => "entity.hoglin.hurt",
        "horse" => "entity.horse.hurt",
        "husk" => "entity.husk.hurt",
        "illusioner" => "entity.illusioner.hurt",
        "iron_golem" => "entity.iron_golem.hurt",
        "llama" => "entity.llama.hurt",
        "magma_cube" => "entity.magma_cube.hurt", // the non-tiny variant
        "mule" => "entity.mule.hurt",
        "nautilus" => "entity.nautilus.hurt",
        "ocelot" => "entity.ocelot.hurt",
        "panda" => "entity.panda.hurt",
        "parrot" => "entity.parrot.hurt",
        "parched" => "entity.parched.hurt",
        "phantom" => "entity.phantom.hurt",
        "pig" => "entity.pig.hurt",
        "piglin" => "entity.piglin.hurt",
        "piglin_brute" => "entity.piglin_brute.hurt",
        "pillager" => "entity.pillager.hurt",
        "polar_bear" => "entity.polar_bear.hurt",
        "pufferfish" => "entity.puffer_fish.hurt",
        "rabbit" => "entity.rabbit.hurt",
        "ravager" => "entity.ravager.hurt",
        "salmon" => "entity.salmon.hurt",
        "sheep" => "entity.sheep.hurt",
        "shulker" => "entity.shulker.hurt", // the open variant
        "silverfish" => "entity.silverfish.hurt",
        "skeleton" => "entity.skeleton.hurt",
        "skeleton_horse" => "entity.skeleton_horse.hurt",
        "slime" => "entity.slime.hurt", // the non-tiny variant
        "sniffer" => "entity.sniffer.hurt",
        "snow_golem" => "entity.snow_golem.hurt",
        "spider" => "entity.spider.hurt",
        "squid" => "entity.squid.hurt",
        "stray" => "entity.stray.hurt",
        "strider" => "entity.strider.hurt",
        "sulfur_cube" => "entity.sulfur_cube.hurt", // the non-tiny variant
        "tadpole" => "entity.tadpole.hurt",
        "trader_llama" => "entity.llama.hurt", // Llama's override, inherited
        "tropical_fish" => "entity.tropical_fish.hurt",
        "turtle" => "entity.turtle.hurt",
        "vex" => "entity.vex.hurt",
        "villager" => "entity.villager.hurt",
        "vindicator" => "entity.vindicator.hurt",
        "wandering_trader" => "entity.wandering_trader.hurt",
        "warden" => "entity.warden.hurt",
        "witch" => "entity.witch.hurt",
        "wither" => "entity.wither.hurt",
        "wither_skeleton" => "entity.wither_skeleton.hurt",
        "wolf" => "entity.wolf.hurt",
        "zoglin" => "entity.zoglin.hurt",
        "zombie" => "entity.zombie.hurt",
        "zombie_horse" => "entity.zombie_horse.hurt",
        "zombie_nautilus" => "entity.zombie_nautilus.hurt_land", // out of water
        "zombie_villager" => "entity.zombie_villager.hurt",
        "zombified_piglin" => "entity.zombified_piglin.hurt",
        _ => return None,
    };
    Some(sound)
}

/// The hurt sound an entity of `entity_name` (a `minecraft:entity_type` name, `None`
/// when the type is unknown) plays for damage with the `effects` id: players the
/// damage type's effects sound, other types their `getHurtSound` override, unknown
/// types vanilla's living-entity default.
pub fn hurt_sound(entity_name: Option<&str>, effects: &str) -> &'static str {
    match entity_name {
        Some(name) if name == "minecraft:player" => player_hurt_sound(effects),
        Some(name) => mob_hurt_sound(name).unwrap_or(GENERIC_HURT),
        None => GENERIC_HURT,
    }
}

/// The volume a hurt sound plays at (vanilla `getSoundVolume`, default 1). `data16`
/// is the entity's synced data byte at index 16: the baby flag of most living
/// entities, the size of the cube mobs (slime family) whose volume scales with it.
pub fn hurt_volume(entity_name: Option<&str>, data16: Option<i8>) -> f32 {
    let path = entity_name
        .and_then(|n| n.strip_prefix("minecraft:"))
        .unwrap_or_default();
    match path {
        "bat" => 0.1,
        "bee" | "squid" | "glow_squid" | "wolf" | "cow" | "allay" => 0.4,
        // The AbstractHorse family (horses, donkeys, mules, llamas, camels).
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "llama"
        | "trader_llama" | "camel" | "camel_husk" => 0.8,
        // A baby ghastling's volume is 1, an adult happy ghast's 4.
        "happy_ghast" => {
            if data16.is_some_and(|b| b != 0) {
                1.0
            } else {
                4.0
            }
        }
        "ghast" | "ender_dragon" => 5.0,
        "warden" => 4.0,
        // The cube mobs' volume scales with their size (synced data index 16).
        "slime" | "magma_cube" | "sulfur_cube" => 0.4 * data16.unwrap_or(1).max(1) as f32,
        _ => 1.0,
    }
}

/// The hurt sound's category (vanilla `Entity.getSoundSource`: `Monster` overrides it
/// to HOSTILE, everything else is NEUTRAL; players are PLAYERS).
pub fn hurt_category(entity_name: Option<&str>) -> i32 {
    let path = entity_name
        .and_then(|n| n.strip_prefix("minecraft:"))
        .unwrap_or_default();
    if path == "player" {
        CATEGORY_PLAYERS
    } else if matches!(
        path,
        "blaze" | "bogged" | "breeze" | "creaking" | "creeper" | "drowned" | "elder_guardian"
            | "enderman" | "endermite" | "evoker" | "ghast" | "giant" | "guardian" | "hoglin"
            | "husk" | "illusioner" | "magma_cube" | "parched" | "phantom" | "piglin"
            | "piglin_brute" | "pillager" | "ravager" | "shulker" | "silverfish" | "skeleton"
            | "slime" | "spider" | "cave_spider" | "stray" | "vex" | "vindicator" | "warden"
            | "witch" | "wither" | "wither_skeleton" | "zoglin" | "zombie" | "zombie_nautilus"
            | "zombie_villager" | "zombified_piglin"
    ) {
        CATEGORY_HOSTILE
    } else {
        CATEGORY_NEUTRAL
    }
}

/// Builds the hurt sound of one damaged entity (vanilla
/// `LivingEntity.handleDamageEvent` -> `playSound(hurtSound, volume, pitch)`): the
/// event by victim type and damage effects, the volume and category by type, the
/// pitch `(rand - rand) * 0.2 + 1`, played at the victim. `own_pos` is where the local
/// player's own sound plays (it is not in the entity store); other entities play from
/// their store position at drain time.
pub fn hurt_request(
    rng: &mut LegacyRandom,
    entity_name: Option<&str>,
    effects: &str,
    data16: Option<i8>,
    entity_id: i32,
    own: bool,
    own_pos: glam::DVec3,
) -> SoundRequest {
    let pitch = 1.0 + (rng.next_float() - rng.next_float()) * 0.2;
    SoundRequest::Play(PlayRequest {
        sound: SoundRef::Inline(hurt_sound(entity_name, effects).to_owned()),
        category: hurt_category(entity_name),
        pos: if own {
            SoundPos::Fixed(own_pos)
        } else {
            SoundPos::Entity(entity_id)
        },
        volume: hurt_volume(entity_name, data16),
        pitch,
        seed: rng.next_long(),
    })
}

#[cfg(test)]
mod tests;
