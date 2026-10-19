use super::*;
use crate::atlas::{AtlasRegion, AtlasRegions};
use haste_core::view::BlockStateTable as _;

/// One sprite transparency class: fully opaque, cutout (alpha 0/255 only) or
/// translucent (partial alpha), as vanilla's `NativeImage.computeTransparency`
/// reports them.
#[derive(Clone, Copy)]
struct TestRegions {
    opaque: bool,
    translucent: bool,
}

impl AtlasRegions for TestRegions {
    fn region_of(&self, _texture_id: &str) -> AtlasRegion {
        AtlasRegion {
            pixel_rect: [0, 0, 16, 16],
            sprite: 1,
            opaque: self.opaque,
            translucent: self.translucent,
        }
    }
}

fn quad() -> BakedQuad {
    BakedQuad::from_element(Direction::Up, [0.0; 3], [1.0; 3], None, None)
}

#[test]
fn classify_layer_by_block_name_and_alpha() {
    let opaque = TestRegions {
        opaque: true,
        translucent: false,
    };
    let cutout = TestRegions {
        opaque: false,
        translucent: false,
    };
    let translucent = TestRegions {
        opaque: false,
        translucent: true,
    };
    let q = quad();
    assert_eq!(
        classify_layer(
            std::slice::from_ref(&q),
            &Identifier::minecraft("stone"),
            &opaque
        ),
        RenderLayer::Solid
    );
    assert_eq!(
        classify_layer(
            std::slice::from_ref(&q),
            &Identifier::minecraft("glass"),
            &opaque
        ),
        RenderLayer::Translucent
    );
    assert_eq!(
        classify_layer(
            std::slice::from_ref(&q),
            &Identifier::minecraft("blue_stained_glass"),
            &opaque
        ),
        RenderLayer::Translucent
    );
    assert_eq!(
        classify_layer(
            std::slice::from_ref(&q),
            &Identifier::minecraft("oak_leaves"),
            &opaque
        ),
        RenderLayer::CutoutMipped
    );
    assert_eq!(
        classify_layer(
            std::slice::from_ref(&q),
            &Identifier::minecraft("oak_sapling"),
            &cutout
        ),
        RenderLayer::Cutout
    );
    // A sprite with partial-alpha texels is translucent by vanilla's per-sprite
    // scan, whatever the block is named.
    assert_eq!(
        classify_layer(
            std::slice::from_ref(&q),
            &Identifier::minecraft("oak_sapling"),
            &translucent
        ),
        RenderLayer::Translucent
    );
}

/// A cube with all six faces, written to the JSON in a scrambled order (the file's
/// key order is irrelevant: baking must always produce vanilla's face order).
const CUBE_MODEL: &str = r##"{
    "textures": {"all": "minecraft:block/stone"},
    "elements": [{
        "from": [0, 0, 0],
        "to": [16, 16, 16],
        "faces": {
            "east": {"texture": "#all"},
            "north": {"texture": "#all"},
            "west": {"texture": "#all", "rotation": 90},
            "south": {"texture": "#all"},
            "up": {"texture": "#all"},
            "down": {"texture": "#all"}
        }
    }]
}"##;

/// Fresh parse -> resolved model -> baked quads, the path `BakedBlockModels::bake`
/// runs per model.
fn bake_cube() -> Vec<BakedQuad> {
    let file: crate::model::ModelFile = serde_json::from_str(CUBE_MODEL).unwrap();
    let mut textures = HashMap::new();
    textures.insert("all".to_string(), "minecraft:block/stone".to_string());
    let resolved = ResolvedModel {
        textures,
        force_translucent: Default::default(),
        elements: file.elements,
        ambientocclusion: true,
        gui_light: false,
        gui_display: None,
        ground_display: None,
        firstperson_display: None,
        thirdperson_display: None,
        thirdperson_left_display: None,
        head_display: None,
        on_shelf_display: None,
        fixed_display: None,
    };
    let mut resolved_models = HashMap::new();
    resolved_models.insert("minecraft:block/cube".to_string(), resolved);
    let refs = [ModelRef {
        model: "minecraft:block/cube".to_string(),
        x: 0,
        y: 0,
        uvlock: false,
    }];
    bake_refs(&refs, &resolved_models).unwrap()
}

#[test]
fn baked_quads_identical_across_fresh_parses() {
    let first = bake_cube();
    let second = bake_cube();
    assert_eq!(first.len(), 6, "one quad per face");
    // Same order, not just the same set: `ElementDef.faces` was a HashMap, so the
    // quad order (and the mesher's vertex order) differed per process.
    assert_eq!(first, second);
}

/// A modded server can send ids past the end of the table: every accessor resolves
/// them to the placeholder state's entry (stone: one quad, full opaque, solid layer).
#[test]
fn out_of_range_ids_resolve_to_the_placeholder() {
    let quad = BakedQuad::from_element(Direction::Up, [0.0; 3], [1.0; 3], None, None);
    let baked = BakedBlockModels {
        quads: vec![None, Some(vec![quad.clone()].into())],
        full_opaque: vec![false, true],
        layer: vec![RenderLayer::Solid, RenderLayer::Solid],
        placeholder: BlockStateId(1),
    };
    assert!(baked.quads(BlockStateId(0)).is_empty());
    assert_eq!(baked.quads(BlockStateId(1)), std::slice::from_ref(&quad));
    for id in [2u32, 100, u32::MAX] {
        let s = BlockStateId(id);
        assert_eq!(baked.quads(s), std::slice::from_ref(&quad));
        assert!(baked.is_full_opaque(s));
        assert_eq!(baked.render_layer(s), RenderLayer::Solid);
    }
}

#[test]
fn baked_faces_come_in_vanilla_order() {
    let quads = bake_cube();
    let dirs: Vec<Direction> = quads.iter().map(|q| q.direction).collect();
    assert_eq!(dirs, Direction::ALL, "down, up, north, south, west, east");
    let file: crate::model::ModelFile = serde_json::from_str(CUBE_MODEL).unwrap();
    let faces: Vec<Direction> = file.elements[0].faces.keys().copied().collect();
    assert_eq!(faces, Direction::ALL.to_vec());
    // Unknown face names are skipped, not parsed into geometry.
    let partial: crate::model::ModelFile = serde_json::from_str(
        r##"{"elements":[{"from":[0,0,0],"to":[16,16,16],
             "faces":{"up":{"texture":"#all"},"inside":{"texture":"#all"}}}]}"##,
    )
    .unwrap();
    assert_eq!(partial.elements[0].faces.len(), 1);
}

#[test]
fn full_opaque_requires_six_full_faces() {
    let opaque = TestRegions {
        opaque: true,
        translucent: false,
    };
    let quads: Vec<BakedQuad> = Direction::ALL
        .iter()
        .copied()
        .map(|d| BakedQuad::from_element(d, [0.0; 3], [1.0; 3], None, None))
        .map(|mut q| {
            q.cullface = Some(q.direction);
            q.texture = "t".into();
            q
        })
        .collect();
    assert!(is_full_opaque(&quads, &opaque));
    let mut partial = quads.clone();
    partial[0].positions[0][1] = 0.5;
    assert!(!is_full_opaque(&partial, &opaque));
    // transparent texture -> not full opaque
    let transparent = TestRegions {
        opaque: false,
        translucent: false,
    };
    assert!(!is_full_opaque(&quads, &transparent));
    // 5 faces -> not full opaque
    assert!(!is_full_opaque(&quads[..5], &opaque));
}

#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn jar_bake_stone_and_stairs() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3").unwrap();
    let reports = haste_core::paths::datagen_reports();
    let states = BlocksJsonStates::load(&reports.join("blocks.json")).unwrap();
    let started = std::time::Instant::now();
    let mut atlas = AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let baked = BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
    let bake_time = started.elapsed();
    eprintln!(
        "bake: {bake_time:?}, atlas {}x{}, {} states",
        atlas.size(),
        atlas.size(),
        states.len()
    );

    // stone: state id 1, 6 quads, full opaque
    let stone = BlockStateId(1);
    assert_eq!(
        baked.quads(stone).len(),
        6,
        "minecraft:stone should have 6 quads"
    );
    assert!(baked.is_full_opaque(stone));

    // Every baked quad samples a real sprite: none falls back to the missing texture
    // (states without a model bake to no quads and are not checked).
    let missing = atlas.missing_region();
    let mut bad = Vec::new();
    for id in 0..states.len() as u32 {
        let sid = BlockStateId(id);
        for q in baked.quads(sid) {
            if atlas.region_of(&q.texture) == missing {
                bad.push(format!("{} -> {}", states.block_of(sid), q.texture));
                break;
            }
        }
    }
    bad.dedup();
    assert!(
        bad.is_empty(),
        "{} states use the missing sprite: {:?}",
        bad.len(),
        &bad[..bad.len().min(20)]
    );

    // grass_block (snowy=false): full opaque despite its side overlay quads, so faces
    // between grass blocks are culled.
    let grass = (0..states.len() as u32)
        .map(BlockStateId)
        .find(|s| states.block_of(*s).path() == "grass_block")
        .unwrap();
    assert!(baked.is_full_opaque(grass), "grass_block full opaque");

    // The cross plants' quads are lit as "up" (`shade_direction_override`
    // in tinted_cross): their shade face is Up, not the quad's own facing.
    for name in ["short_grass", "tall_grass", "poppy"] {
        let plant = (0..states.len() as u32)
            .map(BlockStateId)
            .find(|s| states.block_of(*s).path() == name)
            .unwrap();
        let quads = baked.quads(plant);
        assert!(!quads.is_empty(), "{name} has no quads");
        for q in quads {
            assert_eq!(
                q.shade_face,
                Some(crate::model::Direction::Up),
                "{name} quad must light as up"
            );
        }
    }

    // oak_stairs facing=east (bottom, straight): > 6 quads
    let mut stairs_east = None;
    for id in 0..states.len() as u32 {
        let sid = BlockStateId(id);
        if states.block_of(sid).path() == "oak_stairs"
            && states
                .properties(sid)
                .iter()
                .any(|(k, v)| k == "facing" && v == "east")
            && states
                .properties(sid)
                .iter()
                .any(|(k, v)| k == "half" && v == "bottom")
            && states
                .properties(sid)
                .iter()
                .any(|(k, v)| k == "shape" && v == "straight")
        {
            stairs_east = Some(sid);
            break;
        }
    }
    let stairs = stairs_east.expect("oak_stairs facing=east state exists");
    assert!(
        baked.quads(stairs).len() > 6,
        "oak_stairs facing=east should have more than 6 quads, got {}",
        baked.quads(stairs).len()
    );
    assert!(!baked.is_full_opaque(stairs));

    // glass is translucent; stone is solid
    let glass = find_state(&states, "glass");
    assert_eq!(baked.render_layer(glass), RenderLayer::Translucent);
    assert_eq!(baked.render_layer(stone), RenderLayer::Solid);
    let leaves = find_state(&states, "oak_leaves");
    assert_eq!(baked.render_layer(leaves), RenderLayer::CutoutMipped);

    // The solid layer must only ever hold fully opaque sprites: vanilla picks the
    // layer per sprite from its texel scan (NativeImage.computeTransparency ->
    // ChunkSectionLayer.byTransparency: alpha 0 -> CUTOUT, partial -> TRANSLUCENT,
    // all 255 -> SOLID), and our classify_layer's alpha fallback is what enforces
    // it here. These are the blocks whose sprites are not fully opaque: anything
    // landing in Solid with transparent texels would render them (no discard).
    let cutout_expect = [
        ("grass_block", RenderLayer::Cutout), // side overlay sprite has alpha 0
        // The glass pane's sprites (glass_pane_top, glass) have partial-alpha
        // texels, so vanilla's per-sprite scan puts it in translucent; ours now
        // does too via the region's translucent flag.
        ("glass_pane", RenderLayer::Translucent),
        ("iron_bars", RenderLayer::Cutout),
        ("tall_grass", RenderLayer::Cutout),
        ("poppy", RenderLayer::Cutout),
        ("vine", RenderLayer::CutoutMipped),
    ];
    for (name, want) in cutout_expect {
        // grass_block's snowy=true state uses the fully-opaque snow sprite; the
        // overlay (with its alpha-0 texels) is on snowy=false.
        let s = if name == "grass_block" {
            (0..states.len() as u32)
                .map(BlockStateId)
                .find(|s| {
                    states.block_of(*s).path() == "grass_block"
                        && states
                            .property_value(*s, "snowy")
                            .is_some_and(|v| v == "false")
                })
                .expect("grass_block snowy=false state")
        } else {
            find_state(&states, name)
        };
        assert_eq!(
            baked.render_layer(s),
            want,
            "{name} must not land in the solid layer"
        );
    }
    // And no solid-layer state may use a sprite with a transparent texel.
    let mut solid_with_holes = Vec::new();
    for id in 0..states.len() as u32 {
        let s = BlockStateId(id);
        if baked.render_layer(s) == RenderLayer::Solid {
            for q in baked.quads(s) {
                if !atlas.region(&q.texture).opaque {
                    solid_with_holes.push(format!("{} -> {}", states.block_of(s), q.texture));
                    break;
                }
            }
        }
    }
    assert!(
        solid_with_holes.is_empty(),
        "{} solid-layer states use non-opaque sprites: {:?}",
        solid_with_holes.len(),
        &solid_with_holes[..solid_with_holes.len().min(20)]
    );
    // The grass block's side overlay sprite must be non-opaque in the atlas:
    // it is what keeps grass_block out of the solid layer.
    let overlay = atlas.region("minecraft:block/grass_block_side_overlay");
    assert!(
        !overlay.opaque,
        "grass_block_side_overlay must not be opaque"
    );
}

fn find_state(states: &BlocksJsonStates, block: &str) -> BlockStateId {
    for id in 0..states.len() as u32 {
        let sid = BlockStateId(id);
        if states.block_of(sid).path() == block {
            return sid;
        }
    }
    panic!("state for {block} not found");
}
