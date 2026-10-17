use super::*;
// ---------------------------------------------------------------------------
// Real mob models from the recorder dump.
// ---------------------------------------------------------------------------

/// A minimal dump-shaped JSON: one layer with a nested part, non-trivial pivots/rotations,
/// mirror + inflate on the cube - small enough to check every converted field.
const DUMP_SAMPLE: &str = r#"{
  "version": 1,
  "minecraft": "26.3",
  "layers": [
    {
      "layer": "minecraft:pig#main",
      "texture_width": 64,
      "texture_height": 64,
      "parts": [
        {
          "name": "head",
          "pivot": [0.0, 12.0, -6.0],
          "rotation": [0.1, 0.2, 0.3],
          "scale": [1.0, 1.0, 1.0],
          "cubes": [
            {
              "origin": [-4.0, -4.0, -8.0],
              "size": [8.0, 8.0, 8.0],
              "uv": "0 0",
              "u": 0.0,
              "v": 0.0,
              "mirror": true,
              "inflate": [0.25, 0.25, 0.25],
              "visible_faces": ["up", "down", "north", "south", "west", "east"]
            }
          ],
          "children": [
            {
              "name": "snout",
              "pivot": [0.0, 16.0, -14.0],
              "rotation": [0.0, 0.0, 0.0],
              "scale": [1.0, 1.0, 1.0],
              "cubes": [
                {
                  "origin": [-2.0, 0.0, -3.0],
                  "size": [4.0, 3.0, 1.0],
                  "uv": "16 16",
                  "u": 16.0,
                  "v": 16.0,
                  "mirror": false,
                  "inflate": [0.0, 0.0, 0.0],
                  "visible_faces": ["up", "down", "north", "south", "west", "east"]
                }
              ],
              "children": []
            }
          ]
        },
        {
          "name": "body",
          "pivot": [0.0, 12.0, 2.0],
          "rotation": [1.5707964, 0.0, 0.0],
          "scale": [1.0, 1.0, 1.0],
          "cubes": [
            {
              "origin": [-5.0, -10.0, -7.0],
              "size": [10.0, 16.0, 8.0],
              "uv": "28 8",
              "u": 28.0,
              "v": 8.0,
              "mirror": false,
              "inflate": [0.0, 0.0, 0.0],
              "visible_faces": ["up", "down", "north", "south", "west", "east"]
            }
          ],
          "children": []
        }
      ]
    }
  ]
}"#;

#[test]
fn dump_loader_converts_nested_parts_to_feet_at_zero() {
    let lib = crate::model_dump::ModelLibrary::parse(DUMP_SAMPLE.as_bytes()).unwrap();
    assert_eq!(lib.len(), 1);
    let model = lib.get("minecraft:pig#main", 1.0).unwrap();
    assert_eq!(model.tex_width, 64);
    assert_eq!(model.tex_height, 64);
    // Ground shift: dump pivots were y = 12/16 (feet at dump y = 24) -> haste y = -12/-8.
    let head = find_part(&model.root, "head").unwrap();
    assert!((head.pivot.y - (12.0 - 24.0)).abs() < 1e-6);
    assert!((head.pivot.x - 0.0).abs() < 1e-6 && (head.pivot.z - -6.0).abs() < 1e-6);
    // Rotation arrives in radians, kept as-is.
    assert!((head.rotation.x - 0.1).abs() < 1e-6);
    assert!((head.rotation.y - 0.2).abs() < 1e-6);
    assert!((head.rotation.z - 0.3).abs() < 1e-6);
    // The nested child hangs off the head with its own parent-relative pivot, NOT shifted:
    // a child's PartPose is relative to its parent, not to the ground plane.
    let snout = find_part(&model.root, "snout").unwrap();
    assert!((snout.pivot.y - 16.0).abs() < 1e-6);
    assert!((snout.pivot.z - -14.0).abs() < 1e-6);
    assert_eq!(snout.cubes.len(), 1);
    assert_eq!(snout.cubes[0].uv, [16, 16]);
    assert!((snout.cubes[0].size.y - 3.0).abs() < 1e-6);
    // Cube fields survive: mirror, inflate, origin, size.
    assert!(head.cubes[0].mirror);
    assert!((head.cubes[0].inflate - 0.25).abs() < 1e-6);
    assert!((head.cubes[0].origin.z - -8.0).abs() < 1e-6);
    assert!((head.cubes[0].size.x - 8.0).abs() < 1e-6);
    // Body rotation (pi/2 for quadruped bodies) is preserved.
    let body = find_part(&model.root, "body").unwrap();
    assert!((body.rotation.x - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    // Unknown layer and bad version are handled without panicking.
    assert!(lib.get("minecraft:cow#main", 1.0).is_none());
    let bad = DUMP_SAMPLE.replace("\"version\": 1", "\"version\": 2");
    assert!(crate::model_dump::ModelLibrary::parse(bad.as_bytes()).is_err());
}

#[test]
fn dump_loader_applies_root_scale() {
    let lib = crate::model_dump::ModelLibrary::parse(DUMP_SAMPLE.as_bytes()).unwrap();
    let plain = lib.get("minecraft:pig#main", 1.0).unwrap();
    let scaled = lib.get("minecraft:pig#main", 0.5).unwrap();
    let head = find_part(&plain.root, "head").unwrap();
    let half = find_part(&scaled.root, "head").unwrap();
    // Scaled about the feet: a pivot at -12 becomes -6, cube sizes halve.
    assert!((half.pivot.y - head.pivot.y * 0.5).abs() < 1e-6);
    assert!((half.cubes[0].size.x - head.cubes[0].size.x * 0.5).abs() < 1e-6);
    // Texture size and UVs are untouched by the scale.
    assert_eq!(scaled.tex_width, plain.tex_width);
    assert_eq!(half.cubes[0].uv, head.cubes[0].uv);
}

/// The table's layer ids for pig, zombie and villager (checked against each type's vanilla
/// renderer: PigRenderer, ZombieRenderer, VillagerRenderer).
#[test]
fn table_maps_pig_zombie_villager() {
    let (pig, _) = crate::registry::table_entry("minecraft:pig").unwrap();
    assert_eq!(pig, "minecraft:pig#main");
    let (zombie, _) = crate::registry::table_entry("minecraft:zombie").unwrap();
    assert_eq!(zombie, "minecraft:zombie#main");
    let (villager, _) = crate::registry::table_entry("minecraft:villager").unwrap();
    assert_eq!(villager, "minecraft:villager#main");
    // Not in the table.
    assert!(crate::registry::table_entry("minecraft:item").is_none());
    assert!(crate::registry::table_entry("minecraft:haste_mod_whatever").is_none());
}

/// A mob type the table covers gets its model when the dump is present; without the cache
/// (fast tests) the table entry still resolves and the dump lookup returns nothing, so the
/// store's fallback box applies. `model_for_name` is exercised directly because `model_for`
/// resolves kinds through the real registries.json, which the fast tests must not depend on.
#[test]
fn store_uses_dump_model_for_known_types_and_box_for_unknown() {
    match crate::registry::model_for_name("minecraft:pig") {
        Some(m) => {
            // Cache present: a real pig model with the pig texture, not the placeholder.
            assert_eq!(m.texture.path(), "entity/pig/pig_temperate");
            assert!(find_part(&m.root, "head").is_some());
        }
        None => {
            // No cache: the table still names the layer, the dump is just absent.
            assert_eq!(
                crate::registry::table_entry("minecraft:pig"),
                Some(("minecraft:pig#main", 1.0))
            );
        }
    }
    // Unknown name: no model, callers take the box.
    assert!(crate::registry::model_for_name("minecraft:no_such_mob").is_none());
    // And the box itself still stands in for the uncovered type.
    let boxed = crate::models::box_for(0.9, 0.9);
    assert_eq!(boxed.texture.to_string(), "haste:entity_box");
}

/// The real dump from the cache: pig has head, body and four legs (QuadrupedModel layout).
#[test]
#[ignore = "needs cache/oracle/26.3/entity_models.json (recorder mod dump; HASTE_CACHE overrides)"]
fn real_dump_pig_has_head_body_and_four_legs() {
    let Some(lib) = crate::model_dump::library() else {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    };
    let model = lib
        .get("minecraft:pig#main", 1.0)
        .expect("pig layer in the dump");
    let names = humanoid_parts(&model);
    for expected in [
        "head",
        "body",
        "right_hind_leg",
        "left_hind_leg",
        "right_front_leg",
        "left_front_leg",
    ] {
        assert!(
            names.contains(&expected),
            "pig part {expected} missing (has {names:?})"
        );
    }
    // The dump's pivots are shifted to feet-at-0: the pig's legs pivot at dump y = 18.
    let leg = find_part(&model.root, "right_hind_leg").unwrap();
    assert!(
        (leg.pivot.y - (18.0 - 24.0)).abs() < 1e-4,
        "leg pivot {:?}",
        leg.pivot
    );
    // Also spot-check a scaled layer (villager, 0.9375) resolves.
    let villager = lib.get("minecraft:villager#main", 0.9375);
    assert!(villager.is_some(), "villager layer missing");
}
