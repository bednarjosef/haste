//! Damage-feedback tests: the sound tables against vanilla's class overrides, the
//! damage-type effects lookup through the synced registry, and the request built for
//! a hit (event, volume, category, pitch range, position).

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::Identifier;
use haste_protocol::nbt::NbtCompound;
use haste_protocol::packets::configuration::clientbound::RegistryEntry;
use haste_protocol::SyncedRegistries;

use super::*;

/// A synced `minecraft:damage_type` registry with one entry's NBT.
fn regs_with_damage_types(entries: &[(&str, Option<NbtCompound>)]) -> SyncedRegistries {
    let mut regs = SyncedRegistries::default();
    regs.insert(
        Identifier::minecraft("damage_type"),
        entries
            .iter()
            .map(|(id, nbt)| RegistryEntry {
                id: Identifier::parse(id).unwrap(),
                data: nbt.clone().map(|n| n.write_unnamed()),
            })
            .collect(),
    );
    regs
}

fn damage_type_nbt(effects: &str) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    nbt.put_string("message_id", "generic".into());
    nbt.put_string("scaling", "when_caused_by_living_non_player".into());
    nbt.put_float("exhaustion", 0.0);
    nbt.put_string("effects", effects.into());
    nbt
}

#[test]
fn damage_effects_id_reads_the_synced_entry() {
    let regs = regs_with_damage_types(&[
        ("minecraft:generic", None),
        ("minecraft:drown", Some(damage_type_nbt("drowning"))),
        ("minecraft:freeze", Some(damage_type_nbt("freezing"))),
    ]);
    // The packet's index resolves through the synced registry's entry order.
    assert_eq!(damage_effects_id(1, &regs), "drowning");
    assert_eq!(damage_effects_id(2, &regs), "freezing");
    // No effects field: vanilla's codec default is "hurt".
    assert_eq!(damage_effects_id(0, &regs), "hurt");
    // Out-of-range index or missing registry: the default, no panic.
    assert_eq!(damage_effects_id(99, &regs), "hurt");
    assert_eq!(
        damage_effects_id(0, &SyncedRegistries::default()),
        "hurt"
    );
}

#[test]
fn player_hurt_sound_follows_the_damage_effects() {
    // The DamageEffects -> SoundEvents mapping (Player.getHurtSound).
    assert_eq!(
        player_hurt_sound("hurt"),
        "minecraft:entity.player.hurt"
    );
    assert_eq!(
        player_hurt_sound("thorns"),
        "minecraft:entity.player.hurt"
    );
    assert_eq!(
        player_hurt_sound("drowning"),
        "minecraft:entity.player.hurt_drown"
    );
    assert_eq!(
        player_hurt_sound("burning"),
        "minecraft:entity.player.hurt_on_fire"
    );
    assert_eq!(
        player_hurt_sound("poking"),
        "minecraft:entity.player.hurt_sweet_berry_bush"
    );
    assert_eq!(
        player_hurt_sound("freezing"),
        "minecraft:entity.player.hurt_freeze"
    );
}

#[test]
fn mobs_play_their_own_hurt_sounds() {
    // Spot checks of vanilla's getHurtSound overrides.
    assert_eq!(
        hurt_sound(Some("minecraft:zombie"), "hurt"),
        "minecraft:entity.zombie.hurt"
    );
    assert_eq!(
        hurt_sound(Some("minecraft:skeleton_horse"), "hurt"),
        "minecraft:entity.skeleton_horse.hurt"
    );
    // A spider's subclass inherits its sound.
    assert_eq!(
        hurt_sound(Some("minecraft:cave_spider"), "hurt"),
        "minecraft:entity.spider.hurt"
    );
    // A monster without its own override plays Monster's HOSTILE_HURT.
    assert_eq!(
        hurt_sound(Some("minecraft:giant"), "hurt"),
        "minecraft:entity.hostile.hurt"
    );
    // The armor stand's "hurt" is the hit event.
    assert_eq!(
        hurt_sound(Some("minecraft:armor_stand"), "hurt"),
        "minecraft:entity.armor_stand.hit"
    );
    // The damage type's effects only pick a player's sound.
    assert_eq!(
        hurt_sound(Some("minecraft:zombie"), "drowning"),
        "minecraft:entity.zombie.hurt"
    );
    // Unknown types (a mod's) play vanilla's living default; no panic.
    assert_eq!(
        hurt_sound(Some("minecraft:mod_mobs:gloop"), "hurt"),
        "minecraft:entity.generic.hurt"
    );
    assert_eq!(hurt_sound(None, "hurt"), "minecraft:entity.generic.hurt");
}

#[test]
fn hurt_volumes_follow_get_sound_volume() {
    assert_eq!(hurt_volume(Some("minecraft:bat"), None), 0.1);
    assert_eq!(hurt_volume(Some("minecraft:cow"), None), 0.4);
    assert_eq!(hurt_volume(Some("minecraft:horse"), None), 0.8);
    assert_eq!(hurt_volume(Some("minecraft:ghast"), None), 5.0);
    assert_eq!(hurt_volume(Some("minecraft:warden"), None), 4.0);
    // A baby (synced data 16 set) ghastling is quiet, an adult happy ghast is loud.
    assert_eq!(hurt_volume(Some("minecraft:happy_ghast"), Some(1)), 1.0);
    assert_eq!(hurt_volume(Some("minecraft:happy_ghast"), None), 4.0);
    // The cube mobs scale with their size (their synced data 16).
    assert_eq!(hurt_volume(Some("minecraft:slime"), Some(4)), 1.6);
    assert_eq!(hurt_volume(Some("minecraft:zombie"), None), 1.0);
}

#[test]
fn hurt_categories_follow_get_sound_source() {
    assert_eq!(hurt_category(Some("minecraft:player")), 7);
    assert_eq!(hurt_category(Some("minecraft:zombie")), 5);
    assert_eq!(hurt_category(Some("minecraft:cow")), 6);
    // The sulfur cube overrides Monster's HOSTILE with NEUTRAL.
    assert_eq!(hurt_category(Some("minecraft:sulfur_cube")), 6);
    // Unknown types take Entity's default.
    assert_eq!(hurt_category(None), 6);
}

#[test]
fn hurt_request_plays_at_the_victim_with_vanilla_pitch() {
    let mut rng = LegacyRandom::new(42);
    // Our own hurt: the player sound, PLAYER category, at our position.
    let SoundRequest::Play(own) = hurt_request(
        &mut rng,
        Some("minecraft:player"),
        "drowning",
        None,
        7,
        true,
        DVec3::new(1.0, 64.0, 2.0),
    ) else {
        panic!("not a play request");
    };
    assert_eq!(
        own.sound,
        SoundRef::Inline("minecraft:entity.player.hurt_drown".to_owned())
    );
    assert_eq!(own.category, 7);
    assert_eq!(own.pos, SoundPos::Fixed(DVec3::new(1.0, 64.0, 2.0)));
    assert_eq!(own.volume, 1.0);
    // The hurt pitch is 1 +/- 0.2 (two uniform draws), never outside.
    assert!((0.8..=1.2).contains(&own.pitch));
    // Another entity: resolved from the store at play time, its own category and volume.
    let SoundRequest::Play(other) = hurt_request(
        &mut rng,
        Some("minecraft:bat"),
        "hurt",
        None,
        42,
        false,
        DVec3::ZERO,
    ) else {
        panic!("not a play request");
    };
    assert_eq!(other.sound, SoundRef::Inline("minecraft:entity.bat.hurt".to_owned()));
    assert_eq!(other.category, 6);
    assert_eq!(other.pos, SoundPos::Entity(42));
    assert_eq!(other.volume, 0.1);
    assert!((0.8..=1.2).contains(&other.pitch));
    // Two hits never share a variant seed.
    let SoundRequest::Play(again) = hurt_request(
        &mut rng,
        Some("minecraft:player"),
        "hurt",
        None,
        7,
        true,
        DVec3::ZERO,
    ) else {
        panic!("not a play request");
    };
    assert_ne!(own.seed, again.seed);
}
