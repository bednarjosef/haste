//! The armour stand's armour shapes and the small-model detection, against vanilla's
//! `ArmorStandArmorModel` / `HumanoidModel.BABY_TRANSFORMER` (26.3 client).
use super::*;
use haste_core::model::EquipmentSlot;

/// The body model's stand-in for the detection test: the stand texture and a base plate at
/// `plate_y`, which is all `small_of` reads.
fn stand_body(plate_y: f32) -> CubeModel {
    CubeModel {
        texture: Identifier::minecraft("entity/armorstand/armorstand"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![ModelPart {
                name: "base_plate".into(),
                id: parts::intern("base_plate"),
                pivot: Vec3::new(0.0, plate_y, 0.0),
                rotation: Vec3::ZERO,
                visible: true,
                cubes: Vec::new(),
                children: Vec::new(),
            }],
        },
    }
}

fn part_of<'a>(m: &'a CubeModel, name: &str) -> &'a ModelPart {
    fn find<'a>(p: &'a ModelPart, name: &str) -> Option<&'a ModelPart> {
        if p.name == name {
            return Some(p);
        }
        p.children.iter().find_map(|c| find(c, name))
    }
    find(&m.root, name).unwrap()
}

#[test]
fn armour_shapes_match_the_vanilla_layers() {
    let head = armor_for(EquipmentSlot::Head, false);
    // Head slot: head + hat (the hat is what the body model never draws).
    assert_eq!(head.root.children.len(), 1);
    assert_eq!(part_of(&head, "head").pivot, Vec3::new(0.0, -23.0, 0.0));
    assert_eq!(part_of(&head, "head").cubes[0].inflate, 1.0);
    assert_eq!(part_of(&head, "hat").cubes[0].inflate, 1.5);
    // Chest: body + both arms at the outer deformation.
    let chest = armor_for(EquipmentSlot::Chest, false);
    assert_eq!(chest.root.children.len(), 3);
    assert_eq!(part_of(&chest, "body").cubes[0].inflate, 1.0);
    assert!(part_of(&chest, "left_arm").cubes[0].mirror);
    // Legs: the inner body plus legs at 0.4 (0.5 - 0.1), pivoted one unit above the
    // humanoid's (vanilla y = 11, not 12).
    let legs = armor_for(EquipmentSlot::Legs, false);
    assert_eq!(legs.root.children.len(), 3);
    assert_eq!(part_of(&legs, "body").cubes[0].inflate, 0.5);
    assert_eq!(
        part_of(&legs, "right_leg").pivot,
        Vec3::new(-1.9, -13.0, 0.0)
    );
    assert_eq!(part_of(&legs, "left_leg").cubes[0].inflate, 0.4);
    // Feet: legs only, at the outer deformation less 0.1.
    let feet = armor_for(EquipmentSlot::Feet, false);
    assert_eq!(feet.root.children.len(), 2);
    assert_eq!(part_of(&feet, "right_leg").cubes[0].inflate, 0.9);
    // The armour texture layout is the 64x32 humanoid one.
    assert_eq!(legs.tex_width, 64);
    assert_eq!(legs.tex_height, 32);
}

#[test]
fn small_armour_is_the_baby_transform() {
    let small = armor_for(EquipmentSlot::Chest, true);
    // Heads scale 3/4 (vanilla pivot (0, 17, 0) * 0.75 = (0, 12.75, 0) -> Haste -11.25);
    // body parts 1/2 around the ground (arms' vanilla (±5, 26, 0) * 0.5 = (±2.5, 13, 0)).
    assert_eq!(part_of(&small, "body").pivot, Vec3::new(0.0, -12.0, 0.0));
    assert_eq!(
        part_of(&small, "right_arm").pivot,
        Vec3::new(-2.5, -11.0, 0.0)
    );
    // The leg pivots (vanilla (±1.9, 11, 0) + 24, halved) land half a unit under the body's.
    let small_legs = armor_for(EquipmentSlot::Legs, true);
    assert_eq!(
        part_of(&small_legs, "right_leg").pivot,
        Vec3::new(-0.95, -6.5, 0.0)
    );
    // Cubes scale too, UVs do not.
    assert_eq!(
        part_of(&small, "body").cubes[0].size,
        Vec3::new(4.0, 6.0, 2.0)
    );
    assert_eq!(part_of(&small, "body").cubes[0].uv, [16, 16]);
    // The hat follows the head's 3/4.
    let small_head = armor_for(EquipmentSlot::Head, true);
    assert_eq!(
        part_of(&small_head, "head").pivot,
        Vec3::new(0.0, -11.25, 0.0)
    );
    assert_eq!(
        part_of(&small_head, "hat").cubes[0].size,
        Vec3::new(6.0, 6.0, 6.0)
    );
}

#[test]
fn small_of_reads_the_body_model() {
    // The big plate sits at -12 (a unit above the ground), the small one at -6.
    assert_eq!(small_of(&stand_body(-12.0)), Some(false));
    assert_eq!(small_of(&stand_body(-6.0)), Some(true));
    // Any other model is not an armour stand.
    let mut player = stand_body(-12.0);
    player.texture = Identifier::minecraft("entity/player/wide/steve");
    assert_eq!(small_of(&player), None);
}

/// The stand's texture exists in the client jar (the renderer loads it on demand by
/// identifier; a typo would draw the placeholder with only a log line).
#[test]
#[ignore = "needs the client jar in the cache"]
fn armor_stand_texture_exists_in_the_jar() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    assets
        .texture_any(&Identifier::minecraft("entity/armorstand/armorstand"))
        .unwrap_or_else(|e| panic!("armor stand texture: {e:#}"));
}
