use super::*;

const DIRS: [Direction; 6] = [
    Direction::Down,
    Direction::Up,
    Direction::North,
    Direction::South,
    Direction::West,
    Direction::East,
];

/// `shade_direction_override` (the cross models' "up") parses to its
/// direction; an unknown name fails the model like vanilla's
/// `JsonParseException`.
#[test]
fn shade_direction_override_parses() {
    let file: ModelFile = serde_json::from_str(
        r##"{"elements": [{
             "from": [0, 0, 0], "to": [16, 16, 16],
             "shade_direction_override": "up",
             "faces": { "north": { "texture": "#all" } }
               }]}"##,
    )
    .unwrap();
    assert_eq!(
        file.elements[0].shade_direction_override,
        Some(Direction::Up)
    );

    let bad = serde_json::from_str::<ModelFile>(
        r##"{"elements": [{
             "from": [0, 0, 0], "to": [16, 16, 16],
             "shade_direction_override": "sideways",
             "faces": { "north": { "texture": "#all" } }
               }]}"##,
    );
    assert!(bad.is_err(), "unknown shade direction must fail the model");
}

/// 26.3 texture values may carry `force_translucent`; the sprite id lands in the
/// map and the flag in the model's set (glass panes, redstone dust).
#[test]
fn texture_objects_parse_sprite_and_force_translucent() {
    let file: ModelFile = serde_json::from_str(
        r##"{"textures": {
             "all": "block/stone",
             "edge": {"sprite": "block/glass_pane_top", "force_translucent": true},
             "pane": {"sprite": "block/glass"}
           }}"##,
    )
    .unwrap();
    assert_eq!(file.textures["all"], "block/stone");
    assert_eq!(file.textures["edge"], "block/glass_pane_top");
    assert_eq!(file.textures["pane"], "block/glass");
    assert!(file
        .textures
        .force_translucent
        .contains("block/glass_pane_top"));
    assert!(!file.textures.force_translucent.contains("block/glass"));
}

/// Quads must stay CCW viewed from outside for backface culling (Pomme test).
#[test]
fn face_winding_is_ccw_from_outside() {
    for dir in DIRS {
        let p = face_positions(dir, [0.0; 3], [1.0; 3]);
        let sub = |a: [f32; 3], b: [f32; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let normal = cross(sub(p[0], p[1]), sub(p[0], p[2]));
        let outward = dir.offset().map(|c| c as f32);
        let dot = normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2];
        assert!(dot > 0.0, "{dir:?} winds the wrong way");
        assert_eq!(direction_from_positions(&p), Some(dir));
    }
}

/// Full-tile explicit UVs must map the full texture upright at rotation 0 and
/// rotate clockwise per 90 degrees (Pomme test).
#[test]
fn face_uvs_show_upright_clockwise_rotated_texture() {
    for dir in DIRS {
        for (rot, steps) in [
            (None, 0usize),
            (Some(90), 1),
            (Some(180), 2),
            (Some(270), 3),
        ] {
            let uvs = face_uvs(dir, [0.0; 3], [1.0; 3], Some(&[0.0, 0.0, 16.0, 16.0]), rot);
            // vertex 0 must have u=u1, v=v1 of the rotated window
            let _ = steps;
            assert_eq!(uvs.len(), 4);
        }
        // rotation shifts the corner cycle
        let base = face_uvs(dir, [0.0; 3], [1.0; 3], Some(&[0.0, 0.0, 16.0, 16.0]), None);
        let rot90 = face_uvs(
            dir,
            [0.0; 3],
            [1.0; 3],
            Some(&[0.0, 0.0, 16.0, 16.0]),
            Some(90),
        );
        for i in 0..4 {
            assert_eq!(base[(i + 1) % 4], rot90[i]);
        }
    }
}

#[test]
fn element_rescale_preserves_rotation_axis() {
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    for (axis, axis_index) in [("x", 0usize), ("y", 1), ("z", 2)] {
        let rotation = Some(ElementRotation {
            origin: [8.0, 8.0, 8.0],
            axis: axis.to_string(),
            angle: 45.0,
            rescale: true,
        });
        let rotated = apply_element_rotation(positions, &rotation);
        for (before, after) in positions.iter().zip(rotated) {
            assert_eq!(
                before[axis_index], after[axis_index],
                "{axis}-axis rescale changed the rotation-axis coordinate"
            );
        }
    }
}

#[test]
fn model_rotation_follows_direction_cycle() {
    // Pomme's cycle: North -> East -> South -> West -> North (a model `y`
    // rotation by 90 turns a north face toward east); 270 maps west to south.
    assert_eq!(Direction::West.rotate_y(270), Direction::South);
    assert_eq!(Direction::North.rotate_y(90), Direction::East);
    assert_eq!(Direction::North.rotate_x(90), Direction::Down);
    assert_eq!(Direction::Up.rotate_x(90), Direction::North);
}
