//! The arrow tick and render state (`AbstractArrow.tick`, `ArrowRenderer.extractRenderState`).
use super::*;
use crate::arrow::{self, ArrowWorld};

/// A world for the arrow tick: an optional block the arrow is inside (the stuck check) and
/// an optional clip hit along its movement.
struct TestWorld {
    contains: bool,
    hit: Option<DVec3>,
    water: bool,
}

impl ArrowWorld for TestWorld {
    fn collision_contains(&self, _block: haste_core::BlockPos, _p: DVec3) -> bool {
        self.contains
    }
    fn clip(&self, _from: DVec3, _to: DVec3) -> Option<DVec3> {
        self.hit
    }
    fn in_water(&self, _pos: DVec3) -> bool {
        self.water
    }
}

fn arrow_entity(id: i32, pos: DVec3, velocity: DVec3) -> Entity {
    let mut e = Entity::new(&spawn(id, pos), &EntityDimensions::default());
    e.velocity = velocity;
    e.dimensions = (0.5, 0.5);
    e
}

fn entries(pairs: &[(u8, MetadataValue)]) -> Vec<MetadataEntry> {
    pairs
        .iter()
        .map(|(i, v)| MetadataEntry {
            index: *i,
            value: v.clone(),
        })
        .collect()
}

/// `Projectile.lerpRotation` blends a fifth of the way to the movement's angles
/// (`AbstractArrow.tick`).
#[test]
fn rotation_follows_movement_lerp() {
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(3.0, 0.0, 4.0));
    e.yaw = 0.0;
    e.pitch = 0.0;
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    // yRot = atan2(3, 4) = 36.87°, xRot = 0; a fifth of the way from 0.
    let want_yaw = 0.2f32 * (3.0f64.atan2(4.0).to_degrees() as f32);
    assert!((e.yaw - want_yaw).abs() < 1.0e-4, "yaw {}", e.yaw);
    assert!(e.pitch.abs() < 1.0e-6);
    // The wrap: a yaw near 180 eases through the shortest side (`Projectile.lerpRotation`
    // moves rotO by whole turns so the blend stays inside ±180 of it).
    let mut e = arrow_entity(2, DVec3::new(0.0, 64.0, 0.0), DVec3::new(-0.01, 0.0, 0.0));
    e.yaw = 179.0;
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    // The target is -90°, 19° the short way past 180; the result may sit either side of the
    // ±180 seam, so compare the wrapped distance travelled.
    let travelled = crate::store::wrap_degrees(e.yaw - 179.0);
    assert!(
        (travelled - 18.2f32).abs() < 0.5,
        "yaw must wrap the short way, travelled {travelled}"
    );
}

/// The flight step's order (`AbstractArrow.tick`): move by the velocity, then the air drag,
/// then gravity.
#[test]
fn flight_moves_then_drags_then_gravity() {
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(1.0, 1.0, 0.0));
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(e.pos, DVec3::new(1.0, 65.0, 0.0));
    // Air drag 0.99 on every axis, then gravity 0.05 off y.
    assert!((e.velocity.x - 0.99).abs() < 1.0e-12);
    assert!((e.velocity.y - (1.0 * 0.99 - 0.05)).abs() < 1.0e-12);
    assert!(e.velocity.z.abs() < 1.0e-12);
    // Water swaps the drag for the 0.6 inertia (and no gravity that tick: the branch is
    // `physicsEnabled && !isInGround`, unchanged) — the movement itself is unaffected.
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: None,
            water: true,
        },
        &mut particles,
    );
    assert_eq!(e.pos, DVec3::new(1.0, 64.0, 0.0));
    assert!((e.velocity.x - 0.6).abs() < 1.0e-12);
}

/// The clip's hit plants the arrow at the hit, zeroes the velocity and starts the shake
/// (`AbstractArrow.tick` -> `stepMoveAndHit` -> `onHitBlock`).
#[test]
fn clip_hit_sticks_the_arrow() {
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    e.tick_count = 5;
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: Some(DVec3::new(0.5, 64.0, 0.0)),
            water: false,
        },
        &mut particles,
    );
    assert_eq!(e.pos, DVec3::new(0.5, 64.0, 0.0));
    assert_eq!(e.velocity, DVec3::ZERO);
    assert!(e.arrow_in_ground);
    assert_eq!(e.shake_time, arrow::SHAKE_TICKS);
    // The next tick runs the stuck branch: no motion, the shake counts down.
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: Some(DVec3::new(0.5, 64.0, 0.0)),
            water: false,
        },
        &mut particles,
    );
    assert_eq!(e.pos, DVec3::new(0.5, 64.0, 0.0));
    assert_eq!(e.shake_time, arrow::SHAKE_TICKS - 1);
}

/// The stuck check at the tick start (`AbstractArrow.tick`'s `contains(position)`) also
/// grounds the arrow and shakes it — but not on the entity's first tick (`firstTick`).
#[test]
fn stuck_check_grounds_and_shakes() {
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    e.tick_count = 1;
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: true,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(e.velocity, DVec3::ZERO);
    assert!(e.arrow_in_ground);
    assert_eq!(e.shake_time, 0, "first tick: firstTick is still true");
    // The second tick in the same block: nothing new (already in ground, no shake to
    // restart — `shakeTime <= 0` but the flag no longer flips).
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: true,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(e.shake_time, 0);
}

/// A no-gravity arrow never falls (`Entity.getGravity` is 0 for it, so `applyGravity` does
/// nothing).
#[test]
fn no_gravity_arrow_stays_level() {
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    e.metadata.apply(&entries(&[(
        arrow::NO_GRAVITY_INDEX,
        MetadataValue::Boolean(true),
    )]));
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(e.pos, DVec3::new(1.0, 64.0, 0.0));
    assert!(e.velocity.y.abs() < 1.0e-12, "no-gravity arrow fell");
}

/// A fresh stick on a later tick shakes (`AbstractArrow.tick` at tick start) — and the same
/// tick's `shakeTime--` (which runs after the stuck check) already eats one of the seven,
/// exactly as in vanilla.
#[test]
fn stuck_check_after_first_tick_shakes() {
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    e.tick_count = 2;
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: true,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(e.shake_time, arrow::SHAKE_TICKS - 1);
}

/// `AbstractArrow.onSyncedDataUpdated`: a metadata packet flipping the in-ground flag to
/// true starts the shake, but not on the spawn metadata (firstTick) and not while one runs.
#[test]
fn in_ground_metadata_starts_the_shake() {
    let mut e = arrow_entity(1, DVec3::ZERO, DVec3::ZERO);
    // Before the first tick: the spawn metadata does not shake.
    arrow::on_metadata(
        &mut e,
        &entries(&[(arrow::IN_GROUND_INDEX, MetadataValue::Boolean(true))]),
    );
    assert!(e.arrow_in_ground);
    assert_eq!(e.shake_time, 0);
    // A packet after ticks: shake.
    e.tick_count = 3;
    e.shake_time = 0;
    arrow::on_metadata(
        &mut e,
        &entries(&[(arrow::IN_GROUND_INDEX, MetadataValue::Boolean(true))]),
    );
    assert_eq!(e.shake_time, arrow::SHAKE_TICKS);
    // Unsticking clears the flag without a shake; sticking again restarts it.
    arrow::on_metadata(
        &mut e,
        &entries(&[(arrow::IN_GROUND_INDEX, MetadataValue::Boolean(false))]),
    );
    assert!(!e.arrow_in_ground);
    arrow::on_metadata(
        &mut e,
        &entries(&[(arrow::IN_GROUND_INDEX, MetadataValue::Boolean(true))]),
    );
    assert_eq!(e.shake_time, arrow::SHAKE_TICKS);
    // While a shake runs, a re-stick does not restart it.
    e.shake_time = 3;
    arrow::on_metadata(
        &mut e,
        &entries(&[(arrow::IN_GROUND_INDEX, MetadataValue::Boolean(true))]),
    );
    assert_eq!(e.shake_time, 3);
    // Other indices mean other things to other entities; arrows ignore them.
    let mut e = arrow_entity(2, DVec3::ZERO, DVec3::ZERO);
    arrow::on_metadata(&mut e, &entries(&[(9, MetadataValue::Boolean(true))]));
    assert_eq!(e.shake_time, 0);
    assert!(!e.arrow_in_ground);
}

/// The kind of an entity type name, through the real registries.json (None without the
/// cache: these tests are skipped there).
fn kind_of(name: &str) -> Option<RegistryIndex> {
    crate::registry::entity_type_names()?
        .iter()
        .find(|(_, n)| **n == name)
        .map(|(k, _)| *k)
}

/// The render state (`ArrowRenderer.extractRenderState` / `TippableArrowRenderer`): the
/// tipped flag from the effect colour, the spectral flag by type, and the shake by
/// partial tick.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn decoration_reads_the_arrow_state() {
    let Some(arrow_kind) = kind_of("minecraft:arrow") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let Some(spectral) = kind_of("minecraft:spectral_arrow") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let Some(pig) = kind_of("minecraft:pig") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let mut e = arrow_entity(1, DVec3::ZERO, DVec3::ZERO);
    e.kind = arrow_kind;
    e.yaw = 10.0;
    e.pitch = -20.0;
    e.prev_yaw = 0.0;
    e.prev_pitch = 0.0;
    e.shake_time = 7;
    let Some(haste_core::model::Decoration::Arrow(a)) = arrow::decoration(&e, 0.5) else {
        panic!("arrow must carry the arrow decoration");
    };
    assert!((a.y_rot - 5.0).abs() < 1.0e-6);
    assert!((a.x_rot + 10.0).abs() < 1.0e-6);
    assert!((a.shake - 6.5).abs() < 1.0e-6);
    assert!(!a.tipped && !a.spectral);
    // A tipped arrow (effect colour > 0) and a spectral one.
    e.metadata.apply(&entries(&[(
        arrow::EFFECT_COLOR_INDEX,
        MetadataValue::VarInt(0x3F_FF_FF),
    )]));
    let Some(haste_core::model::Decoration::Arrow(a)) = arrow::decoration(&e, 0.0) else {
        panic!();
    };
    assert!(a.tipped);
    e.kind = spectral;
    let Some(haste_core::model::Decoration::Arrow(a)) = arrow::decoration(&e, 0.0) else {
        panic!();
    };
    assert!(a.spectral);
    assert!(!a.tipped, "the spectral arrow has no potion colour");
    // Any other type: no arrow decoration.
    e.kind = pig;
    assert!(arrow::decoration(&e, 0.0).is_none());
}

/// `Arrow.makeParticle`'s cadence: two colour particles a tick in flight, one every fifth
/// tick in the ground; `Arrow.handleEntityEvent`'s puff is twenty.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn tipped_arrow_particles() {
    let Some(arrow_kind) = kind_of("minecraft:arrow") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let mut e = arrow_entity(1, DVec3::ZERO, DVec3::ZERO);
    e.kind = arrow_kind;
    e.tick_count = 5;
    e.metadata.apply(&entries(&[(
        arrow::EFFECT_COLOR_INDEX,
        MetadataValue::VarInt(0x3F_FF_FF),
    )]));
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(particles.len(), 2, "two colour particles per flight tick");
    assert!(particles
        .iter()
        .all(|p| p.kind == arrow::ArrowParticleKind::Color(0x3F_FF_FF)));
    // Stuck: one particle every fifth in-ground tick (inGroundTime % 5 == 0).
    e.arrow_in_ground = true;
    e.velocity = DVec3::ZERO;
    particles.clear();
    for want in [0, 0, 0, 0, 1, 0, 0, 0, 0, 1] {
        arrow::tick(
            &mut e,
            &TestWorld {
                contains: true,
                hit: None,
                water: false,
            },
            &mut particles,
        );
        assert_eq!(particles.len(), want);
        particles.clear();
    }
    // The puff.
    particles.clear();
    assert!(arrow::potion_puff(&e, &mut particles));
    assert_eq!(particles.len(), 20);
    assert!(particles
        .iter()
        .all(|p| p.kind == arrow::ArrowParticleKind::Color(0x3F_FF_FF)));
}

/// `SpectralArrow.tick`: one white `effect` particle a tick while flying, at the arrow's
/// exact position, none once stuck.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn spectral_arrow_breathes_white_particles() {
    let Some(spectral) = kind_of("minecraft:spectral_arrow") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
    e.kind = spectral;
    e.spectral = true;
    e.tick_count = 5;
    let mut particles = Vec::new();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: false,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(particles.len(), 1);
    assert_eq!(particles[0].kind, arrow::ArrowParticleKind::SpectralSwirl);
    assert_eq!(particles[0].pos, e.pos, "at the ticked position");
    // Stuck: nothing.
    e.arrow_in_ground = true;
    e.velocity = DVec3::ZERO;
    particles.clear();
    arrow::tick(
        &mut e,
        &TestWorld {
            contains: true,
            hit: None,
            water: false,
        },
        &mut particles,
    );
    assert_eq!(particles.len(), 0);
}

/// The trident runs the same client tick (`ThrownTrident` extends `AbstractArrow`) and is not
/// drawn as an arrow.
#[test]
fn the_trident_is_an_abstract_arrow_but_not_an_arrow() {
    let (Some(trident), Some(arrow_kind), Some(pig)) = (
        kind_of("minecraft:trident"),
        kind_of("minecraft:arrow"),
        kind_of("minecraft:pig"),
    ) else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    assert!(arrow::is_abstract_arrow(trident) && !arrow::is_arrow(trident));
    assert!(arrow::is_abstract_arrow(arrow_kind));
    assert!(!arrow::is_abstract_arrow(pig));
}

/// `ThrownTrident.getWaterInertia`: a trident in water keeps 0.99 of its speed where an arrow
/// keeps 0.6 (then the air drag is skipped in water and gravity applies).
#[test]
fn a_trident_keeps_its_speed_in_water() {
    let Some(trident) = kind_of("minecraft:trident") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let water = TestWorld {
        contains: false,
        hit: None,
        water: true,
    };
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(0.0, 0.0, 2.0));
    e.kind = trident;
    let mut particles = Vec::new();
    arrow::tick(&mut e, &water, &mut particles);
    assert!((e.velocity.z - 2.0 * 0.99).abs() < 1e-9, "{:?}", e.velocity);
    assert!((e.velocity.y + arrow::GRAVITY).abs() < 1e-9);
    // The step itself is the movement read before the inertia (`AbstractArrow.tick`).
    assert!((e.pos.z - 2.0).abs() < 1e-9);
}

/// A returning trident (loyalty sets no-physics) is left to the server's position packets:
/// the homing `ThrownTrident.tick` adds is not modelled.
#[test]
fn a_returning_trident_is_not_moved_by_the_client_tick() {
    let Some(trident) = kind_of("minecraft:trident") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let mut e = arrow_entity(1, DVec3::new(0.0, 64.0, 0.0), DVec3::new(0.0, 0.0, 1.0));
    e.kind = trident;
    e.metadata.apply(&entries(&[(
        arrow::FLAGS_INDEX,
        MetadataValue::Byte(arrow::NO_PHYSICS_FLAG as i8),
    )]));
    let mut particles = Vec::new();
    let air = TestWorld {
        contains: false,
        hit: None,
        water: false,
    };
    arrow::tick(&mut e, &air, &mut particles);
    assert_eq!(e.pos, DVec3::new(0.0, 64.0, 0.0));
    assert_eq!(e.velocity, DVec3::new(0.0, 0.0, 1.0));
}
