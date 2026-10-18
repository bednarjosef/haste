//! Damage-feedback tests: the sound tables against vanilla's class overrides, the
//! damage-type effects lookup through the synced registry, and the request built for
//! a hit (event, volume, category, pitch range, position).

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::Identifier;
use haste_protocol::nbt::{Nbt, NbtCompound};
use haste_protocol::packets::configuration::clientbound::RegistryEntry;
use haste_protocol::SyncedRegistries;

use super::*;

/// A minimal do-nothing world sink (the session's `apply` needs one; the damage paths
/// write nothing to the world).
#[derive(Default)]
struct NullWorld;

impl super::super::WorldSink for NullWorld {
    fn load_chunk(
        &mut self,
        _d: &haste_protocol::haste_core::chunk::ChunkData,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn unload_chunk(&mut self, _p: haste_protocol::haste_core::ChunkPos) {}
    fn set_block(
        &mut self,
        _p: haste_protocol::haste_core::BlockPos,
        _s: haste_core::BlockStateId,
    ) {
    }
    fn apply_light(
        &mut self,
        _p: haste_protocol::haste_core::ChunkPos,
        _l: &haste_protocol::haste_core::chunk::LightData,
    ) {
    }
    fn replace_biomes(&mut self, _p: haste_protocol::haste_core::ChunkPos, _d: &[u8]) {}
    fn set_block_entity(
        &mut self,
        _p: haste_protocol::haste_core::BlockPos,
        _d: haste_protocol::haste_core::chunk::BlockEntityData,
    ) {
    }
    fn set_biome_zoom_seed(&mut self, _s: i64) {}
    fn block_state_at(&self, _p: haste_protocol::haste_core::BlockPos) -> haste_core::BlockStateId {
        haste_core::BlockStateId::AIR
    }
    fn reset(&mut self, _d: &haste_protocol::haste_core::game::DimensionInfo) {}
    fn chunk_count(&self) -> usize {
        0
    }
    fn non_empty_sections(&self) -> usize {
        0
    }
}

/// A minimal recording entity sink (only the damage paths write anything).
#[derive(Default)]
struct NullEntities {
    log: Vec<String>,
    /// Registry index per entity id (the spawn's `kind`), like the store's.
    kinds: std::collections::HashMap<EntityId, i32>,
}

impl super::super::EntitySink for NullEntities {
    fn spawn(&mut self, s: &haste_protocol::haste_core::entity::EntitySpawn) {
        self.kinds.insert(s.id, s.kind.0 as i32);
    }
    fn remove(&mut self, ids: &[EntityId]) {
        for id in ids {
            self.kinds.remove(id);
        }
    }
    fn move_delta(&mut self, _id: EntityId, _d: DVec3, _on_ground: bool) {}
    fn set_rotation(&mut self, _id: EntityId, _yaw: f32, _pitch: f32, _on_ground: bool) {}
    fn teleport(&mut self, _id: EntityId, _p: DVec3, _yaw: f32, _pitch: f32, _on_ground: bool) {}
    fn set_head_yaw(&mut self, _id: EntityId, _yaw: f32) {}
    fn set_velocity(&mut self, _id: EntityId, _v: DVec3) {}
    fn mark_hurt(&mut self, id: EntityId) {
        self.log.push(format!("hurt {}", id.0));
    }
    fn mark_dead(&mut self, _id: EntityId) {}
    fn set_metadata(
        &mut self,
        _id: EntityId,
        _e: &[haste_protocol::haste_core::entity::MetadataEntry],
    ) {
    }
    fn set_equipment(
        &mut self,
        _id: EntityId,
        _slot: u8,
        _item: haste_protocol::haste_core::entity::ItemStackData,
    ) {
    }
    fn player_info(&mut self, _i: &[haste_protocol::haste_core::entity::PlayerInfo]) {}
    fn set_passengers(&mut self, _v: EntityId, _p: &[EntityId]) {}
    fn pose(&self, _id: EntityId) -> Option<(DVec3, f32, f32)> {
        Some((DVec3::ZERO, 0.0, 0.0))
    }
    fn clear(&mut self) {}
    fn tick(&mut self) {}
    fn len(&self) -> usize {
        0
    }
    fn entity_type(&self, id: EntityId) -> Option<haste_core::RegistryIndex> {
        self.kinds
            .get(&id)
            .map(|k| haste_core::RegistryIndex(*k as u32))
    }
}

/// One session with both sinks, applying packets like `NetDriver::receive` does.
struct DamageRig {
    session: Session,
    world: NullWorld,
    entities: NullEntities,
    regs: SyncedRegistries,
    out: Vec<haste_protocol::ServerboundPlay>,
}

impl DamageRig {
    fn new() -> Self {
        Self {
            session: Session::new(
                EntityId(7),
                haste_core::game::GameMode::Survival,
                haste_core::game::DimensionInfo::overworld(),
                std::sync::Arc::new(Vec::new()),
            ),
            world: NullWorld,
            entities: NullEntities::default(),
            regs: SyncedRegistries::default(),
            out: Vec::new(),
        }
    }

    fn apply(&mut self, p: haste_protocol::ClientboundPlay) {
        let Self {
            session,
            world,
            entities,
            regs,
            out,
        } = self;
        session.apply(p, world, entities, regs, out);
    }

    fn damage(&mut self, entity_id: i32, source_type: i32) {
        self.apply(haste_protocol::ClientboundPlay::DamageEvent(
            haste_protocol::packets::play::clientbound::DamageEvent {
                entity_id,
                source_type,
                source_cause: 0,
                source_direct: 0,
                source_pos: None,
            },
        ));
    }

    fn set_health(&mut self, health: f32) {
        self.apply(haste_protocol::ClientboundPlay::SetHealth(
            haste_protocol::packets::play::clientbound::SetHealth {
                health,
                food: 20,
                saturation: 5.0,
            },
        ));
    }

    fn end_tick(&mut self) {
        let world = &mut self.world;
        self.session.end_tick(
            &haste_core::input::InputState::default(),
            world,
            &self.entities,
            &mut self.out,
        );
        // The hurt and death counters tick with the camera (vanilla's per-tick step).
        self.session.tick_camera(None, false);
    }
}

/// A synced `minecraft:damage_type` registry with one entry's NBT.
fn regs_with_damage_types(entries: &[(&str, Option<NbtCompound>)]) -> SyncedRegistries {
    let mut regs = SyncedRegistries::default();
    regs.insert(
        Identifier::minecraft("damage_type"),
        entries
            .iter()
            .map(|(id, nbt)| RegistryEntry {
                id: Identifier::parse(id).unwrap(),
                data: nbt
                    .clone()
                    .map(|n| Nbt::new(String::new(), n).write_unnamed()),
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
    assert_eq!(damage_effects_id(0, &SyncedRegistries::default()), "hurt");
}

#[test]
fn player_hurt_sound_follows_the_damage_effects() {
    // The DamageEffects -> SoundEvents mapping (Player.getHurtSound).
    assert_eq!(player_hurt_sound("hurt"), "minecraft:entity.player.hurt");
    assert_eq!(player_hurt_sound("thorns"), "minecraft:entity.player.hurt");
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
    assert_eq!(
        other.sound,
        SoundRef::Inline("minecraft:entity.bat.hurt".to_owned())
    );
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

#[test]
fn damage_event_on_the_local_player_tilts_blinks_and_sounds() {
    let mut r = DamageRig::new();
    // The synced damage types: index 1 is drowning.
    r.regs = regs_with_damage_types(&[
        ("minecraft:generic", None),
        ("minecraft:drown", Some(damage_type_nbt("drowning"))),
    ]);
    r.damage(7, 1);
    // Vanilla `LivingEntity.handleDamageEvent` for our own entity: hurtTime 10 (the
    // camera tilt), damageCooldownTime 20 (the HUD blink's gate) and the hurt sound.
    assert_eq!(r.session.hurt_ticks, 10);
    assert_eq!(r.session.damage_cooldown, 20);
    let [SoundRequest::Play(sound)] = &r.session.sound_requests[..] else {
        panic!("no hurt sound recorded");
    };
    assert_eq!(
        sound.sound,
        SoundRef::Inline("minecraft:entity.player.hurt_drown".to_owned())
    );
    assert_eq!(sound.category, 7);
    assert_eq!(sound.pos, SoundPos::Fixed(DVec3::ZERO));
    // The cooldown ticks down (vanilla `LivingEntity.tick`).
    for _ in 0..5 {
        r.end_tick();
    }
    assert_eq!(r.session.damage_cooldown, 15);
    assert_eq!(r.session.hurt_ticks, 5);
}

#[test]
fn damage_event_on_another_entity_sounds_from_the_store() {
    let mut r = DamageRig::new();
    r.session.set_entity_type_names(vec![
        Identifier::minecraft("player"),
        Identifier::minecraft("zombie"),
    ]);
    r.apply(haste_protocol::ClientboundPlay::AddEntity(
        haste_protocol::packets::play::clientbound::AddEntity {
            entity_id: 42,
            uuid: Default::default(),
            kind: 1,
            pos: DVec3::new(10.0, 70.0, 10.0),
            velocity: DVec3::ZERO,
            pitch: 0.0,
            yaw: 0.0,
            head_yaw: 0.0,
            data: 0,
        },
    ));
    r.damage(42, 0);
    // No tilt or blink of our own: the victim is another entity.
    assert_eq!(r.session.hurt_ticks, 0);
    assert_eq!(r.session.damage_cooldown, 0);
    let [SoundRequest::Play(sound)] = &r.session.sound_requests[..] else {
        panic!("no hurt sound recorded");
    };
    assert_eq!(
        sound.sound,
        SoundRef::Inline("minecraft:entity.zombie.hurt".to_owned())
    );
    // The hostile category, played from the entity at drain time.
    assert_eq!(sound.category, 5);
    assert_eq!(sound.pos, SoundPos::Entity(42));
    // The entity is marked for the red flash.
    assert!(r.entities.log.iter().any(|l| l == "hurt 42"));
}

#[test]
fn set_health_drop_tilts_and_blinks_like_hurt_to() {
    let mut r = DamageRig::new();
    // The first set_health after spawn only stores the health (vanilla
    // `flashOnSetHealth`): no tilt, no blink window.
    r.set_health(12.0);
    assert_eq!(r.session.hurt_ticks, 0);
    assert_eq!(r.session.damage_cooldown, 0);
    // The second one is a real drop (vanilla `LocalPlayer.hurtTo`): tilt and cooldown.
    r.set_health(8.0);
    assert_eq!(r.session.hurt_ticks, 10);
    assert_eq!(r.session.damage_cooldown, 20);
    assert_eq!(r.session.player.health, 8.0);
    // A heal: the short cooldown (10), no tilt (the drop's 10 ticks keep counting down).
    r.set_health(20.0);
    assert_eq!(r.session.hurt_ticks, 10);
    assert_eq!(r.session.damage_cooldown, 10);
}
