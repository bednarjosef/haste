//! Tests for terrain face culling from the dump's occlusion shapes (`FaceOccluders`,
//! vanilla `Block.shouldRenderFace`): a fence and a slab neighbour, the gate ordering
//! (a full-block neighbour hides even a face whose own shape is empty), and the `None`
//! fallback for old dumps and unknown ids.

use haste_assets::Direction;
use haste_core::BlockStateId;
use haste_data::GameData;

use crate::mesher::FaceOccluders;

/// air 0, stone 1, oak_fence 2, bottom slab 3, top slab 4, glass 5.
const BLOCKS_JSON: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:oak_fence": { "states": [ { "id": 2, "default": true } ] },
  "minecraft:stone_slab": {
    "properties": { "type": ["bottom", "top"] },
    "states": [
      { "id": 3, "default": true, "properties": { "type": "bottom" } },
      { "id": 4, "properties": { "type": "top" } }
    ]
  },
  "minecraft:glass": { "states": [ { "id": 5, "default": true } ] }
}"#;

/// The occlusion shapes: the fence the thin post of `FenceBlock.getOcclusionShape`
/// (`makeShapes(4, 16, 2, 6, 15)`, touching only the cell floor), the slabs half cubes.
const ORACLE_JSON: &str = r#"{
  "version": 1,
  "states": [
    { "id": 0, "occlusion": [] },
    { "id": 1, "occlusion": [[0,0,0,1,1,1]] },
    { "id": 2, "occlusion": [[0.375,0,0.375,0.625,0.9375,0.625]] },
    { "id": 3, "occlusion": [[0,0,0,1,0.5,1]] },
    { "id": 4, "occlusion": [[0,0.5,0,1,1,1]] },
    { "id": 5, "occlusion": [] }
  ]
}"#;

fn occluders(oracle: &str) -> FaceOccluders {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("haste-render-face-test-{}-{n}", std::process::id()));
    let reports = dir.join("reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("blocks.json"), BLOCKS_JSON).unwrap();
    std::fs::write(
        reports.join("registries.json"),
        r#"{
  "minecraft:block": {
    "default": "minecraft:air",
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:stone": { "protocol_id": 1 },
      "minecraft:oak_fence": { "protocol_id": 2 },
      "minecraft:stone_slab": { "protocol_id": 3 },
      "minecraft:glass": { "protocol_id": 4 }
    }
  }
}"#,
    )
    .unwrap();
    let oracle_path = dir.join("block_shapes.json");
    std::fs::write(&oracle_path, oracle).unwrap();
    let data = GameData::load_from(&reports, Some(&oracle_path)).unwrap();
    FaceOccluders::from_shapes(&data.blocks, &data.shapes)
}

/// The fence's occlusion shape (the post) reaches no side plane and not the ceiling, so
/// vanilla shows every fence face against another fence — where the model rule (the
/// neighbour's arm quads covering the shared plane) hid them. Against stone the face is
/// hidden; the post on the floor still hides nothing below it (the neighbour's up-face
/// shape is empty there).
#[test]
fn fence_faces_show_against_fences_and_hide_against_stone() {
    let t = occluders(ORACLE_JSON);
    let fence = BlockStateId(2);
    let stone = BlockStateId(1);
    for dir in [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        assert_eq!(
            t.should_render_face(fence, fence, dir),
            Some(true),
            "fence against fence shows {dir:?}"
        );
        assert_eq!(
            t.should_render_face(fence, stone, dir),
            Some(false),
            "stone hides the fence's {dir:?} face"
        );
    }
    // The post reaches the floor: a stone slab below it (its up-face shape covers the
    // full cell only for a full cube) — a bottom slab's up face touches nothing, so the
    // fence's bottom face shows above it.
    assert_eq!(
        t.should_render_face(fence, BlockStateId(3), Direction::Down),
        Some(true)
    );
}

/// Slab faces follow the occlusion shapes: two bottom slabs hide their mutual side faces
/// (each covers the lower half of the shared plane), a bottom and a top slab show theirs
/// (disjoint halves), a full-block neighbour hides even a face whose own shape is empty
/// (vanilla checks `occluder == Shapes.block()` first — a slab's top face under stone),
/// and a slab sitting on stone hides the stone's top face.
#[test]
fn slab_faces_follow_the_occlusion_shapes() {
    let t = occluders(ORACLE_JSON);
    let stone = BlockStateId(1);
    let bottom = BlockStateId(3);
    let top = BlockStateId(4);
    let glass = BlockStateId(5);
    for dir in [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        assert_eq!(
            t.should_render_face(bottom, bottom, dir),
            Some(false),
            "two bottom slabs hide the shared {dir:?} face"
        );
        assert_eq!(
            t.should_render_face(top, top, dir),
            Some(false),
            "two top slabs hide the shared {dir:?} face"
        );
        // Disjoint halves on the plane: both render.
        assert_eq!(
            t.should_render_face(bottom, top, dir),
            Some(true),
            "bottom against top slab shows the {dir:?} face"
        );
        assert_eq!(
            t.should_render_face(top, bottom, dir),
            Some(true),
            "top against bottom slab shows the {dir:?} face"
        );
    }
    // The slab's own top face does not reach the plane, but a full-block neighbour hides
    // it anyway (the block gate runs first).
    assert_eq!(
        t.should_render_face(bottom, stone, Direction::Up),
        Some(false),
        "stone above hides the slab's top face"
    );
    // A bottom slab on stone hides the stone's top face: the slab's down-face shape is
    // the full plane (its box spans the whole cell footprint).
    assert_eq!(
        t.should_render_face(stone, bottom, Direction::Up),
        Some(false),
        "a slab above hides the stone's top face"
    );
    // Below stone, a bottom slab leaves the stone's bottom face visible: the slab's
    // up-face shape is empty (it does not reach the shared plane).
    assert_eq!(
        t.should_render_face(stone, bottom, Direction::Down),
        Some(true),
        "a slab below shows the stone's bottom face"
    );
    // Glass occludes nothing.
    assert_eq!(
        t.should_render_face(bottom, glass, Direction::North),
        Some(true)
    );
}

/// Dumps from before the occlusion field, ids past the table and the absent table answer
/// `None`: the mesher's model rule decides.
#[test]
fn missing_occlusion_data_falls_back_to_the_model_rule() {
    let old_oracle = r#"{
  "version": 1,
  "states": [
    { "id": 1, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "opaque": true }
  ]
}"#;
    let t = occluders(old_oracle);
    let stone = BlockStateId(1);
    assert_eq!(t.should_render_face(stone, stone, Direction::Up), None);
    // Ids past the table (modded servers) never panic and never decide.
    assert_eq!(
        t.should_render_face(BlockStateId(99), stone, Direction::Up),
        None
    );
    assert_eq!(
        t.should_render_face(stone, BlockStateId(99), Direction::Up),
        None
    );
    for table in [FaceOccluders::absent(), FaceOccluders::default()] {
        assert_eq!(
            table.should_render_face(stone, stone, Direction::Up),
            None,
            "the absent table answers None"
        );
    }
}
