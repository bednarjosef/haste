//! 26.3 item model definitions (`assets/minecraft/items/<name>.json`) reduced to what a GUI
//! slot shows for a default stack: `select` takes its fallback, `condition` its `on_false`,
//! `range_dispatch` the entry for property value 0 (else its fallback), `composite` all of its
//! models, `special` its `base` model's transforms with no geometry (chests, beds' special
//! parts, banners, heads, shields... are drawn by special renderers we do not have yet).
//! The walk of the definition tree follows the format documented on minecraft.wiki ("Items
//! model definition"); tint sources resolve to their `default`/`value`, `grass` through the
//! grass colormap at the given temperature/downfall.
//!
//! Every leaf's model is the one its definition names (`minecraft:item/short_grass`,
//! `minecraft:block/stone`, ...): vanilla's `ItemModelResolver.appendItemLayers` asks
//! `ModelManager` for the item model of the `minecraft:item_model` component, and
//! `CuboidItemModelWrapper.Unbaked` bakes that model's geometry. A block item (`stone`) keeps
//! the block model; a plant item gets `builtin/generated`, whose layer sprites are extruded
//! 1/16 deep (`ItemModelGenerator`: MIN_Z 7.5/16, MAX_Z 8.5/16, the mirrored north face and a
//! side quad per opaque sprite pixel with a transparent neighbour).

use anyhow::{Context, Result};

use haste_core::Identifier;

use crate::assets::Assets;
use crate::model::{
    apply_element_rotation, face_positions, face_uvs, BakedQuad, Direction, DisplayTransform,
};

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
    /// `display.ground` (identity when the chain has none): dropped item entities.
    pub ground: DisplayTransform,
    /// `display.fixed` (identity when the chain has none): item frames and other fixed mounts.
    pub fixed: DisplayTransform,
    /// `gui_light: front` (flat items) instead of the default side lighting.
    pub gui_light_front: bool,
    /// Composite `transformation` applied to the geometry before the display transform.
    pub transformation: Option<PartTransformation>,
}

#[derive(Clone, Debug)]
pub enum ItemGeometry {
    /// `builtin/generated`: the `layerN` textures, each extruded 1/16 deep like vanilla's
    /// `ItemModelGenerator` (front face, mirrored back face, sides at the sprite's edges).
    Generated { layers: Vec<GeneratedLayer> },
    /// Model elements as quads in 0..1 block units with face-local 0..1 UVs; `texture` is the
    /// texture id each quad samples.
    Elements(Vec<BakedQuad>),
    /// Drawn by a special renderer (not implemented) or the `empty` model.
    None,
}

/// One `builtin/generated` layer: the sprite it samples and the quads baked from it.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedLayer {
    /// The layer's texture id (`layer0` resolved).
    pub texture: String,
    /// The extruded sprite: the front face at z = 8.5/16, the mirrored back face at 7.5/16 and
    /// the 1/16-deep sides, in 0..1 item space with 0..1 UVs. `tint_index` is the layer's
    /// index, so every quad of the layer is tinted alike.
    pub quads: Vec<BakedQuad>,
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
                ground: DisplayTransform::IDENTITY,
                fixed: DisplayTransform::IDENTITY,
                gui_light_front: false,
                transformation: leaf.transformation,
            });
        };
        let resolved = self.resolve_model(&Identifier::parse(model)?)?;
        let gui = resolved.gui_display.unwrap_or(DisplayTransform::IDENTITY);
        let firstperson = resolved
            .firstperson_display
            .unwrap_or(DisplayTransform::IDENTITY);
        let ground = resolved
            .ground_display
            .unwrap_or(DisplayTransform::IDENTITY);
        let fixed = resolved.fixed_display.unwrap_or(DisplayTransform::IDENTITY);
        let geometry = if leaf.special {
            ItemGeometry::None
        } else if resolved.elements.is_empty() {
            let mut layers = Vec::new();
            while let Some(t) = resolved.textures.get(&format!("layer{}", layers.len())) {
                let texture = normalize_texture_id(t);
                let image = Identifier::parse(&texture)
                    .ok()
                    .and_then(|id| self.texture(&id).ok());
                layers.push(GeneratedLayer {
                    texture,
                    quads: generated_layer_quads(image.as_ref(), layers.len() as u32),
                });
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
            ground,
            fixed,
            gui_light_front: resolved.gui_light,
            transformation: leaf.transformation,
        })
    }

    /// A block model's element faces as quads (positions in 0..1 block units, face-local 0..1
    /// UVs, the texture id each face samples), without blockstate rotation or atlas UVs. For
    /// entity renderers that draw a block model through their own atlas (item frames).
    pub fn model_quads(&mut self, id: &Identifier) -> Result<Vec<BakedQuad>> {
        let resolved = self.resolve_model(id)?;
        Ok(bake_elements(&resolved))
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

/// Vanilla's `ItemModelGenerator` extrusion of one `builtin/generated` layer: MIN_Z 7.5/16 and
/// MAX_Z 8.5/16 around the sprite, the south (z = 8.5/16) face with the sprite's UVs, the
/// north (z = 7.5/16) face with them mirrored, and a 1/16-deep side quad for every opaque
/// sprite pixel whose neighbour in that direction is transparent, sampling that pixel (the UVs
/// are inset 0.1 texel inside it, as vanilla's are). `image` is the sprite; without it
/// (an unreadable texture) only the two faces are baked. Positions are in 0..1 item space with
/// y up and the front towards +z, UVs 0..1, and every quad carries `tint_index`.
fn generated_layer_quads(image: Option<&image::RgbaImage>, tint_index: u32) -> Vec<BakedQuad> {
    // Vanilla `ItemModelGenerator.bakeExtrudedSprite`: from (0,0,7.5) to (16,16,8.5), the south
    // face with the sprite's UVs and the north face with them mirrored.
    let mut quads = vec![
        face_quad(
            Direction::South,
            [0.0, 0.0, 7.5],
            [16.0, 16.0, 8.5],
            [0.0, 0.0, 16.0, 16.0],
            tint_index,
        ),
        face_quad(
            Direction::North,
            [0.0, 0.0, 7.5],
            [16.0, 16.0, 8.5],
            [16.0, 0.0, 0.0, 16.0],
            tint_index,
        ),
    ];
    let Some(image) = image else {
        return quads;
    };
    let (w, h) = image.dimensions();
    let x_scale = 16.0 / w.max(1) as f32;
    let y_scale = 16.0 / h.max(1) as f32;
    // Out of the sprite counts as transparent (vanilla's `isTransparent` returns true there).
    let opaque = |x: i32, y: i32| {
        (0..w as i32).contains(&x)
            && (0..h as i32).contains(&y)
            && image.get_pixel(x as u32, y as u32).0[3] != 0
    };
    for py in 0..h as i32 {
        for px in 0..w as i32 {
            if !opaque(px, py) {
                continue;
            }
            let x0 = px as f32 * x_scale;
            let x1 = x0 + x_scale;
            let y_top = 16.0 - py as f32 * y_scale;
            let y_bottom = y_top - y_scale;
            // The pixel's window in 0..16 sprite units, inset by vanilla's 0.1 texel.
            let u0 = x0 + 0.1 * x_scale;
            let u1 = x1 - 0.1 * x_scale;
            let v0 = py as f32 * y_scale + 0.1 * y_scale;
            let v1 = v0 + y_scale - 0.2 * y_scale;
            if !opaque(px, py - 1) {
                quads.push(face_quad(
                    Direction::Up,
                    [x0, y_top, 7.5],
                    [x1, y_top, 8.5],
                    [u0, v0, u1, v1],
                    tint_index,
                ));
            }
            if !opaque(px, py + 1) {
                quads.push(face_quad(
                    Direction::Down,
                    [x0, y_bottom, 7.5],
                    [x1, y_bottom, 8.5],
                    [u0, v0, u1, v1],
                    tint_index,
                ));
            }
            if !opaque(px - 1, py) {
                quads.push(face_quad(
                    Direction::West,
                    [x0, y_bottom, 7.5],
                    [x0, y_top, 8.5],
                    [u0, v1, u1, v0],
                    tint_index,
                ));
            }
            if !opaque(px + 1, py) {
                quads.push(face_quad(
                    Direction::East,
                    [x1, y_bottom, 7.5],
                    [x1, y_top, 8.5],
                    [u0, v1, u1, v0],
                    tint_index,
                ));
            }
        }
    }
    quads
}

/// One quad of the extruded sprite: positions from `from`/`to` (0..16 sprite units) on `dir` in
/// vanilla's `FaceInfo` order, and the UV rectangle `uv` (0..16 sprite units) mapped as
/// vanilla's `CuboidFace.UVs` corner cycle is.
fn face_quad(
    dir: Direction,
    from: [f32; 3],
    to: [f32; 3],
    uv: [f32; 4],
    tint_index: u32,
) -> BakedQuad {
    let mut quad = BakedQuad::from_element(
        dir,
        from.map(|v| v / 16.0),
        to.map(|v| v / 16.0),
        Some(&uv),
        None,
    );
    quad.texture = String::new();
    quad.cullface = None;
    quad.shade_face = Some(dir);
    quad.tint_index = Some(tint_index);
    quad.direction = dir;
    quad
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

    /// A generated layer extruded like vanilla's `ItemModelGenerator`: the sprite's front face
    /// at z = 8.5/16, the mirrored back face at 7.5/16, and at least one side quad.
    fn extruded_1_over_16(quads: &[BakedQuad]) -> bool {
        let face_at = |dir: Direction, z: f32| {
            quads
                .iter()
                .any(|q| q.direction == dir && q.positions.iter().all(|p| (p[2] - z).abs() < 1e-6))
        };
        face_at(Direction::South, 8.5 / 16.0)
            && face_at(Direction::North, 7.5 / 16.0)
            && quads.iter().any(|q| {
                matches!(
                    q.direction,
                    Direction::Up | Direction::Down | Direction::West | Direction::East
                )
            })
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
        assert!(matches!(
            &sword.parts[0].geometry,
            ItemGeometry::Generated { layers } if layers.len() == 1 && layers[0].texture == "minecraft:item/diamond_sword"
        ));
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

    /// Plants and other flat items take the item model their `items/<id>.json` names
    /// (`minecraft:item/<id>`, a `builtin/generated` model over the block's sprite), not the
    /// block model; block items keep the block model. Follows `ItemModelResolver` and
    /// `CuboidItemModelWrapper.Unbaked` on the one hand and `ItemModelGenerator` on the other.
    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn jar_flat_items_take_the_item_model_block_items_keep_the_block_model() {
        let jar = haste_core::paths::client_jar();
        let mut a = Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
        for (name, sprite) in [
            ("short_grass", "minecraft:block/short_grass"),
            ("oak_sapling", "minecraft:block/oak_sapling"),
            ("poppy", "minecraft:block/poppy"),
            ("rail", "minecraft:block/rail"),
            ("oak_door", "minecraft:item/oak_door"),
            ("torch", "minecraft:block/torch"),
        ] {
            let model = a.item_model(&Identifier::minecraft(name)).unwrap();
            let ItemGeometry::Generated { layers } = &model.parts[0].geometry else {
                panic!("{name} is not a generated flat item");
            };
            assert_eq!(layers.len(), 1, "{name}");
            assert_eq!(layers[0].texture, sprite, "{name}");
            assert!(model.parts[0].gui_light_front, "{name} is front-lit");
            assert!(
                extruded_1_over_16(&layers[0].quads),
                "{name} is not extruded 1/16 deep"
            );
        }
        for name in ["stone", "oak_planks"] {
            let model = a.item_model(&Identifier::minecraft(name)).unwrap();
            assert!(
                matches!(&model.parts[0].geometry, ItemGeometry::Elements(q) if q.len() == 6),
                "{name} should keep its block model"
            );
            assert!(!model.parts[0].gui_light_front, "{name} is side-lit");
        }
        // An id with no item definition is missing, never a panic.
        assert!(a
            .item_model(&Identifier::minecraft("no_such_item"))
            .is_err());
    }

    /// The extruded sprite: front and back face 1/16 apart between 7.5/16 and 8.5/16, and one
    /// side quad per opaque sprite pixel with a transparent neighbour.
    #[test]
    fn generated_geometry_is_extruded() {
        // A 4x4 sprite: the left column opaque (its left and, for the top/bottom pixels, its
        // up/down neighbours transparent), the rest transparent.
        let mut image = image::RgbaImage::new(4, 4);
        for y in 0..4 {
            image.put_pixel(0, y, image::Rgba([255, 0, 0, 255]));
        }
        let quads = generated_layer_quads(Some(&image), 2);
        assert_eq!(
            quads.len(),
            2 + 4 + 4 + 2,
            "faces + 4 west + 4 east sides + the top and bottom pixel's up/down sides"
        );
        assert!(quads.iter().all(|q| q.tint_index == Some(2)));
        let front = quads
            .iter()
            .find(|q| q.direction == Direction::South)
            .unwrap();
        let back = quads
            .iter()
            .find(|q| q.direction == Direction::North)
            .unwrap();
        let z = |p: [[f32; 3]; 4]| p.map(|v| v[2]);
        assert!(z(front.positions)
            .iter()
            .all(|&v| (v - 8.5 / 16.0).abs() < 1e-6));
        assert!(z(back.positions)
            .iter()
            .all(|&v| (v - 7.5 / 16.0).abs() < 1e-6));
        // The four west sides sit at the pixel column's x, the east ones at the next column's.
        let left = quads
            .iter()
            .filter(|q| q.direction == Direction::West)
            .count();
        assert_eq!(left, 4);
        let east = quads
            .iter()
            .filter(|q| q.direction == Direction::East)
            .count();
        assert_eq!(east, 4);
        for q in quads
            .iter()
            .filter(|q| matches!(q.direction, Direction::West | Direction::East))
        {
            assert!(
                (q.positions[0][0] - q.positions[1][0]).abs() < 1e-6,
                "one constant x"
            );
            assert!(q
                .positions
                .iter()
                .all(|p| (7.5 / 16.0..=8.5 / 16.0).contains(&p[2])));
        }
        assert_eq!(
            quads
                .iter()
                .filter(|q| q.direction == Direction::Up || q.direction == Direction::Down)
                .count(),
            2
        );
        // A fully opaque sprite: only the border pixels have transparent neighbours.
        let solid = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 255, 0, 255]));
        let quads = generated_layer_quads(Some(&solid), 0);
        assert_eq!(quads.len(), 2 + 16, "two faces + 16 border side quads");
        // Without the sprite (an unreadable texture) the two faces alone remain.
        assert_eq!(generated_layer_quads(None, 0).len(), 2);
    }
}
