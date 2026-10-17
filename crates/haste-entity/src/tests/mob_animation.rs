use super::*;
/// `VillagerModel.setupAnim`: an unhappy villager (`AbstractVillager.DATA_UNHAPPY_COUNTER`,
/// metadata index 18, > 0) shakes its head (`zRot = 0.3*sin(0.45*ageInTicks)`, `xRot = 0.4`),
/// and the legs swing at half the humanoid amplitude.
#[test]
fn villager_unhappy_shakes_head_and_halves_walk() {
    let model = mob_model(
        "entity/villager/villager",
        &["head", "arms", "right_leg", "left_leg"],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 18,
            value: MetadataValue::VarInt(1),
        }],
    );
    for _ in 0..10 {
        s.move_delta(EntityId(1), DVec3::new(0.1, 0.0, 0.0), true);
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let head = rot.get("head").unwrap();
    assert!((head.x - 0.4).abs() < 1e-6, "unhappy head x {}", head.x);
    let age = e.tick_count as f32;
    assert!(
        (head.z - 0.3 * (0.45 * age).sin()).abs() < 1e-5,
        "unhappy head z {}",
        head.z
    );
    let pos = e.walk.position(0.0);
    let speed = e.walk.speed(0.0);
    let want = (pos * 0.6662).cos() * 1.4 * speed * 0.5;
    let rl = rot.get("right_leg").unwrap();
    assert!(
        (rl.x - want).abs() < 1e-5,
        "villager leg {} != {want}",
        rl.x
    );
}

/// `IllagerModel.setupAnim`: a calm vindicator crosses its arms (the "arms" part shown, the two
/// arms hidden); an aggressive one attacks with `animateZombieArms(aggressive = true)`
/// (`armDrop = -PI / 1.5`).
#[test]
fn illager_crosses_arms_and_attacks_when_aggressive() {
    let model = mob_model(
        "entity/illager/vindicator",
        &[
            "head",
            "arms",
            "right_arm",
            "left_arm",
            "right_leg",
            "left_leg",
        ],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    assert!(pose.hidden.iter().any(|h| h == "right_arm"));
    assert!(pose.hidden.iter().any(|h| h == "left_arm"));
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 15,
            value: MetadataValue::Byte(0x04),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    assert!(pose.hidden.iter().any(|h| h == "arms"));
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let ra = rot.get("right_arm").unwrap();
    assert!(
        (ra.x - -2.094_395_2).abs() < 1e-4,
        "attacking illager arm {} != -PI/1.5",
        ra.x
    );
}

/// `IronGolemModel.setupAnim`: entity event 4 starts a 10-tick attack animation; at the first
/// frame `attackTicksRemaining = 10`, `triangleWave(10, 10) = 1`, so both arms are at -0.5.
#[test]
fn iron_golem_attack_arms() {
    let model = mob_model(
        "entity/iron_golem/iron_golem",
        &[
            "head",
            "body",
            "right_arm",
            "left_arm",
            "right_leg",
            "left_leg",
        ],
    );
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.mark_attack(EntityId(1));
    let e = s.get(EntityId(1)).unwrap();
    let pose = crate::animation::setup_anim(e, &model, true, 0.0, false);
    let rot: std::collections::HashMap<_, _> = pose.rotations.iter().cloned().collect();
    let ra = rot.get("right_arm").unwrap();
    let la = rot.get("left_arm").unwrap();
    assert!((ra.x - -0.5).abs() < 1e-5, "golem arm {} != -0.5", ra.x);
    assert!((la.x - -0.5).abs() < 1e-5);
}

/// `CreeperRenderer.scale`: a fresh creeper is unscaled; a swelling one grows wider (x/z) and
/// taller (y) from `getSwelling(partial) = lerp(partial, oldSwell, swell) / (maxSwell - 2)`.
#[test]
fn creeper_swell_scales_the_model() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(
        crate::animation::creeper_scale(e, 0.0),
        haste_core::math::Vec3::ONE
    );
    // Swell direction 1 (index 16, int) for 28 ticks: oldSwell 27, swell 28.
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: 16,
            value: MetadataValue::VarInt(1),
        }],
    );
    for _ in 0..28 {
        s.tick();
    }
    let e = s.get(EntityId(1)).unwrap();
    let sc = crate::animation::creeper_scale(e, 0.0);
    assert!((sc.x - sc.z).abs() < 1e-6, "x/z should match");
    assert!(sc.x > 1.0, "swollen creeper x {} not wider", sc.x);
    assert!(sc.y > 1.0, "swollen creeper y {} not taller", sc.y);
}
