//! Remote-entity interpolation against the public `EntityStore` API: position syncs, head yaw,
//! spawn body yaw, baby walk cadence, death, and the yaw/pitch lerp (the vanilla classes are
//! named at each test).
use haste_core::entity::{EntitySpawn, MetadataEntry, MetadataValue};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::DVec3;
use haste_entity::{EntityDimensions, EntityStore};
use uuid::Uuid;

fn store() -> EntityStore {
    EntityStore::new(EntityDimensions::default())
}

fn spawn(id: i32, pos: DVec3) -> EntitySpawn {
    EntitySpawn {
        id: EntityId(id),
        uuid: Uuid::new_v4(),
        kind: RegistryIndex(0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn near_f(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

// ---------------------------------------------------------------------------
// Item 2: entity_position_sync interpolates (ClientPacketListener.handleEntityPositionSync).
// ---------------------------------------------------------------------------

/// A sync within 64 blocks goes through `Entity.moveOrInterpolateTo`: three 1/remaining steps
/// (`LinearInterpolationHandler.doInterpolate`), landing exactly on the third tick.
#[test]
fn position_sync_within_64_blocks_interpolates_over_three_ticks() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(9.0, 0.0, 0.0), 0.0, 0.0, true);
    // Nothing moves until the tick: the renderer must not see a snap.
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos, DVec3::ZERO);
    assert_eq!(e.prev_pos, DVec3::ZERO);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near(e.pos.x, 3.0), "tick 1: {}", e.pos.x);
    assert_eq!(e.prev_pos, DVec3::ZERO);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near(e.pos.x, 6.0), "tick 2: {}", e.pos.x);
    assert!(near(e.prev_pos.x, 3.0));
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos.x, 9.0);
    s.tick();
    assert_eq!(s.get(EntityId(1)).unwrap().pos.x, 9.0);
}

/// `distanceToSqr > 4096` snaps (`Entity.snapTo`): both positions jump at once and no step
/// follows.
#[test]
fn position_sync_beyond_64_blocks_snaps() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(65.0, 0.0, 0.0), 0.0, 0.0, true);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos.x, 65.0);
    assert_eq!(e.prev_pos.x, 65.0);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.pos.x, 65.0);
    assert_eq!(e.prev_pos.x, 65.0);
}

/// Exactly 64 blocks is not "too big" (the test is strictly greater than 4096).
#[test]
fn position_sync_at_exactly_64_blocks_still_interpolates() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(64.0, 0.0, 0.0), 0.0, 0.0, true);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near(e.pos.x, 64.0 / 3.0), "{}", e.pos.x);
}

/// A sync that lands on the target already in flight does not restart the window
/// (`AbstractInterpolationHandler.interpolateTo` skips an identical target).
#[test]
fn position_sync_to_the_current_target_does_not_restart_the_window() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.move_delta(EntityId(1), DVec3::new(6.0, 0.0, 0.0), true);
    s.tick(); // 2.0
    s.teleport(EntityId(1), DVec3::new(6.0, 0.0, 0.0), 0.0, 0.0, true);
    s.tick(); // still step 2 of 3: half of the remaining 4 -> 4.0
    assert!(near(s.get(EntityId(1)).unwrap().pos.x, 4.0));
    s.tick();
    assert_eq!(s.get(EntityId(1)).unwrap().pos.x, 6.0);
}

/// A delta after a sync builds on the synced position (the position codec's base is the sync).
#[test]
fn delta_after_position_sync_builds_on_the_synced_target() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(10.0, 0.0, 0.0), 0.0, 0.0, true);
    s.move_delta(EntityId(1), DVec3::new(2.0, 0.0, 0.0), true);
    for _ in 0..3 {
        s.tick();
    }
    assert_eq!(s.get(EntityId(1)).unwrap().pos.x, 12.0);
}

fn metadata(index: u8, value: MetadataValue) -> MetadataEntry {
    MetadataEntry { index, value }
}

// ---------------------------------------------------------------------------
// Item 4: head yaw eases over 3 ticks (LivingEntity.lerpHeadTo / lerpHeadRotationStep).
// ---------------------------------------------------------------------------

/// `handleRotateMob` -> `lerpHeadTo(yaw, 3)`: nothing turns on arrival; `aiStep` then rotLerps
/// by 1/3, 1/2 and 1/1 of the remaining difference (30, 60, 90 degrees), with `baseTick`
/// recording the previous value each tick for `getYHeadRot(partial)`.
#[test]
fn head_yaw_eases_over_three_ticks() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_head_yaw(EntityId(1), 90.0);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.head_yaw, 0.0);
    assert_eq!(e.head_yaw_at(0.5), 0.0);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.head_yaw, 30.0), "tick 1: {}", e.head_yaw);
    assert!(near_f(e.head_yaw_at(0.0), 0.0));
    assert!(near_f(e.head_yaw_at(0.5), 15.0));
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.head_yaw, 60.0), "tick 2: {}", e.head_yaw);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.head_yaw, 90.0), "tick 3: {}", e.head_yaw);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.head_yaw, 90.0));
    assert!(near_f(e.head_yaw_at(0.5), 90.0));
}

/// `Mth.rotLerp` takes the short way round the wrap: 170 -> -170 moves +20, not -340.
#[test]
fn head_yaw_lerp_wraps_the_short_way() {
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.head_yaw = 170.0;
    s.spawn(&sp);
    s.set_head_yaw(EntityId(1), -170.0);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.head_yaw, 170.0 + 20.0 / 3.0), "{}", e.head_yaw);
}

/// A second packet mid-ease replaces the target and restarts the three steps.
#[test]
fn head_yaw_packet_mid_ease_restarts_the_steps() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_head_yaw(EntityId(1), 90.0);
    s.tick(); // 30
    s.set_head_yaw(EntityId(1), 0.0);
    s.tick(); // 30 - 10 = 20
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.head_yaw, 20.0), "{}", e.head_yaw);
}

// ---------------------------------------------------------------------------
// Item 14: spawn body yaw from the head yaw (LivingEntity.recreateFromPacket).
// ---------------------------------------------------------------------------

/// `recreateFromPacket` sets `yBodyRot` and `yHeadRot` (and their `O` copies) from the add_entity
/// packet's head yaw, so a mob entering view with its head turned does not start with its body
/// facing the look direction and then swing round.
#[test]
fn spawn_body_yaw_comes_from_the_head_yaw() {
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.yaw = 10.0;
    sp.head_yaw = 70.0;
    s.spawn(&sp);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.body_yaw, 70.0);
    assert_eq!(e.prev_body_yaw, 70.0);
    assert_eq!(e.body_yaw_at(0.5), 70.0);
    assert_eq!(e.head_yaw, 70.0);
    assert_eq!(e.yaw, 10.0);
}

// ---------------------------------------------------------------------------
// Item 9: babies' walk animation position scale (LivingEntity.updateWalkAnimation).
// ---------------------------------------------------------------------------

/// `updateWalkAnimation` passes `positionScale = 3.0` for `isBaby()`: the same speed, but the
/// limb-swing position (`WalkAnimationState.position(partial)`) advances three times as fast.
#[test]
fn baby_walk_position_advances_three_times_as_fast() {
    let walk_position = |baby: bool| {
        let mut s = store();
        s.spawn(&spawn(1, DVec3::ZERO));
        if baby {
            // `AgeableMob.DATA_BABY_ID` (index 16).
            s.set_metadata(EntityId(1), &[metadata(16, MetadataValue::Boolean(true))]);
        }
        for _ in 0..4 {
            s.move_delta(EntityId(1), DVec3::new(0.3, 0.0, 0.0), true);
            s.tick();
        }
        let e = s.get(EntityId(1)).unwrap();
        (e.walk.position(1.0), e.walk.speed(1.0))
    };
    let (adult_pos, adult_speed) = walk_position(false);
    let (baby_pos, baby_speed) = walk_position(true);
    assert!(adult_pos > 0.0);
    assert!(
        near_f(baby_pos, adult_pos * 3.0),
        "{baby_pos} vs {adult_pos}"
    );
    assert!(near_f(baby_speed, adult_speed));
}

// ---------------------------------------------------------------------------
// Item 15: death from health <= 0, red overlay until removal.
// ---------------------------------------------------------------------------

fn hurt(s: &EntityStore) -> f32 {
    let mut out = Vec::new();
    s.render_instances(0.0, &mut out);
    out[0].hurt
}

/// `LivingEntityRenderer.extractRenderState`: `hasRedOverlay = hurtTime > 0 || deathTime > 0`,
/// and `deathTime` only ever grows, so a dead entity stays red until `remove_entities`, however
/// late that packet is.
#[test]
fn dead_entity_keeps_the_red_overlay_past_twenty_ticks() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_dead(EntityId(1));
    for _ in 0..40 {
        assert_eq!(hurt(&s), 1.0);
        s.tick();
    }
    assert_eq!(hurt(&s), 1.0);
    assert!(s.get(EntityId(1)).unwrap().dead);
}

/// `LivingEntity.isDeadOrDying` is `getHealth() <= 0 || dead`: health 0 in the metadata starts
/// `tickDeath` without entity event 3 (the server does not send the event on every path).
#[test]
fn health_zero_in_metadata_starts_the_death() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(EntityId(1), &[metadata(9, MetadataValue::Float(4.0))]);
    assert!(!s.get(EntityId(1)).unwrap().dead);
    assert_eq!(hurt(&s), 0.0);
    s.set_metadata(EntityId(1), &[metadata(9, MetadataValue::Float(0.0))]);
    let e = s.get(EntityId(1)).unwrap();
    assert!(e.dead);
    assert_eq!(hurt(&s), 1.0);
    // The death counter starts now: `deathTime` is 20 - hurt_ticks for `animation::death_time`.
    assert_eq!(e.hurt_ticks, 20);
}

/// Event 3 arriving after the health metadata (or the reverse) does not restart the counter.
#[test]
fn second_death_trigger_does_not_restart_the_counter() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(EntityId(1), &[metadata(9, MetadataValue::Float(-1.0))]);
    for _ in 0..5 {
        s.tick();
    }
    assert_eq!(s.get(EntityId(1)).unwrap().hurt_ticks, 15);
    s.mark_dead(EntityId(1));
    assert_eq!(s.get(EntityId(1)).unwrap().hurt_ticks, 15);
}

/// A non-living float at index 9 above zero (or an int) is not a death.
#[test]
fn positive_health_is_not_a_death() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(EntityId(1), &[metadata(9, MetadataValue::VarInt(0))]);
    assert!(!s.get(EntityId(1)).unwrap().dead);
    s.set_metadata(EntityId(1), &[metadata(9, MetadataValue::Float(0.5))]);
    assert!(!s.get(EntityId(1)).unwrap().dead);
}

// ---------------------------------------------------------------------------
// Item 3: yaw/pitch from move packets ease with the position (LinearInterpolationHandler).
// ---------------------------------------------------------------------------

/// `handleMoveEntity` -> `moveOrInterpolateTo(yRot, xRot)`: `doInterpolate` moves yaw by
/// `Mth.rotLerp` and pitch by `Mth.lerp`, 1/remaining per tick; `getYRot(partial)` /
/// `getXRot(partial)` read the previous/current pair.
#[test]
fn rotation_packet_eases_yaw_and_pitch_over_three_ticks() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_rotation(EntityId(1), 90.0, 30.0, true);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.yaw, 0.0);
    assert_eq!(e.pitch, 0.0);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.yaw, 30.0), "{}", e.yaw);
    assert!(near_f(e.pitch, 10.0), "{}", e.pitch);
    assert!(near_f(e.yaw_at(0.5), 15.0));
    assert!(near_f(e.pitch_at(0.5), 5.0));
    assert_eq!(e.yaw_at(1.0), e.yaw);
    assert_eq!(e.pitch_at(1.0), e.pitch);
    s.tick();
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.yaw, 90.0), "{}", e.yaw);
    assert!(near_f(e.pitch, 30.0), "{}", e.pitch);
    assert!(near_f(e.prev_yaw, 60.0));
    assert!(near_f(e.prev_pitch, 20.0));
}

/// `interpolateTo` with no rotation keeps the rotation already in flight, and with no position
/// keeps the position in flight: a pos+rot packet split into two calls lands on both.
#[test]
fn position_and_rotation_targets_are_kept_for_each_other() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_rotation(EntityId(1), 90.0, 0.0, true);
    s.tick(); // yaw 30
    s.move_delta(EntityId(1), DVec3::new(3.0, 0.0, 0.0), true);
    s.tick(); // restarted: yaw 30 + 60/3 = 50, x = 1
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.yaw, 50.0), "{}", e.yaw);
    assert!(near(e.pos.x, 1.0), "{}", e.pos.x);
    s.set_rotation(EntityId(1), 0.0, 0.0, true);
    for _ in 0..3 {
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.yaw, 0.0), "{}", e.yaw);
    assert_eq!(e.pos.x, 3.0);
}

/// A rotation past the wrap takes the short way (`Mth.rotLerp`), pitch does not wrap.
#[test]
fn yaw_lerp_takes_the_short_way_round() {
    let mut s = store();
    let mut sp = spawn(1, DVec3::ZERO);
    sp.yaw = 170.0;
    s.spawn(&sp);
    s.set_rotation(EntityId(1), -170.0, 0.0, true);
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert!(near_f(e.yaw, 170.0 + 20.0 / 3.0), "{}", e.yaw);
    assert!(near_f(e.yaw_at(0.5), 170.0 + 10.0 / 3.0));
}

/// A far position sync (`snapTo`) sets the rotation and its previous values at once.
#[test]
fn far_sync_snaps_rotation_too() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.teleport(EntityId(1), DVec3::new(0.0, 100.0, 0.0), 45.0, -10.0, true);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(e.yaw_at(0.5), 45.0);
    assert_eq!(e.pitch_at(0.5), -10.0);
}

/// The render list carries the interpolated pitch (and yaw for non-living models).
#[test]
fn render_instances_use_the_interpolated_pitch() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_rotation(EntityId(1), 0.0, 30.0, true);
    s.tick();
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    assert!(near_f(out[0].pitch, 5.0), "{}", out[0].pitch);
}
