//! 26.3 item model definitions (`assets/minecraft/items/<name>.json`) reduced to what a GUI
//! slot shows for a default stack: `select` takes its fallback, `condition` its `on_false`,
//! `range_dispatch` the entry for property value 0 (else its fallback), `composite` all of its
//! models, `special` its `base` model's transforms with no geometry (chests, beds' special
//! parts, banners, heads, shields... are drawn by special renderers we do not have yet).
//! The walk of the definition tree follows the format documented on minecraft.wiki ("Items
//! model definition"); tint sources resolve to their `default`/`value`, `grass` through the
//! grass colormap at the given temperature/downfall.

use anyhow::{Context, Result};

use haste_core::Identifier;

use crate::assets::Assets;
use crate::model::{apply_element_rotation, face_positions, face_uvs, BakedQuad, DisplayTransform};

/// Everything needed to draw one item in a GUI slot.
#[derive(Clone, Debug, Default)]
pub struct ItemModel {
    pub parts: Vec<ItemModelPart>,
}

#[derive(Clone, Debug)]
pub struct ItemModelPart {
    pub geometry: ItemGeometry,
    /// Tint colour (ARGB) by tint index; quads/layers with an index past the end are untinted.
    pub tints: Vec<u32>,
    /// `display.gui` of the model (identity when the chain has none).
    pub gui: DisplayTransform,
    /// `display.firstperson_righthand` (identity when the chain has none).
    pub firstperson: DisplayTransform,
    /// `gui_light: front` (flat items) instead of the default side lighting.
    pub gui_light_front: bool,
    /// Composite `transformation` applied to the geometry before the display transform.
    pub transformation: Option<PartTransformation>,
}

#[derive(Clone, Debug)]
pub enum ItemGeometry {
    /// `builtin/generated`: the `layerN` textures (tint index N), drawn flat in the GUI.
    Generated { layers: Vec<String> },
    /// Model elements as quads in 0..1 block units with face-local 0..1 UVs; `texture` is the
    /// texture id each quad samples.
    Elements(Vec<BakedQuad>),
    /// Drawn by a special renderer (not implemented) or the `empty` model.
    None,
}

/// A composite part's `transformation` (translation, left rotation quaternion xyzw, scale,
/// right rotation quaternion), applied as T * L * S * R.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartTransformation {
    pub translation: [f32; 3],
    pub left_rotation: [f32; 4],
    pub scale: [f32; 3],
    pub right_rotation: [f32; 4],
}

impl Assets {
    /// The GUI model of an item (`minecraft:stone` -> `items/stone.json`).
    pub fn item_model(&mut self, item: &Identifier) -> Result<ItemModel> {
        let key = crate::assets::asset_key(item, "items", ".json");
        let bytes = self.read_asset(&key)?;
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        let root = json
            .get("model")
            .context("item definition without `model`")?;
        let mut leaves = Vec::new();
        collect_leaves(root, None, &mut leaves);
        let mut parts = Vec::with_capacity(leaves.len());
        for leaf in leaves {
            parts.push(self.item_part(&leaf)?);
        }
        Ok(ItemModel { parts })
    }

    fn item_part(&mut self, leaf: &Leaf) -> Result<ItemModelPart> {
        let tints = leaf
            .tints
            .iter()
            .map(|t| self.tint_color(t))
            .collect::<Vec<u32>>();
        let Some(model) = &leaf.model else {
            return Ok(ItemModelPart {
                geometry: ItemGeometry::None,
                tints,
                gui: DisplayTransform::IDENTITY,
                firstperson: DisplayTransform::IDENTITY,
                gui_light_front: false,
                transformation: leaf.transformation,
            });
        };
        let resolved = self.resolve_model(&Identifier::parse(model)?)?;
        let gui = resolved.gui_display.unwrap_or(DisplayTransform::IDENTITY);
        let firstperson = resolved
            .firstperson_display
            .unwrap_or(DisplayTransform::IDENTITY);
        let geometry = if leaf.special {
            ItemGeometry::None
        } else if resolved.elements.is_empty() {
            let mut layers = Vec::new();
            while let Some(t) = resolved.textures.get(&format!("layer{}", layers.len())) {
                layers.push(normalize_texture_id(t));
            }
            if layers.is_empty() {
                ItemGeometry::None
            } else {
                ItemGeometry::Generated { layers }
            }
        } else {
            ItemGeometry::Elements(bake_elements(&resolved))
        };
        Ok(ItemModelPart {
            geometry,
            tints,
            gui,
            firstperson,
            gui_light_front: resolved.gui_light,
            transformation: leaf.transformation,
        })
    }

    fn tint_color(&mut self, tint: &serde_json::Value) -> u32 {
        let kind = tint
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim_start_matches("minecraft:");
        let color = match kind {
            "constant" => tint.get("value").and_then(parse_rgb),
            "grass" => {
                let t = tint
                    .get("temperature")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5) as f32;
                let d = tint.get("downfall").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                self.grass_color(t, d)
            }
            _ => tint.get("default").and_then(parse_rgb),
        };
        color.unwrap_or(0xFFFF_FFFF) | 0xFF00_0000
    }

    /// The grass colormap at (temperature, downfall), minecraft.wiki "Color" formula:
    /// x = (1 - t) * 255, y = (1 - d * t) * 255 with both clamped to 0..1.
    fn grass_color(&mut self, temperature: f32, downfall: f32) -> Option<u32> {
        let img = self
            .texture(&Identifier::minecraft("colormap/grass"))
            .ok()?;
        let t = temperature.clamp(0.0, 1.0);
        let d = downfall.clamp(0.0, 1.0) * t;
        let x = (((1.0 - t) * 255.0) as u32).min(img.width() - 1);
        let y = (((1.0 - d) * 255.0) as u32).min(img.height() - 1);
        let p = img.get_pixel(x, y).0;
        Some(0xFF00_0000 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32)
    }
}

use crate::bake::normalize_texture_id;

/// An integer ARGB or a three-float RGB array.
fn parse_rgb(v: &serde_json::Value) -> Option<u32> {
    if let Some(i) = v.as_i64() {
        return Some(i as i32 as u32);
    }
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let c = |i: usize| ((a[i].as_f64().unwrap_or(0.0) as f32 * 255.0).floor() as u32) & 0xFF;
    Some(0xFF00_0000 | c(0) << 16 | c(1) << 8 | c(2))
}

struct Leaf {
    model: Option<String>,
    tints: Vec<serde_json::Value>,
    special: bool,
    transformation: Option<PartTransformation>,
}

fn node_type(node: &serde_json::Value) -> &str {
    node.get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim_start_matches("minecraft:")
}

fn collect_leaves(
    node: &serde_json::Value,
    transformation: Option<PartTransformation>,
    out: &mut Vec<Leaf>,
) {
    let transformation = node
        .get("transformation")
        .and_then(parse_transformation)
        .or(transformation);
    match node_type(node) {
        "model" => out.push(Leaf {
            model: node.get("model").and_then(|m| m.as_str()).map(String::from),
            tints: node
                .get("tints")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default(),
            special: false,
            transformation,
        }),
        "special" => out.push(Leaf {
            model: node.get("base").and_then(|m| m.as_str()).map(String::from),
            tints: Vec::new(),
            special: true,
            transformation,
        }),
        "composite" => {
            for m in node
                .get("models")
                .and_then(|m| m.as_array())
                .into_iter()
                .flatten()
            {
                collect_leaves(m, transformation, out);
            }
        }
        "condition" => {
            if let Some(n) = node.get("on_false") {
                collect_leaves(n, transformation, out);
            }
        }
        "select" => {
            let first_case = node
                .get("cases")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("model"));
            if let Some(n) = node.get("fallback").or(first_case) {
                collect_leaves(n, transformation, out);
            }
        }
        "range_dispatch" => {
            // Property value 0 for a default stack: the entry with the largest threshold <= 0.
            let entry = node
                .get("entries")
                .and_then(|e| e.as_array())
                .into_iter()
                .flatten()
                .filter(|e| {
                    e.get("threshold")
                        .and_then(|t| t.as_f64())
                        .is_some_and(|t| t <= 0.0)
                })
                .max_by(|a, b| {
                    let t = |e: &serde_json::Value| e["threshold"].as_f64().unwrap_or(0.0);
                    t(a).total_cmp(&t(b))
                })
                .and_then(|e| e.get("model"));
            if let Some(n) = entry.or(node.get("fallback")) {
                collect_leaves(n, transformation, out);
            }
        }
        // `empty`, `bundle/selected_item`, unknown types: nothing to draw.
        _ => {}
    }
}

fn parse_transformation(v: &serde_json::Value) -> Option<PartTransformation> {
    let arr = |k: &str| -> Option<Vec<f32>> {
        v.get(k)?
            .as_array()?
            .iter()
            .map(|x| x.as_f64().map(|f| f as f32))
            .collect()
    };
    let v3 = |k: &str, d: f32| -> [f32; 3] {
        arr(k)
            .filter(|a| a.len() == 3)
            .map(|a| [a[0], a[1], a[2]])
            .unwrap_or([d; 3])
    };
    let quat = |k: &str| -> [f32; 4] {
        arr(k)
            .filter(|a| a.len() == 4)
            .map(|a| [a[0], a[1], a[2], a[3]])
            .unwrap_or([0.0, 0.0, 0.0, 1.0])
    };
    v.as_object()?;
    Some(PartTransformation {
        translation: v3("translation", 0.0),
        left_rotation: quat("left_rotation"),
        scale: v3("scale", 1.0),
        right_rotation: quat("right_rotation"),
    })
}

/// Model elements to quads (0..1 block units, face-local UVs), the same geometry rules as
/// block baking without a blockstate rotation.
fn bake_elements(resolved: &crate::model::ResolvedModel) -> Vec<BakedQuad> {
    let mut quads = Vec::new();
    for el in &resolved.elements {
        let from = el.from.map(|v| v / 16.0);
        let to = el.to.map(|v| v / 16.0);
        // `faces` is keyed by direction in vanilla's fixed order (down, up, north,
        // south, west, east), which the map iteration follows.
        for (dir, face) in &el.faces {
            let Some(tex) = crate::bake::face_texture_id(&face.texture, &resolved.textures) else {
                continue;
            };
            let positions = apply_element_rotation(face_positions(*dir, from, to), &el.rotation);
            quads.push(BakedQuad {
                positions,
                uvs: face_uvs(*dir, from, to, face.uv.as_ref(), face.rotation),
                texture: normalize_texture_id(&tex),
                cullface: None,
                shade_face: el.shade.then_some(*dir),
                tint_index: face.tint_index.and_then(|i| u32::try_from(i).ok()),
                direction: crate::model::direction_from_positions(&positions).unwrap_or(*dir),
            });
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(json: &str) -> Vec<Leaf> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut out = Vec::new();
        collect_leaves(&v["model"], None, &mut out);
        out
    }

    #[test]
    fn select_takes_fallback_condition_on_false() {
        let l = leaves(
            r#"{"model":{"type":"minecraft:select","cases":[{"when":"x","model":{"type":"minecraft:model","model":"a"}}],
            "fallback":{"type":"minecraft:condition","on_true":{"type":"minecraft:model","model":"b"},"on_false":{"type":"minecraft:model","model":"c"}}}}"#,
        );
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].model.as_deref(), Some("c"));
    }

    #[test]
    fn range_dispatch_picks_value_zero() {
        let l = leaves(
            r#"{"model":{"type":"minecraft:range_dispatch","entries":[{"threshold":0.5,"model":{"type":"model","model":"half"}},{"threshold":0.0,"model":{"type":"model","model":"zero"}}],
            "fallback":{"type":"model","model":"fb"}}}"#,
        );
        assert_eq!(l[0].model.as_deref(), Some("zero"));
        let l = leaves(
            r#"{"model":{"type":"minecraft:range_dispatch","entries":[{"threshold":0.65,"model":{"type":"model","model":"p1"}}],
            "fallback":{"type":"model","model":"p0"}}}"#,
        );
        assert_eq!(l[0].model.as_deref(), Some("p0"));
    }

    #[test]
    fn composite_keeps_part_transformations() {
        let l = leaves(
            r#"{"model":{"type":"minecraft:composite","models":[{"type":"model","model":"head"},
            {"type":"model","model":"foot","transformation":{"translation":[0,0,1],"left_rotation":[0,0,0,1],"right_rotation":[0,0,0,1],"scale":[1,1,1]}}]}}"#,
        );
        assert_eq!(l.len(), 2);
        assert!(l[0].transformation.is_none());
        assert_eq!(l[1].transformation.unwrap().translation, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn tint_colors_parse() {
        assert_eq!(parse_rgb(&serde_json::json!(-12012264)), Some(0xFF48B518));
        assert_eq!(
            parse_rgb(&serde_json::json!([1.0, 0.0, 0.5])),
            Some(0xFFFF007F)
        );
    }

    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn jar_item_models() {
        let jar = haste_core::paths::client_jar();
        let mut a = Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
        let stone = a.item_model(&Identifier::minecraft("stone")).unwrap();
        assert_eq!(stone.parts.len(), 1);
        assert!(matches!(&stone.parts[0].geometry, ItemGeometry::Elements(q) if q.len() == 6));
        assert_eq!(stone.parts[0].gui.rotation, [30.0, 225.0, 0.0]);
        assert!(!stone.parts[0].gui_light_front);

        let sword = a
            .item_model(&Identifier::minecraft("diamond_sword"))
            .unwrap();
        assert!(
            matches!(&sword.parts[0].geometry, ItemGeometry::Generated { layers } if layers == &["minecraft:item/diamond_sword"])
        );
        assert!(sword.parts[0].gui_light_front);

        let potion = a.item_model(&Identifier::minecraft("potion")).unwrap();
        assert_eq!(potion.parts[0].tints, vec![-13083194i32 as u32]);
        assert!(
            matches!(&potion.parts[0].geometry, ItemGeometry::Generated { layers } if layers.len() == 2)
        );

        let grass = a.item_model(&Identifier::minecraft("grass_block")).unwrap();
        let g = grass.parts[0].tints[0];
        assert!(
            (g >> 8) & 0xFF > (g >> 16) & 0xFF,
            "grass tint is green: {g:08x}"
        );

        let bed = a.item_model(&Identifier::minecraft("red_bed")).unwrap();
        assert_eq!(bed.parts.len(), 2);

        let chest = a.item_model(&Identifier::minecraft("chest")).unwrap();
        assert!(matches!(chest.parts[0].geometry, ItemGeometry::None));

        // Every item definition in the jar resolves without error.
        let names: Vec<String> = a
            .names_in("assets/minecraft/items/", ".json")
            .collect::<Vec<_>>();
        assert!(names.len() > 1000);
        let mut failed = Vec::new();
        for n in &names {
            if let Err(e) = a.item_model(&Identifier::minecraft(n)) {
                failed.push(format!("{n}: {e:#}"));
            }
        }
        assert!(
            failed.is_empty(),
            "{} failed: {:?}",
            failed.len(),
            &failed[..failed.len().min(10)]
        );
    }
}
