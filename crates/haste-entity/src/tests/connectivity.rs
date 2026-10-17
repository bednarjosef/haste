use super::*;
// ---------------------------------------------------------------------------
// Rest-pose connectivity audit (every table-covered type, no images needed).
// ---------------------------------------------------------------------------

/// One cube's world-space box at rest pose, in the renderer's block space: the same chain
/// `emit_part` builds (parent * translate(pivot) * Rz * Ry * Rx per part), then the model
/// scale (1/16) and the renderer's y flip ((-1, -1, 1) * s / 16) that puts the feet at y=0
/// with y up. `pos` is the box's centre, `half` its half-extents.
struct WorldBox {
    lo: glam::Vec3,
    hi: glam::Vec3,
}

/// Collects every cube's world box (see `WorldBox`), rest pose, no overrides.
fn world_boxes(part: &ModelPart, parent: glam::Mat4, out: &mut Vec<WorldBox>) {
    let m = parent
        * glam::Mat4::from_translation(part.pivot)
        * glam::Mat4::from_rotation_z(part.rotation.z)
        * glam::Mat4::from_rotation_y(part.rotation.y)
        * glam::Mat4::from_rotation_x(part.rotation.x);
    for c in &part.cubes {
        let lo = c.origin - glam::Vec3::splat(c.inflate);
        let hi = c.origin + c.size + glam::Vec3::splat(c.inflate);
        // Model space (y down, 1/16 units) -> block space (y up): mirror x and y, like the
        // renderer's scale (-s, -s, s) / 16 (entity.rs texture_and_scale path). A rotated
        // cube's AABB needs all eight corners, not just the transformed min/max.
        let to_blocks = |p: glam::Vec3| {
            let b = m.transform_point3(p) / 16.0;
            glam::Vec3::new(-b.x, -b.y, b.z)
        };
        let mut blo = glam::Vec3::splat(f32::INFINITY);
        let mut bhi = glam::Vec3::splat(f32::NEG_INFINITY);
        for cx in [lo.x, hi.x] {
            for cy in [lo.y, hi.y] {
                for cz in [lo.z, hi.z] {
                    let p = to_blocks(glam::Vec3::new(cx, cy, cz));
                    blo = blo.min(p);
                    bhi = bhi.max(p);
                }
            }
        }
        out.push(WorldBox { lo: blo, hi: bhi });
    }
    for child in &part.children {
        world_boxes(child, m, out);
    }
}

/// The rest-pose audit: for every table-covered entity type (91 mob kinds), the dumped model
/// at rest pose (no animation, no overrides - the pose the dump baked) must be
///
/// 1. **connected**: one piece, any cube reachable from any other through touching or
///    overlapping cubes. A model that splits into islands means a part landed in the wrong
///    place (the horse-head bug class: the neck's children were shifted twice).
/// 2. **grounded** (land types): some geometry within `margin` of the ground plane y=0. A
///    model floating a whole block above it means the whole tree is misplaced (the copper
///    golem bug class: the layer's root PartPose translation was lost and the ground shift
///    applied on top). Types vanilla does not draw standing on the ground (swimming, flying,
///    hovering) are exempt; each names the vanilla reason.
///
/// A model *exceeding its hitbox* is NOT a failure: vanilla models routinely extend past
/// their hitbox (horse heads, bat wings, warden arms), so dimension checks only produce noise.
/// Every exemption was checked against the vanilla 26.3 model/renderer code by hand.
#[test]
#[ignore = "needs cache/oracle/26.3 (entity_models.json + entity_dimensions.json; HASTE_CACHE overrides)"]
fn rest_pose_models_are_connected_and_grounded() {
    let Some(lib) = crate::model_dump::library() else {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    };
    // entity_dimensions.json: {version, types: [{id, type, width, height, eye_height}]}.
    let dims_path = haste_core::paths::oracle_dir().join("entity_dimensions.json");
    let Ok(dims_bytes) = std::fs::read(&dims_path) else {
        eprintln!(
            "skipped: no entity dimensions dump at {}",
            dims_path.display()
        );
        return;
    };
    let dims: serde_json::Value = serde_json::from_slice(&dims_bytes).unwrap();
    let height_of = |name: &str| -> Option<f32> {
        dims["types"]
            .as_array()?
            .iter()
            .find(|t| t["type"].as_str() == Some(name))
            .map(|t| t["height"].as_f64().unwrap() as f32)
    };

    let mut failures = Vec::new();
    for name in LIVING_TYPES {
        let (layer, _root_scale) = crate::registry::table_entry(name)
            .unwrap_or_else(|| panic!("{name} missing from the model table"));
        let model = crate::registry::model_for_name(name);
        let Some(model) = model else {
            // A table row whose layer is absent from the dump is a coverage gap, not a
            // connectivity failure; the box fallback covers it at runtime.
            if lib.get(layer, 1.0).is_none() {
                failures.push(format!("{name}: layer {layer} missing from the dump"));
            }
            continue;
        };
        let Some(_height) = height_of(name) else {
            failures.push(format!("{name}: no dimensions in entity_dimensions.json"));
            continue;
        };
        let mut boxes = Vec::new();
        world_boxes(&model.root, glam::Mat4::IDENTITY, &mut boxes);
        if boxes.is_empty() {
            failures.push(format!("{name}: model has no cubes"));
            continue;
        }
        let mut min = glam::Vec3::splat(f32::INFINITY);
        let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
        for b in &boxes {
            min = min.min(b.lo);
            max = max.max(b.hi);
        }
        // Grounding: land types stand on the plane (the renderer puts model y=24 at the
        // entity position). Ground slack of 2 model units covers vanilla's 0.016 ground
        // fudge and hoof-antenna details drawn at the exact plane.
        let grounded = min.y <= 2.0 / 16.0;
        if !grounded && !GROUND_EXEMPT.contains(name) {
            failures.push(format!(
                "{name}: floats {min:.2} blocks above the ground at rest"
            ));
        }
        // Connectivity: union-find over boxes that touch or overlap (faces, not just
        // interiors; a model's parts attach face-to-face).
        let n = boxes.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], i: usize) -> usize {
            let mut root = i;
            while parent[root] != root {
                root = parent[root];
            }
            let mut j = i;
            while parent[j] != root {
                let next = parent[j];
                parent[j] = root;
                j = next;
            }
            root
        }
        let touching = |a: &WorldBox, b: &WorldBox| {
            // Closed-box overlap with a hair of tolerance for shared faces.
            const EPS: f32 = 1.0 / 64.0;
            a.lo.x <= b.hi.x + EPS
                && b.lo.x <= a.hi.x + EPS
                && a.lo.y <= b.hi.y + EPS
                && b.lo.y <= a.hi.y + EPS
                && a.lo.z <= b.hi.z + EPS
                && b.lo.z <= a.hi.z + EPS
        };
        for i in 0..n {
            for j in (i + 1)..n {
                if touching(&boxes[i], &boxes[j]) {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
        let roots: std::collections::HashSet<usize> =
            (0..n).map(|i| find(&mut parent, i)).collect();
        if roots.len() > 1 && !CONNECTIVITY_EXCEPTIONS.contains(name) {
            failures.push(format!(
                "{name}: rest pose splits into {} disconnected pieces",
                roots.len()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} model audit failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Types whose vanilla rest pose is genuinely several separate pieces; each names the vanilla
/// reason (checked against the 26.3 client's model code by hand).
const CONNECTIVITY_EXCEPTIONS: &[&str] = &[
    // BreezeModel.createBodyLayer: only "head" and "rods" are retained; the three wind rods
    // whirl around the head and touch nothing at rest.
    "minecraft:breeze",
    // BlazeModel.createBodyLayer: the head plus twelve independently spinning rods.
    "minecraft:blaze",
];

/// Types vanilla does not draw standing on the ground at rest, so the grounding check does
/// not apply (each names the vanilla reason).
const GROUND_EXEMPT: &[&str] = &[
    // Water mobs (SquidModel/GlowSquidModel: tentacles hang below the body, which vanilla
    // centers on the entity position; they are not drawn standing).
    "minecraft:squid",
    "minecraft:glow_squid",
    "minecraft:guardian",
    "minecraft:elder_guardian",
    // Flying / hovering mobs.
    "minecraft:ghast",
    "minecraft:happy_ghast",
    "minecraft:phantom",
    "minecraft:bee",
    "minecraft:allay",
    "minecraft:bat",
    "minecraft:parrot",
    // BlazeModel: head at model y=0 with the rod rings below it (a hovering column of fire;
    // the lowest rod still clears the ground).
    "minecraft:blaze",
    // BreezeModel: the drawn layer is head + whirling rods; only the separate wind layer
    // (BreezeWindLayer, drawn by its own renderer pass) reaches the ground.
    "minecraft:breeze",
    // WitherBossModel: a flying boss whose ribcage hovers above the tail (WitherBossRenderer
    // scales it 2x around the hovering pose; nothing stands on the ground).
    "minecraft:wither",
    // Schooling fish (CodModel/SalmonModel: the body rides above the entity position).
    "minecraft:cod",
    "minecraft:salmon",
    "minecraft:tropical_fish",
    "minecraft:tadpole",
    "minecraft:axolotl",
    "minecraft:dolphin",
    // SulfurCubeRenderer.scale translates the model down ~0.92 blocks (vanilla draws the
    // cube centered on the entity position plus a vOffset, not standing on the ground);
    // that per-renderer offset is not model geometry.
    "minecraft:sulfur_cube",
];
