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

use super::{EntityId, EntitySink, Session, DAMAGE_COOLDOWN_TICKS, HURT_TILT_TICKS};
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
        "allay" => "minecraft:entity.allay.hurt",
        "armor_stand" => "minecraft:entity.armor_stand.hit",
        "armadillo" => "minecraft:entity.armadillo.hurt",
        "axolotl" => "minecraft:entity.axolotl.hurt",
        "bat" => "minecraft:entity.bat.hurt",
        "bee" => "minecraft:entity.bee.hurt",
        "blaze" => "minecraft:entity.blaze.hurt",
        "bogged" => "minecraft:entity.bogged.hurt",
        "breeze" => "minecraft:entity.breeze.hurt",
        "camel" => "minecraft:entity.camel.hurt",
        "camel_husk" => "minecraft:entity.camel_husk.hurt",
        // The variant sound sets (cow, pig, chicken, cat, wolf) all start from the
        // classic set; a variant pick needs the synced variant field (not mirrored).
        "cat" => "minecraft:entity.cat.hurt",
        "cave_spider" => "minecraft:entity.spider.hurt", // Spider's override, inherited
        "chicken" => "minecraft:entity.chicken.hurt",
        "cod" => "minecraft:entity.cod.hurt",
        "copper_golem" => "minecraft:entity.copper_golem.hurt", // unweathered level
        "cow" => "minecraft:entity.cow.hurt",
        "creeper" => "minecraft:entity.creeper.hurt",
        "creaking" => "minecraft:entity.hostile.hurt", // Monster's, unless heart-bound
        "dolphin" => "minecraft:entity.dolphin.hurt",
        "donkey" => "minecraft:entity.donkey.hurt",
        "drowned" => "minecraft:entity.drowned.hurt", // the land variant
        "elder_guardian" => "minecraft:entity.elder_guardian.hurt",
        "ender_dragon" => "minecraft:entity.ender_dragon.hurt",
        "enderman" => "minecraft:entity.enderman.hurt",
        "endermite" => "minecraft:entity.endermite.hurt",
        "evoker" => "minecraft:entity.evoker.hurt",
        "fox" => "minecraft:entity.fox.hurt",
        "frog" => "minecraft:entity.frog.hurt",
        "ghast" => "minecraft:entity.ghast.hurt",
        "giant" => "minecraft:entity.hostile.hurt", // Monster's base
        "glow_squid" => "minecraft:entity.glow_squid.hurt",
        "goat" => "minecraft:entity.goat.hurt", // the non-screaming variant
        "guardian" => "minecraft:entity.guardian.hurt_land", // out of water
        "happy_ghast" => "minecraft:entity.happy_ghast.hurt",
        "hoglin" => "minecraft:entity.hoglin.hurt",
        "horse" => "minecraft:entity.horse.hurt",
        "husk" => "minecraft:entity.husk.hurt",
        "illusioner" => "minecraft:entity.illusioner.hurt",
        "iron_golem" => "minecraft:entity.iron_golem.hurt",
        "llama" => "minecraft:entity.llama.hurt",
        "magma_cube" => "minecraft:entity.magma_cube.hurt", // the non-tiny variant
        "mule" => "minecraft:entity.mule.hurt",
        "nautilus" => "minecraft:entity.nautilus.hurt",
        "ocelot" => "minecraft:entity.ocelot.hurt",
        "panda" => "minecraft:entity.panda.hurt",
        "parrot" => "minecraft:entity.parrot.hurt",
        "parched" => "minecraft:entity.parched.hurt",
        "phantom" => "minecraft:entity.phantom.hurt",
        "pig" => "minecraft:entity.pig.hurt",
        "piglin" => "minecraft:entity.piglin.hurt",
        "piglin_brute" => "minecraft:entity.piglin_brute.hurt",
        "pillager" => "minecraft:entity.pillager.hurt",
        "polar_bear" => "minecraft:entity.polar_bear.hurt",
        "pufferfish" => "minecraft:entity.puffer_fish.hurt",
        "rabbit" => "minecraft:entity.rabbit.hurt",
        "ravager" => "minecraft:entity.ravager.hurt",
        "salmon" => "minecraft:entity.salmon.hurt",
        "sheep" => "minecraft:entity.sheep.hurt",
        "shulker" => "minecraft:entity.shulker.hurt", // the open variant
        "silverfish" => "minecraft:entity.silverfish.hurt",
        "skeleton" => "minecraft:entity.skeleton.hurt",
        "skeleton_horse" => "minecraft:entity.skeleton_horse.hurt",
        "slime" => "minecraft:entity.slime.hurt", // the non-tiny variant
        "sniffer" => "minecraft:entity.sniffer.hurt",
        "snow_golem" => "minecraft:entity.snow_golem.hurt",
        "spider" => "minecraft:entity.spider.hurt",
        "squid" => "minecraft:entity.squid.hurt",
        "stray" => "minecraft:entity.stray.hurt",
        "strider" => "minecraft:entity.strider.hurt",
        "sulfur_cube" => "minecraft:entity.sulfur_cube.hurt", // the non-tiny variant
        "tadpole" => "minecraft:entity.tadpole.hurt",
        "trader_llama" => "minecraft:entity.llama.hurt", // Llama's override, inherited
        "tropical_fish" => "minecraft:entity.tropical_fish.hurt",
        "turtle" => "minecraft:entity.turtle.hurt",
        "vex" => "minecraft:entity.vex.hurt",
        "villager" => "minecraft:entity.villager.hurt",
        "vindicator" => "minecraft:entity.vindicator.hurt",
        "wandering_trader" => "minecraft:entity.wandering_trader.hurt",
        "warden" => "minecraft:entity.warden.hurt",
        "witch" => "minecraft:entity.witch.hurt",
        "wither" => "minecraft:entity.wither.hurt",
        "wither_skeleton" => "minecraft:entity.wither_skeleton.hurt",
        "wolf" => "minecraft:entity.wolf.hurt",
        "zoglin" => "minecraft:entity.zoglin.hurt",
        "zombie" => "minecraft:entity.zombie.hurt",
        "zombie_horse" => "minecraft:entity.zombie_horse.hurt",
        "zombie_nautilus" => "minecraft:entity.zombie_nautilus.hurt_land", // out of water
        "zombie_villager" => "minecraft:entity.zombie_villager.hurt",
        "zombified_piglin" => "minecraft:entity.zombified_piglin.hurt",
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
        Some("minecraft:player") => player_hurt_sound(effects),
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
        "blaze"
            | "bogged"
            | "breeze"
            | "creaking"
            | "creeper"
            | "drowned"
            | "elder_guardian"
            | "enderman"
            | "endermite"
            | "evoker"
            | "ghast"
            | "giant"
            | "guardian"
            | "hoglin"
            | "husk"
            | "illusioner"
            | "magma_cube"
            | "parched"
            | "phantom"
            | "piglin"
            | "piglin_brute"
            | "pillager"
            | "ravager"
            | "shulker"
            | "silverfish"
            | "skeleton"
            | "slime"
            | "spider"
            | "cave_spider"
            | "stray"
            | "vex"
            | "vindicator"
            | "warden"
            | "witch"
            | "wither"
            | "wither_skeleton"
            | "zoglin"
            | "zombie"
            | "zombie_nautilus"
            | "zombie_villager"
            | "zombified_piglin"
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

/// One `damage_event` (vanilla `ClientPacketListener.handleDamageEvent` ->
/// `LivingEntity.handleDamageEvent`): the hurt animation runs (`hurtTime` 10 — the
/// red flash for other entities, the camera tilt for the local player), the damage
/// cooldown starts (the HUD heart blink's gate) and the victim's hurt sound plays
/// with its type's volume and category and the hurt pitch `1 ± 0.2`.
pub(crate) fn apply_damage_event<E: EntitySink + ?Sized>(
    session: &mut Session,
    d: &haste_protocol::packets::play::clientbound::DamageEvent,
    entities: &mut E,
    regs: &SyncedRegistries,
) {
    let id = EntityId(d.entity_id);
    entities.mark_hurt(id);
    let own_hit = id == session.entity_id;
    if own_hit {
        session.hurt_ticks = HURT_TILT_TICKS;
        session.damage_cooldown = DAMAGE_COOLDOWN_TICKS;
    }
    // The victim's type: ours from the login (our own entity is not in the store),
    // others' from their spawn's registry index. An index past the name table (a mod
    // the server synced names for) falls through to the generic hurt, no panic.
    let name: Option<String> = if own_hit {
        Some("minecraft:player".to_owned())
    } else {
        entities
            .entity_type(id)
            .and_then(|k| session.entity_type_names.get(k.0 as usize))
            .map(|n| n.to_string())
    };
    if let Some(name) = &name {
        if mob_hurt_sound(name).is_none() && session.unknown_hurt_types.insert(name.clone()) {
            tracing::info!("unknown entity type {name}; its hurt sound is the generic one");
        }
    }
    // The synced data byte at index 16: the baby flag of most living entities, or the
    // cube mobs' size (their volume scales with it).
    let data16 = entities.metadata_value(id, 16).and_then(|v| match v {
        haste_core::entity::MetadataValue::Byte(b) => Some(*b),
        _ => None,
    });
    let effects = damage_effects_id(d.source_type, regs);
    session.sound_requests.push(hurt_request(
        &mut session.hurt_rng,
        name.as_deref(),
        &effects,
        data16,
        d.entity_id,
        own_hit,
        session.player.state.pos,
    ));
}

/// One `set_health` for the local player (vanilla
/// `ClientPacketListener.handleSetHealth` -> `LocalPlayer.hurtTo`): a health drop
/// starts the hurt tilt (`hurtTime` 10) and the damage cooldown (20), a gain starts a
/// short cooldown (10). The first `set_health` of a player (join, respawn) only stores
/// the health (`flashOnSetHealth`): the login packet must not flash or tilt.
pub(crate) fn hurt_to(session: &mut Session, old: f32, new: f32) {
    if !session.health_seen {
        session.health_seen = true;
        return;
    }
    let dmg = old - new;
    if dmg <= 0.0 {
        if dmg < 0.0 {
            session.damage_cooldown = 10;
        }
    } else {
        session.hurt_ticks = HURT_TILT_TICKS;
        session.damage_cooldown = DAMAGE_COOLDOWN_TICKS;
    }
}

#[cfg(test)]
mod tests;
