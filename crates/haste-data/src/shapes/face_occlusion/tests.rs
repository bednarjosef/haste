//! Tests for `BlockShapes::shape_occludes` (vanilla `LightEngine.shapeOccludes` /
//! `Shapes.faceShapeOccludes`): hand-made states — slabs, stairs per facing, snow layers,
//! a full block, air — plus the old-dump fallback and `is_empty_shape`.

use haste_core::{BlockStateId, Direction};

use crate::{BlockShapes, BlockStates};

/// air = 0, stone = 1 (the default state and placeholder). The states' occlusion shapes
/// and flags come from the real 26.3 dump's values for these blocks (see NOTES.md): a
/// bottom slab is the half box with `use_shape_for_light_occlusion`, stairs are the base
/// slab plus the top box on their solid side.
const BLOCKS_JSON: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:stone_slab": {
    "properties": { "type": ["bottom", "top"] },
    "states": [
      { "id": 2, "default": true, "properties": { "type": "bottom" } },
      { "id": 3, "properties": { "type": "top" } }
    ]
  },
  "minecraft:oak_stairs": {
    "properties": { "facing": ["east", "west", "north"] },
    "states": [
      { "id": 4, "default": true, "properties": { "facing": "east" } },
      { "id": 5, "properties": { "facing": "west" } },
      { "id": 6, "properties": { "facing": "north" } }
    ]
  },
  "minecraft:snow": { "states": [ { "id": 7, "default": true } ] }
}"#;

/// One fixture state: (id, occlusion boxes, light dampening, propagates skylight down,
/// can occlude, use shape for light occlusion).
const STATES: &[(u32, &str, u8, bool, bool, bool)] = &[
    (0, "[]", 0, true, false, false),
    (1, "[[0,0,0,1,1,1]]", 15, false, true, false),
    (2, "[[0,0,0,1,0.5,1]]", 0, true, true, true),
    (3, "[[0,0.5,0,1,1,1]]", 0, true, true, true),
    (
        4,
        "[[0,0,0,1,0.5,1],[0.5,0.5,0,1,1,1]]",
        0,
        true,
        true,
        true,
    ),
    (
        5,
        "[[0,0,0,1,0.5,1],[0,0.5,0,0.5,1,1]]",
        0,
        true,
        true,
        true,
    ),
    (
        6,
        "[[0,0,0,1,0.5,1],[0,0.5,0,1,1,0.5]]",
        0,
        true,
        true,
        true,
    ),
    (7, "[[0,0,0,1,0.25,1]]", 0, true, true, true),
];

/// The fixture dump; `flags` false omits the two light-shape flags (an old dump).
fn oracle(flags: bool) -> String {
    let mut out = String::from("{\n  \"version\": 1,\n  \"states\": [\n");
    for (i, &(id, occlusion, dampening, skylight, can_occlude, use_shape)) in
        STATES.iter().enumerate()
    {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!(
            "    {{ \"id\": {id}, \"occlusion\": {occlusion}, \"light_dampening\": {dampening}, \
             \"propagates_skylight_down\": {skylight}"
        ));
        if flags {
            out.push_str(&format!(
                ", \"can_occlude\": {can_occlude}, \
                 \"use_shape_for_light_occlusion\": {use_shape}"
            ));
        }
        out.push_str(" }");
    }
    out.push_str("\n  ]\n}");
    out
}

fn shapes(flags: bool) -> BlockShapes {
    let blocks = BlockStates::parse(BLOCKS_JSON, None).unwrap();
    BlockShapes::parse_oracle(&oracle(flags), &blocks).unwrap()
}

const AIR: BlockStateId = BlockStateId(0);
const STONE: BlockStateId = BlockStateId(1);
const SLAB_BOTTOM: BlockStateId = BlockStateId(2);
const SLAB_TOP: BlockStateId = BlockStateId(3);
const STAIRS_EAST: BlockStateId = BlockStateId(4);
const STAIRS_WEST: BlockStateId = BlockStateId(5);
const STAIRS_NORTH: BlockStateId = BlockStateId(6);
const SNOW: BlockStateId = BlockStateId(7);

/// A bottom slab blocks light crossing its bottom face — down out of the slab and up into
/// it — but not its open top, and not sideways through its open half (the sideways face
/// shape is the whole half-height shape, which covers only half the plane). A top slab is
/// the mirror image.
#[test]
fn slabs_block_through_their_solid_face_only() {
    let s = shapes(true);
    assert!(
        s.shape_occludes(SLAB_BOTTOM, AIR, Direction::Down),
        "down out"
    );
    assert!(s.shape_occludes(AIR, SLAB_BOTTOM, Direction::Up), "up in");
    assert!(
        !s.shape_occludes(SLAB_BOTTOM, AIR, Direction::Up),
        "open top"
    );
    assert!(
        !s.shape_occludes(AIR, SLAB_BOTTOM, Direction::Down),
        "down in"
    );
    for d in [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        assert!(!s.shape_occludes(SLAB_BOTTOM, AIR, d), "sideways out {d:?}");
        assert!(!s.shape_occludes(AIR, SLAB_BOTTOM, d), "sideways in {d:?}");
        // two slabs side by side: the open top halves align, light passes
        assert!(
            !s.shape_occludes(SLAB_BOTTOM, SLAB_BOTTOM, d),
            "slab-slab {d:?}"
        );
    }
    assert!(s.shape_occludes(SLAB_TOP, AIR, Direction::Up), "up out");
    assert!(s.shape_occludes(AIR, SLAB_TOP, Direction::Down), "down in");
    assert!(
        !s.shape_occludes(SLAB_TOP, AIR, Direction::Down),
        "open bottom"
    );
    assert!(!s.shape_occludes(AIR, SLAB_TOP, Direction::Up), "up in");
}

/// Stairs block on their solid sides and through their full bottom, and pass light
/// everywhere their shape is open (the top half of the open side, upward through the open
/// half of the top face).
#[test]
fn stairs_block_on_their_solid_sides_per_facing() {
    let s = shapes(true);
    // An east-facing stair (its top box on the east): blocked east, open west.
    assert!(
        s.shape_occludes(STAIRS_EAST, AIR, Direction::East),
        "east out"
    );
    assert!(
        s.shape_occludes(AIR, STAIRS_EAST, Direction::West),
        "west in"
    );
    assert!(
        !s.shape_occludes(STAIRS_EAST, AIR, Direction::West),
        "west out"
    );
    assert!(
        !s.shape_occludes(AIR, STAIRS_EAST, Direction::East),
        "east in"
    );
    // A west-facing stair is the mirror image.
    assert!(
        s.shape_occludes(STAIRS_WEST, AIR, Direction::West),
        "west out"
    );
    assert!(
        s.shape_occludes(AIR, STAIRS_WEST, Direction::East),
        "east in"
    );
    assert!(
        !s.shape_occludes(STAIRS_WEST, AIR, Direction::East),
        "east out"
    );
    assert!(
        !s.shape_occludes(AIR, STAIRS_WEST, Direction::West),
        "west in"
    );
    // A north-facing stair (top box on the north): blocked north, open south.
    assert!(
        s.shape_occludes(STAIRS_NORTH, AIR, Direction::North),
        "north out"
    );
    assert!(
        !s.shape_occludes(STAIRS_NORTH, AIR, Direction::South),
        "south out"
    );
    // Every stair's bottom face is the full base slab.
    for stairs in [STAIRS_EAST, STAIRS_WEST, STAIRS_NORTH] {
        assert!(s.shape_occludes(stairs, AIR, Direction::Down), "bottom out");
        assert!(s.shape_occludes(AIR, stairs, Direction::Up), "bottom in");
        assert!(
            !s.shape_occludes(stairs, AIR, Direction::Up),
            "open top half up"
        );
    }
}

/// Snow layers behave like a thin bottom slab: blocked through the full bottom face, open
/// top, open sides. A full block and air keep the pre-shape behaviour (their occlusion
/// never applies: stone fails `use_shape_for_light_occlusion`, air `canOcclude`).
#[test]
fn snow_blocks_down_and_full_block_and_air_never_occlude_by_shape() {
    let s = shapes(true);
    assert!(s.shape_occludes(SNOW, AIR, Direction::Down), "snow bottom");
    assert!(!s.shape_occludes(SNOW, AIR, Direction::Up), "snow open top");
    assert!(
        !s.shape_occludes(SNOW, AIR, Direction::East),
        "snow open side"
    );
    // Stone and air: not shape-occluders (dampening still blocks stone in the engine).
    for from in [STONE, AIR] {
        for to in [STONE, AIR, SLAB_BOTTOM] {
            assert!(
                !s.shape_occludes(from, to, Direction::Down),
                "{from:?} -> {to:?}"
            );
        }
    }
    assert!(s.is_empty_shape(AIR), "air is an empty shape");
    assert!(s.is_empty_shape(STONE), "stone fails use_shape");
    assert!(!s.is_empty_shape(SLAB_BOTTOM), "the slab uses its shape");
    assert_eq!(s.use_shape_for_light_occlusion(SLAB_BOTTOM), Some(true));
    assert_eq!(s.use_shape_for_light_occlusion(STONE), Some(false));
}

/// Old dumps (no flags) and unknown state ids: no shape test at all, and the placeholder
/// (stone) answers for ids past the table.
#[test]
fn old_dumps_and_unknown_ids_keep_the_pre_shape_behaviour() {
    let old = shapes(false);
    for from in [AIR, STONE, SLAB_BOTTOM, STAIRS_EAST, SNOW] {
        for to in [AIR, STONE, SLAB_BOTTOM, STAIRS_EAST, SNOW] {
            assert!(
                !old.shape_occludes(from, to, Direction::Down),
                "{from:?} -> {to:?}"
            );
        }
    }
    assert!(old.is_empty_shape(SLAB_BOTTOM), "no flags: empty shape");
    assert_eq!(old.use_shape_for_light_occlusion(SLAB_BOTTOM), None);
    // Unknown ids resolve to the placeholder (stone): not a shape occluder, but still
    // occluded by a full face (the slab's solid bottom blocks into anything).
    let s = shapes(true);
    let unknown = BlockStateId(99);
    assert!(!s.shape_occludes(unknown, SLAB_BOTTOM, Direction::Down));
    assert!(s.shape_occludes(SLAB_BOTTOM, unknown, Direction::Down));
    assert!(s.is_empty_shape(unknown));
    assert_eq!(s.use_shape_for_light_occlusion(unknown), Some(false));
}
