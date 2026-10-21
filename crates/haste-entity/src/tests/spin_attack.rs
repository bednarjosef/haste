//! The riptide spin layer (`SpinAttackEffectLayer` and `SpinAttackEffectModel`): each box's yaw
//! for an age, its scale and y offset, the baked boxes, and the living flag that turns it on.
use glam::Vec3;
use haste_core::entity::{MetadataEntry, MetadataValue};
use haste_core::model::{Cube, EntityInstance, PoseOverrides};
use haste_core::parts;

use crate::metadata::Metadata;
use crate::spin_attack::{box_y_offset, box_yaw, dress, layer, pose};

fn assert_near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}");
}

fn assert_vec_near(a: Vec3, b: Vec3) {
    assert_near(a.x, b.x);
    assert_near(a.y, b.y);
    assert_near(a.z, b.z);
}

/// A player instance with no overlays (the fields `dress` reads and writes).
fn player_instance() -> EntityInstance {
    EntityInstance {
        id: haste_core::EntityId(1),
        model: crate::models::humanoid(false),
        pos: glam::DVec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: haste_core::model::BodyTransform::default(),
        body_scale: Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.5,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 0.6,
        height: 1.8,
        is_discrete: false,
        name_tag_y: 1.8,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}

/// The layer's cube of the named box.
fn box_cube(name: &str) -> Cube {
    let model = layer();
    let part = model
        .root
        .children
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no {name} box"));
    part.cubes[0].clone()
}

#[test]
fn the_boxes_lift_as_vanilla_creates_them() {
    assert_near(box_y_offset(0), 6.4);
    assert_near(box_y_offset(1), 16.0);
}

#[test]
fn the_boxes_keep_vanilla_cube_sizes_at_the_model_origin() {
    // Origin (-8, -16 + yOffset, -8) and size (16, 32, 16), unscaled so the box UVs cover the
    // 64 x 64 texture; the part sits at the model origin, 24 units above the feet.
    let b0 = box_cube("box0");
    assert_vec_near(b0.origin, Vec3::new(-8.0, -9.6, -8.0));
    assert_vec_near(b0.size, Vec3::new(16.0, 32.0, 16.0));
    assert_eq!(b0.uv, [0, 0]);
    let b1 = box_cube("box1");
    assert_vec_near(b1.origin, Vec3::new(-8.0, 0.0, -8.0));
    assert_vec_near(b1.size, Vec3::new(16.0, 32.0, 16.0));
    for part in &layer().root.children {
        assert_vec_near(part.pivot, Vec3::new(0.0, -24.0, 0.0));
    }
    assert_eq!((layer().tex_width, layer().tex_height), (64, 64));
}

#[test]
fn dress_adds_the_layer_only_while_spinning() {
    let mut inst = player_instance();
    let mut m = Metadata::default();
    dress(&mut inst, &m, 4.0);
    assert!(inst.overlays[1].is_none());
    m.apply(&[MetadataEntry {
        index: 8,
        value: MetadataValue::Byte(0x04),
    }]);
    dress(&mut inst, &m, 4.0);
    assert!(inst.overlays[1]
        .as_ref()
        .is_some_and(|o| std::sync::Arc::ptr_eq(&o.model, &layer())));
    let r1 = inst.pose.rotation_of(parts::intern("box1")).unwrap();
    assert_vec_near(r1, Vec3::new(0.0, 140.0f32.to_radians(), 0.0));
}

#[test]
fn the_layer_draws_the_riptide_texture() {
    assert_eq!(
        layer().texture,
        haste_core::Identifier::minecraft("textures/entity/trident/trident_riptide.png")
    );
}

#[test]
fn box_yaw_turns_each_box_against_the_age() {
    // angle = age * -(45 + (i + 1) * 5), wrapped to [-180, 180) degrees (`Mth.wrapDegrees`).
    assert_near(box_yaw(0, 0.0), 0.0);
    assert_near(box_yaw(0, 1.0), (-50.0f32).to_radians());
    assert_near(box_yaw(1, 1.0), (-55.0f32).to_radians());
    // -200 and -220 wrap to 160 and 140.
    assert_near(box_yaw(0, 4.0), 160.0f32.to_radians());
    assert_near(box_yaw(1, 4.0), 140.0f32.to_radians());
    // -500 wraps to -140.
    assert_near(box_yaw(0, 10.0), (-140.0f32).to_radians());
}

#[test]
fn the_pose_sets_both_boxes_yaw_for_the_age() {
    let mut p = PoseOverrides::default();
    pose(&mut p, 4.0);
    let r0 = p.rotation_of(parts::intern("box0")).expect("box0 posed");
    let r1 = p.rotation_of(parts::intern("box1")).expect("box1 posed");
    assert_vec_near(r0, Vec3::new(0.0, 160.0f32.to_radians(), 0.0));
    assert_vec_near(r1, Vec3::new(0.0, 140.0f32.to_radians(), 0.0));
}

#[test]
fn the_spin_flag_is_bit_four_of_the_living_byte() {
    let set = |b: i8| {
        let mut m = Metadata::default();
        m.apply(&[MetadataEntry {
            index: 8,
            value: MetadataValue::Byte(b),
        }]);
        m.living.auto_spin_attack
    };
    assert!(set(0x04));
    assert!(set(0x04 | 0x01));
    assert!(!set(0x01 | 0x02));
    assert!(!set(0));
}
