//! The arm poses (`AvatarRenderer.getArmPose` for players, `HumanoidMobRenderer.getArmPose`
//! for humanoid mobs) and their application to the arms (`HumanoidModel.poseRightArm`/
//! `poseLeftArm`), checked against vanilla's rules.
use super::*;

/// Equipment stack: one of item `item` in wire slot `slot`, with `components_raw` for the
/// charged-crossbow check.
fn stack(slot: u8, item: u32, components_raw: Vec<u8>) -> (u8, haste_core::entity::ItemStackData) {
    (
        slot,
        haste_core::entity::ItemStackData {
            count: 1,
            item: Some(RegistryIndex(item)),
            components_raw: components_raw.into(),
        },
    )
}

/// The test item table: 2 stone, 4 bread, 5 shield, 6 bow, 7 potion, 8 crossbow, 9 spear
/// (arbitrary indices; the pose code only reads what the table says).
fn item_uses() -> crate::animation::ItemUses {
    use crate::animation::{ItemUse, UseAnimation as U};
    let mut items = vec![ItemUse::default(); 10];
    items[2] = ItemUse::default(); // stone: not used over time
    items[4] = ItemUse {
        animation: U::Eat,
        ..ItemUse::default()
    };
    items[5] = ItemUse {
        animation: U::Block,
        ..ItemUse::default()
    };
    items[6] = ItemUse {
        animation: U::Bow,
        ..ItemUse::default()
    };
    items[7] = ItemUse {
        animation: U::Drink,
        ..ItemUse::default()
    };
    items[8] = ItemUse {
        animation: U::Crossbow,
        crossbow: true,
        ..ItemUse::default()
    };
    items[9] = ItemUse {
        animation: U::Spear,
        spear: true,
        ..ItemUse::default()
    };
    crate::animation::ItemUses::new(items, Some(34))
}

/// A player entity (the tests' store shape) with `equipment` as its stacks, still owned by
/// the store: the tests read it back per case.
fn player(equipment: &[(u8, haste_core::entity::ItemStackData)]) -> EntityStore {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Alex".into(),
        display_name: None,
        latency: 30,
        game_mode: Some(GameMode::Creative),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    for (slot, item) in equipment {
        s.set_equipment(EntityId(1), *slot, item.clone());
    }
    s
}

/// Metadata update that only sets the living-flags byte.
fn living_flags(flags: i8) -> Vec<MetadataEntry> {
    vec![MetadataEntry {
        index: 8,
        value: MetadataValue::Byte(flags),
    }]
}

/// The (right, left) arm poses of the store's player entity under the test table.
fn poses(s: &EntityStore) -> (crate::animation::ArmPose, crate::animation::ArmPose) {
    crate::animation::poses(s.get(EntityId(1)).unwrap(), &item_uses())
}

/// An empty hand is the EMPTY pose; holding anything is the ITEM pose.
#[test]
fn empty_and_held_item_poses() {
    assert_eq!(
        poses(&player(&[])),
        arm_pose(crate::animation::ArmPose::Empty)
    );
    // A sword (stone here: any item without a use animation) in the main hand.
    assert_eq!(
        poses(&player(&[stack(0, 2, vec![])])),
        arm_pose(crate::animation::ArmPose::Item)
    );
}

/// An off-hand item mirrors onto the left arm.
#[test]
fn offhand_item_poses_the_left_arm() {
    assert_eq!(
        poses(&player(&[stack(1, 2, vec![])])),
        (
            crate::animation::ArmPose::Empty,
            crate::animation::ArmPose::Item
        )
    );
}

/// A left main arm (the synced `DATA_PLAYER_MAIN_HAND` byte) mirrors the hands.
#[test]
fn left_main_arm_mirrors_the_poses() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Alex".into(),
        display_name: None,
        latency: 30,
        game_mode: Some(GameMode::Creative),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    s.set_equipment(EntityId(1), 0, stack(0, 2, vec![]).1);
    s.set_metadata(
        EntityId(1),
        &[MetadataEntry {
            index: crate::avatar::DATA_PLAYER_MAIN_HAND,
            value: MetadataValue::Byte(1),
        }],
    );
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(
        crate::animation::poses(e, &item_uses()),
        (
            crate::animation::ArmPose::Empty,
            crate::animation::ArmPose::Item
        )
    );
}

/// Eating and drinking have no arm pose of their own in 26.3 (the enum has no EAT/DRINK):
/// the arm keeps the ITEM pose while the use runs.
#[test]
fn eating_and_drinking_keep_the_item_pose() {
    for item in [4u32, 7] {
        let mut s = player(&[stack(0, item, vec![])]);
        s.set_metadata(EntityId(1), &living_flags(0x01));
        assert_eq!(
            poses(&s).0,
            crate::animation::ArmPose::Item,
            "item {item} while using"
        );
    }
}

/// Using a shield is the BLOCK pose, on the hand the use runs on (the off-hand bit).
#[test]
fn using_a_shield_blocks_on_that_hand() {
    let mut s = player(&[stack(1, 5, vec![])]);
    // The use flag plus the off-hand bit: the off-hand use poses the left arm.
    s.set_metadata(EntityId(1), &living_flags(0x01 | 0x02));
    assert_eq!(poses(&s).1, crate::animation::ArmPose::Block);
    let mut s = player(&[stack(0, 5, vec![])]);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    assert_eq!(poses(&s).0, crate::animation::ArmPose::Block);
}

/// Drawing a bow is the two-handed BOW_AND_ARROW pose: the off hand drops to EMPTY when it
/// is empty, or stays at ITEM when it holds something (`getArmPose`'s outer rule).
#[test]
fn drawing_a_bow_is_two_handed() {
    let mut s = player(&[stack(0, 6, vec![])]);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    assert_eq!(
        poses(&s),
        (
            crate::animation::ArmPose::BowAndArrow,
            crate::animation::ArmPose::Empty
        )
    );
    let mut s = player(&[stack(0, 6, vec![]), stack(1, 2, vec![])]);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    assert_eq!(
        poses(&s),
        (
            crate::animation::ArmPose::BowAndArrow,
            crate::animation::ArmPose::Item
        )
    );
}

/// A charged crossbow at rest is held up (`CROSSBOW_HOLD`); an uncharged one is just an
/// ITEM, and charging it is the `CROSSBOW_CHARGE` pose.
#[test]
fn charged_crossbow_at_rest_is_held_up() {
    // The stack's patch adds one component, the charged-projectiles type (34).
    let charged = vec![1, 0, 34];
    assert_eq!(
        poses(&player(&[stack(0, 8, charged)])).0,
        crate::animation::ArmPose::CrossbowHold
    );
    // Uncharged (an empty patch).
    assert_eq!(
        poses(&player(&[stack(0, 8, vec![])])).0,
        crate::animation::ArmPose::Item
    );
    // While charging, the use takes over.
    let mut s = player(&[stack(0, 8, vec![])]);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    assert_eq!(poses(&s).0, crate::animation::ArmPose::CrossbowCharge);
}

/// Holding a spear is the SPEAR pose even without a use (`usesSpearPose`).
#[test]
fn holding_a_spear_is_the_spear_pose() {
    assert_eq!(
        poses(&player(&[stack(0, 9, vec![])])).0,
        crate::animation::ArmPose::Spear
    );
}

/// Humanoid mobs hold only spears up: everything else hangs at the EMPTY pose
/// (`HumanoidMobRenderer.getArmPose`).
#[test]
fn humanoid_mobs_only_hold_spears_up() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.set_equipment(EntityId(1), 0, stack(0, 2, vec![]).1);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(
        crate::animation::poses(e, &item_uses()),
        (
            crate::animation::ArmPose::Empty,
            crate::animation::ArmPose::Empty
        )
    );
    s.set_equipment(EntityId(1), 0, stack(0, 9, vec![]).1);
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(
        crate::animation::poses(e, &item_uses()).0,
        crate::animation::ArmPose::Spear
    );
}

/// The ITEM pose bends the arm to `walkSwing / 2 - pi / 10` (`HumanoidModel.poseRightArm`),
/// through the whole `setup_anim` path.
#[test]
fn item_pose_bends_the_arm() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Alex".into(),
        display_name: None,
        latency: 30,
        game_mode: Some(GameMode::Creative),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    s.set_equipment(EntityId(1), 0, stack(0, 2, vec![]).1);
    s.tick();
    let mut out = Vec::new();
    s.render_instances(0.5, &mut out);
    let rot = rotations_by_name(&out[0].pose);
    let arm = rot.get("right_arm").unwrap();
    // The walk swing is 0 standing still; the idle bob adds up to 0.05 in x and z.
    assert!(
        (arm.x - -std::f32::consts::PI / 10.0).abs() <= 0.051,
        "right arm x {} != -pi/10",
        arm.x
    );
    assert!(arm.y.abs() < 1e-6, "the ITEM pose zeroes the arm yaw");
}

/// The whole use path through the store: the living flags the server syncs while another
/// player eats drive the arm pose (the use state `haste-entity/src/using_item.rs` tracks).
#[test]
fn synced_use_flags_pose_the_eating_arm() {
    let mut s = store();
    let sp = spawn(1, DVec3::ZERO);
    let uuid = sp.uuid;
    s.player_info(&[PlayerInfo {
        uuid,
        name: "Alex".into(),
        display_name: None,
        latency: 30,
        game_mode: Some(GameMode::Creative),
        listed: true,
        skin_texture_url: None,
    }]);
    s.spawn(&sp);
    // Bread in the main hand, then the server's use flag.
    s.set_equipment(EntityId(1), 0, stack(0, 4, vec![]).1);
    s.set_metadata(EntityId(1), &living_flags(0x01));
    s.tick();
    let e = s.get(EntityId(1)).unwrap();
    assert_eq!(
        crate::animation::poses(e, &item_uses()).0,
        crate::animation::ArmPose::Item
    );
}

/// Helper for compact pose assertions.
fn arm_pose(
    p: crate::animation::ArmPose,
) -> (crate::animation::ArmPose, crate::animation::ArmPose) {
    (p, crate::animation::ArmPose::Empty)
}
