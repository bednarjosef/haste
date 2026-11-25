//! Tests for `BlockShapes`: placeholder resolution, oracle parsing, the AO solid rule,
//! render shape and suffocation flags, and the occlusion/light-dampening fields.

use super::*;
use haste_core::view::CollisionShapes as _;

/// air = 0, stone = 1 (the placeholder): the fallback gives stone a solid full cube.
const BLOCKS_JSON: &str = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] }
}"#;

fn fallback_shapes() -> BlockShapes {
    let blocks = BlockStates::parse(BLOCKS_JSON, None).unwrap();
    BlockShapes::fallback(&blocks)
}

#[test]
fn out_of_range_ids_use_the_placeholder_entry() {
    let s = fallback_shapes();
    let stone = BlockStateId(1);
    assert!(s.is_opaque_full(stone));
    assert!(!s.is_air(stone));
    assert_eq!(s.collision_boxes(stone).len(), 1);
    for id in [2u32, 100, u32::MAX] {
        let unknown = BlockStateId(id);
        assert_eq!(s.light_emission(unknown), s.light_emission(stone));
        assert_eq!(s.is_air(unknown), s.is_air(stone));
        assert_eq!(s.is_opaque_full(unknown), s.is_opaque_full(stone));
        assert_eq!(s.fluid(unknown), s.fluid(stone));
        assert_eq!(s.collision_boxes(unknown), s.collision_boxes(stone));
        assert_eq!(s.outline_boxes(unknown), s.outline_boxes(stone));
    }
}

#[test]
fn oracle_backed_shapes_are_total_too() {
    let blocks = BlockStates::parse(BLOCKS_JSON, None).unwrap();
    let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 0, "air": true, "collision": [], "outline": [], "light": 0, "opaque": false },
    { "id": 1, "air": false, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": true }
  ]
}"#;
    let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
    assert!(s.is_oracle_backed());
    let stone = blocks.placeholder_state();
    for id in [2u32, u32::MAX] {
        let unknown = BlockStateId(id);
        assert_eq!(s.collision_boxes(unknown), s.collision_boxes(stone));
        assert_eq!(s.outline_boxes(unknown), s.outline_boxes(stone));
        assert_eq!(s.is_opaque_full(unknown), s.is_opaque_full(stone));
        assert_eq!(s.is_air(unknown), s.is_air(stone));
        assert_eq!(s.light_emission(unknown), s.light_emission(stone));
        assert_eq!(s.fluid(unknown), s.fluid(stone));
    }
}

/// The AO occluder rule (vanilla `BlockBehaviour.getShadeBrightness`, read
/// from the 26.3 source): a full collision cube darkens (0.2) unless the
/// block's class overrides the brightness to 1.0 — glass
/// (`TransparentBlock`) keeps a full collision cube but stays bright. A
/// slab (half cube) and leaves' neighbors in the model sense stay bright
/// only when the shape is not full: leaves ARE a full collision cube, so
/// they darken like stone in vanilla.
#[test]
fn ao_solid_follows_the_collision_shape_not_render_opacity() {
    // 0 air, 1 stone, 2 glass, 3 oak_leaves, 4 bottom slab, 5 stained glass,
    // 6 snow (2 layers), 7 snow (8 layers).
    let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
  "minecraft:oak_leaves": { "states": [ { "id": 3, "default": true } ] },
  "minecraft:stone_slab": { "states": [ { "id": 4, "default": true } ] },
  "minecraft:orange_stained_glass": { "states": [ { "id": 5, "default": true } ] },
  "minecraft:snow": {
    "properties": { "layers": ["2", "8"] },
    "states": [
      { "id": 6, "default": true, "properties": { "layers": "2" } },
      { "id": 7, "properties": { "layers": "8" } }
    ]
  }
}"#;
    let blocks = BlockStates::parse(blocks_json, None).unwrap();
    let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 0, "air": true, "collision": [], "outline": [], "light": 0, "opaque": false },
    { "id": 1, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": true },
    { "id": 2, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": false },
    { "id": 3, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": false },
    { "id": 4, "collision": [[0,0,0,1,0.5,1]], "outline": [[0,0,0,1,0.5,1]], "light": 0, "opaque": false },
    { "id": 5, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": false },
    { "id": 6, "collision": [[0,0,0,1,0.25,1]], "outline": [[0,0,0,1,0.25,1]], "light": 0, "opaque": false },
    { "id": 7, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "light": 0, "opaque": true }
  ]
}"#;
    let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
    // Full collision cube, no class override: darkens.
    assert!(s.is_ao_solid(BlockStateId(1)), "stone darkens");
    // Full cube but glass (TransparentBlock): stays bright.
    assert!(!s.is_ao_solid(BlockStateId(2)), "glass stays bright");
    assert!(
        !s.is_ao_solid(BlockStateId(5)),
        "stained glass stays bright"
    );
    // Leaves: full collision cube, no override — vanilla darkens them.
    assert!(s.is_ao_solid(BlockStateId(3)), "leaves darken (full cube)");
    // Half cube: bright.
    assert!(!s.is_ao_solid(BlockStateId(4)), "slab stays bright");
    // Snow: 0.2 only at layers = 8 (the full-cube state).
    assert!(!s.is_ao_solid(BlockStateId(6)), "thin snow stays bright");
    assert!(s.is_ao_solid(BlockStateId(7)), "full snow darkens");
}

/// render_shape/suffocating from the oracle dump's per-state fields (the recorder mod's
/// `stateJson`): vanilla `getRenderShape()` and `isSuffocating(EmptyBlockGetter, origin)`.
/// Unknown ids answer false for both (no shadow skip, no camera push-out), unlike the shape
/// lookups, which resolve to the placeholder.
#[test]
fn render_shape_and_suffocation_flags_from_the_oracle() {
    // 0 air, 1 stone, 2 glass, 3 barrier, 4 bottom slab.
    let blocks_json = r#"{
          "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
          "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
          "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
          "minecraft:barrier": { "states": [ { "id": 3, "default": true } ] },
          "minecraft:stone_slab": { "states": [ { "id": 4, "default": true } ] }
        }"#;
    let blocks = BlockStates::parse(blocks_json, None).unwrap();
    let oracle = r#"{
          "version": 1,
          "states": [
            { "id": 0, "air": true, "render_shape": "invisible", "suffocating": false },
            { "id": 1, "collision": [[0,0,0,1,1,1]], "render_shape": "model", "suffocating": true },
            { "id": 2, "collision": [[0,0,0,1,1,1]], "render_shape": "model", "suffocating": false },
            { "id": 3, "render_shape": "invisible", "suffocating": false },
            { "id": 4, "collision": [[0,0,0,1,0.5,1]], "render_shape": "model", "suffocating": false }
          ]
        }"#;
    let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
    assert!(
        !s.render_shape_invisible(BlockStateId(1)),
        "stone is a model"
    );
    assert!(
        s.render_shape_invisible(BlockStateId(0)),
        "air is invisible"
    );
    assert!(
        s.render_shape_invisible(BlockStateId(3)),
        "barrier is invisible"
    );
    assert!(
        !s.render_shape_invisible(BlockStateId(2)),
        "glass is a model"
    );
    assert!(s.is_suffocating(BlockStateId(1)), "stone suffocates");
    assert!(
        !s.is_suffocating(BlockStateId(2)),
        "glass does not suffocate"
    );
    assert!(
        !s.is_suffocating(BlockStateId(4)),
        "half slab does not suffocate"
    );
    assert!(!s.is_suffocating(BlockStateId(0)), "air does not suffocate");
    // Unknown ids: false for both, no panic.
    for id in [5u32, 100, u32::MAX] {
        assert!(!s.render_shape_invisible(BlockStateId(id)));
        assert!(!s.is_suffocating(BlockStateId(id)));
    }
}

/// Dumps from before the render_shape/suffocating regeneration: render_shape defaults to
/// "model" everywhere; suffocating falls back to the full-collision-cube rule (the shape
/// half of vanilla's default predicate — glass diverges until the dump is regenerated).
#[test]
fn old_dumps_default_to_model_and_the_full_cube_rule() {
    // 0 air, 1 stone, 2 glass, 3 bottom slab.
    let blocks_json = r#"{
          "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
          "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
          "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
          "minecraft:stone_slab": { "states": [ { "id": 3, "default": true } ] }
        }"#;
    let blocks = BlockStates::parse(blocks_json, None).unwrap();
    let oracle = r#"{
          "version": 1,
          "states": [
            { "id": 1, "collision": [[0,0,0,1,1,1]] },
            { "id": 2, "collision": [[0,0,0,1,1,1]] },
            { "id": 3, "collision": [[0,0,0,1,0.5,1]] }
          ]
        }"#;
    let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
    assert!(
        !s.render_shape_invisible(BlockStateId(1)),
        "stone stays a model"
    );
    assert!(s.is_suffocating(BlockStateId(1)), "full cube suffocates");
    assert!(
        s.is_suffocating(BlockStateId(2)),
        "full-cube fallback (glass diverges)"
    );
    assert!(!s.is_suffocating(BlockStateId(3)), "half cube does not");
}

/// Fallback table (no oracle dump): air and fluids render no model (vanilla
/// `AirBlock`/`LiquidBlock` are RenderShape.INVISIBLE); suffocation uses the full-cube rule.
#[test]
fn fallback_render_shape_and_suffocation() {
    let s = fallback_shapes();
    let stone = BlockStateId(1);
    assert!(!s.render_shape_invisible(stone), "stone is a model");
    assert!(s.is_suffocating(stone), "stone suffocates (full cube)");
    for id in [2u32, 100, u32::MAX] {
        assert!(!s.render_shape_invisible(BlockStateId(id)));
        assert!(!s.is_suffocating(BlockStateId(id)));
    }
    // Air and fluids: no model (the fallback knows their paths directly).
    let blocks_json = r#"{
          "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
          "minecraft:water": { "states": [ { "id": 1, "default": true } ] },
          "minecraft:lava": { "states": [ { "id": 2, "default": true } ] }
        }"#;
    let blocks = BlockStates::parse(blocks_json, None).unwrap();
    let f = BlockShapes::fallback(&blocks);
    for id in [0u32, 1, 2] {
        assert!(f.render_shape_invisible(BlockStateId(id)), "air/fluid {id}");
        assert!(!f.is_suffocating(BlockStateId(id)), "no collision");
    }
}

/// The 2026-10-18 dump fields: `occlusion` (vanilla `getOcclusionShape`), `light_dampening`
/// (`getLightDampening`, the cached value with class overrides in) and
/// `propagates_skylight_down` (`propagatesSkylightDown`). Values from the real 26.3 server:
/// stone is a full opaque cube, glass occludes nothing and passes skylight, tinted glass
/// blocks light entirely, leaves and water dampen by 1, a waterlogged slab dampens by 1
/// (its fluid state stops skylight) and a dry one by 0.
#[test]
fn occlusion_light_dampening_and_skylight_from_the_oracle() {
    // 0 air, 1 stone, 2 glass, 3 tinted_glass, 4 water, 5 oak_leaves,
    // 6 stone_slab waterlogged, 7 stone_slab dry.
    let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:glass": { "states": [ { "id": 2, "default": true } ] },
  "minecraft:tinted_glass": { "states": [ { "id": 3, "default": true } ] },
  "minecraft:water": { "states": [ { "id": 4, "default": true } ] },
  "minecraft:oak_leaves": { "states": [ { "id": 5, "default": true } ] },
  "minecraft:stone_slab": {
    "properties": { "waterlogged": ["true", "false"] },
    "states": [
      { "id": 6, "default": true, "properties": { "waterlogged": "true" } },
      { "id": 7, "properties": { "waterlogged": "false" } }
    ]
  }
}"#;
    let blocks = BlockStates::parse(blocks_json, None).unwrap();
    let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 0, "occlusion": [], "light_dampening": 0, "propagates_skylight_down": true },
    { "id": 1, "occlusion": [[0,0,0,1,1,1]], "light_dampening": 15, "propagates_skylight_down": false },
    { "id": 2, "occlusion": [], "light_dampening": 0, "propagates_skylight_down": true },
    { "id": 3, "occlusion": [], "light_dampening": 15, "propagates_skylight_down": false },
    { "id": 4, "occlusion": [], "light_dampening": 1, "propagates_skylight_down": false },
    { "id": 5, "occlusion": [], "light_dampening": 1, "propagates_skylight_down": true },
    { "id": 6, "occlusion": [[0,0,0,1,0.5,1]], "light_dampening": 1, "propagates_skylight_down": false },
    { "id": 7, "occlusion": [[0,0,0,1,0.5,1]], "light_dampening": 0, "propagates_skylight_down": true }
  ]
}"#;
    let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
    // Occlusion shapes: full cube for stone, empty for glass and leaves, half box for slabs.
    let full = [FULL_CUBE];
    assert_eq!(s.occlusion_boxes(BlockStateId(1)), Some(&full[..]));
    assert_eq!(s.occlusion_boxes(BlockStateId(2)), Some(&[][..]));
    assert_eq!(s.occlusion_boxes(BlockStateId(5)), Some(&[][..]));
    let slab = [Aabb {
        min: DVec3::new(0.0, 0.0, 0.0),
        max: DVec3::new(1.0, 0.5, 1.0),
    }];
    assert_eq!(s.occlusion_boxes(BlockStateId(6)), Some(&slab[..]));
    assert_eq!(s.occlusion_boxes(BlockStateId(7)), Some(&slab[..]));
    assert_eq!(
        s.occlusion_boxes(BlockStateId(4)),
        Some(&[][..]),
        "water occludes nothing"
    );
    // Light dampening: the dump's value, straight.
    assert_eq!(s.light_dampening(BlockStateId(1)), Some(15), "stone");
    assert_eq!(s.light_dampening(BlockStateId(2)), Some(0), "glass");
    assert_eq!(s.light_dampening(BlockStateId(3)), Some(15), "tinted glass");
    assert_eq!(s.light_dampening(BlockStateId(4)), Some(1), "water");
    assert_eq!(s.light_dampening(BlockStateId(5)), Some(1), "leaves");
    assert_eq!(
        s.light_dampening(BlockStateId(6)),
        Some(1),
        "waterlogged slab"
    );
    assert_eq!(s.light_dampening(BlockStateId(7)), Some(0), "dry slab");
    // Skylight down: glass and dry slabs pass it, stone and water do not.
    assert_eq!(s.propagates_skylight_down(BlockStateId(2)), Some(true));
    assert_eq!(s.propagates_skylight_down(BlockStateId(7)), Some(true));
    assert_eq!(s.propagates_skylight_down(BlockStateId(1)), Some(false));
    assert_eq!(s.propagates_skylight_down(BlockStateId(4)), Some(false));
    // Unknown ids resolve to the placeholder (stone), never a panic.
    for id in [8u32, 100, u32::MAX] {
        let unknown = BlockStateId(id);
        assert_eq!(
            s.occlusion_boxes(unknown),
            s.occlusion_boxes(BlockStateId(1))
        );
        assert_eq!(s.light_dampening(unknown), Some(15));
        assert_eq!(s.propagates_skylight_down(unknown), Some(false));
    }
}

/// Dumps from before the occlusion regeneration: all three accessors answer `None`, and the
/// boxes are interned without a per-state Vec (all slabs share one arena entry).
#[test]
fn old_dumps_answer_none_for_the_occlusion_fields() {
    let blocks_json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:stone_slab": { "states": [ { "id": 2, "default": true } ] }
}"#;
    let blocks = BlockStates::parse(blocks_json, None).unwrap();
    let oracle = r#"{
  "version": 1,
  "states": [
    { "id": 1, "collision": [[0,0,0,1,1,1]], "outline": [[0,0,0,1,1,1]], "opaque": true },
    { "id": 2, "collision": [[0,0,0,1,0.5,1]], "outline": [[0,0,0,1,0.5,1]] }
  ]
}"#;
    let s = BlockShapes::parse_oracle(oracle, &blocks).unwrap();
    for id in [0u32, 1, 2, 100] {
        let state = BlockStateId(id);
        assert_eq!(s.occlusion_boxes(state), None, "state {id}");
        assert_eq!(s.light_dampening(state), None, "state {id}");
        assert_eq!(s.propagates_skylight_down(state), None, "state {id}");
    }
    // The fallback table (no oracle dump at all) answers None likewise.
    let f = BlockShapes::fallback(&blocks);
    for id in [0u32, 1, 2] {
        let state = BlockStateId(id);
        assert_eq!(f.occlusion_boxes(state), None);
        assert_eq!(f.light_dampening(state), None);
        assert_eq!(f.propagates_skylight_down(state), None);
    }
}
