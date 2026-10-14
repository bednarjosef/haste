//! Loader for the recorder mod's entity-model dump (`cache/oracle/26.3/entity_models.json`,
//! oracles; format in `crates/haste-testkit/FORMATS.md`).
//!
//! The dump holds every model layer the real game's `EntityModelSet` baked, in vanilla model
//! space: units are 1/16 block, y grows downward, and the standard ground plane sits at
//! y = 24 model units (vanilla draws models with a -1.501-block offset, i.e. 24.016 units,
//! after flipping y; the 0.016 fudge is invisible and dropped here). `CubeModel` instead has
//! the feet at y = 0, so converting a layer subtracts 24 from every part pivot's y (cube
//! origins are pivot-relative and need no shift). Layers whose vanilla definition was built
//! with a root `MeshTransformer.scaling(f)` (villager 0.9375, ghast 4.5, giant 6.0, ...) are
//! dumped unscaled - the dump's root part has no pose - so the caller's lookup key carries
//! the scale to apply on top of the layer geometry.
//!
//! The library is loaded once, lazily, and shares one `Arc<CubeModel>` per layer: rendering an
//! entity never allocates. A missing or unreadable dump logs once and yields an empty
//! library; callers fall back to the generic box.
use std::sync::{Arc, OnceLock};

use ahash::AHashMap;
use haste_core::model::{Cube, CubeModel, ModelPart};
use haste_core::Identifier;
use serde::Deserialize;

/// The vanilla ground plane in model units (see the module doc).
const GROUND_Y: f32 = 24.0;
/// The dump's format version this loader understands.
const KNOWN_VERSION: u32 = 1;

#[derive(Deserialize)]
struct DumpFile {
    version: u32,
    #[serde(default)]
    #[allow(dead_code)] // traceability field written by the mod; not needed once loaded
    minecraft: Option<String>,
    layers: Vec<LayerJson>,
}

#[derive(Deserialize)]
struct LayerJson {
    layer: String,
    texture_width: u32,
    texture_height: u32,
    parts: Vec<PartJson>,
}

#[derive(Deserialize)]
struct PartJson {
    name: String,
    pivot: [f32; 3],
    #[serde(default)]
    rotation: [f32; 3],
    #[serde(default)]
    scale: [f32; 3],
    #[serde(default)]
    cubes: Vec<CubeJson>,
    #[serde(default)]
    children: Vec<PartJson>,
}

#[derive(Deserialize)]
struct CubeJson {
    origin: [f32; 3],
    size: [f32; 3],
    #[serde(default)]
    u: f32,
    #[serde(default)]
    v: f32,
    #[serde(default)]
    mirror: bool,
    /// The dump has per-axis grow (CubeDeformation growX/Y/Z); our `Cube` only has the uniform
    /// field, so the mean is used (vanilla definitions are overwhelmingly uniform).
    #[serde(default)]
    inflate: [f32; 3],
}

/// A loaded layer: the converted model tree (ground at y = 0) and the root scale vanilla's
/// layer definition applied, if any (1.0 when it did not). Rendering multiplies the scale in.
pub struct Layer {
    pub model: Arc<CubeModel>,
    pub root_scale: f32,
}

/// The shared library of dumped layers, keyed by layer id (`minecraft:pig#main`).
pub struct ModelLibrary {
    layers: AHashMap<String, Layer>,
}

impl ModelLibrary {
    /// Parses a dump (the file's bytes). Fails only on a bad format version or malformed JSON;
    /// individual layers with unusable geometry are skipped rather than failing the file.
    pub fn parse(bytes: &[u8]) -> anyhow::Result<ModelLibrary> {
        let file: DumpFile = serde_json::from_slice(bytes)?;
        if file.version != KNOWN_VERSION {
            anyhow::bail!(
                "entity_models.json: unsupported version {} (expected {KNOWN_VERSION})",
                file.version
            );
        }
        let mut layers = AHashMap::with_capacity(file.layers.len());
        for l in file.layers {
            let model = CubeModel {
                texture: Identifier::new("haste", "entity_box"),
                tex_width: l.texture_width.max(1),
                tex_height: l.texture_height.max(1),
                root: ModelPart {
                    name: "root".to_owned(),
                    pivot: glam::Vec3::ZERO,
                    rotation: glam::Vec3::ZERO,
                    visible: true,
                    cubes: Vec::new(),
                    children: l.parts.into_iter().map(convert_root_part).collect(),
                },
            };
            layers.insert(
                l.layer,
                Layer {
                    model: Arc::new(model),
                    root_scale: 1.0,
                },
            );
        }
        Ok(ModelLibrary { layers })
    }

    /// The layer with `id`, with `root_scale` applied on top of its baked geometry (see the
    /// module doc). `None` when the dump is absent or does not name the layer.
    pub fn get(&self, id: &str, root_scale: f32) -> Option<Arc<CubeModel>> {
        let layer = self.layers.get(id)?;
        // Scale 1 is the common case: hand back the shared model untouched.
        if (root_scale - 1.0).abs() < 1e-6 {
            return Some(layer.model.clone());
        }
        Some(Arc::new(scaled_model(&layer.model, root_scale)))
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// Every layer id in the dump, sorted; for tests and the `list_layers` example.
    pub fn layer_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.layers.keys().map(|s| s.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

/// Converts one of the root's direct children: its pivot is in absolute model space, where the
/// ground plane sits at y = 24 model units, so the y is shifted to the feet-at-0 convention.
/// Every part below it keeps its parent-relative pivot unchanged (vanilla `PartPose` of a child
/// is relative to its parent, and the dump already stored it that way).
fn convert_root_part(p: PartJson) -> ModelPart {
    let mut part = convert_part(p);
    part.pivot.y -= GROUND_Y;
    part
}

/// Converts one dump part (rotations are already radians in vanilla x/y/z order). A part scale
/// other than 1 is folded into its cubes' inflate as a mean and skipped for the pivot: vanilla
/// uses part scales almost exclusively for whole-part effects (spinning arrows, end-crystal
/// cores) that rest-pose rendering does not need to reproduce exactly.
fn convert_part(p: PartJson) -> ModelPart {
    let pivot = glam::Vec3::new(p.pivot[0], p.pivot[1], p.pivot[2]);
    let rotation = glam::Vec3::new(p.rotation[0], p.rotation[1], p.rotation[2]);
    let uniform = (p.scale[0] + p.scale[1] + p.scale[2]) / 3.0;
    let cubes = p
        .cubes
        .into_iter()
        .map(|c| Cube {
            origin: glam::Vec3::new(c.origin[0], c.origin[1], c.origin[2]),
            size: glam::Vec3::new(c.size[0], c.size[1], c.size[2]),
            inflate: (c.inflate[0] + c.inflate[1] + c.inflate[2]) / 3.0 * uniform,
            uv: [c.u.max(0.0) as u32, c.v.max(0.0) as u32],
            mirror: c.mirror,
        })
        .collect();
    ModelPart {
        name: p.name,
        pivot,
        rotation,
        visible: true,
        cubes,
        children: p.children.into_iter().map(convert_part).collect(),
    }
}

/// Returns a copy of `model` scaled by `factor` about the feet (uniform root scale, like
/// vanilla's `MeshTransformer.scaling`: pivots and cube sizes scale, y keeps the ground at 0).
fn scaled_model(model: &CubeModel, factor: f32) -> CubeModel {
    fn scaled_part(p: &ModelPart, factor: f32) -> ModelPart {
        ModelPart {
            name: p.name.clone(),
            pivot: p.pivot * factor,
            rotation: p.rotation,
            visible: p.visible,
            cubes: p
                .cubes
                .iter()
                .map(|c| Cube {
                    origin: c.origin * factor,
                    size: c.size * factor,
                    inflate: c.inflate * factor,
                    uv: c.uv,
                    mirror: c.mirror,
                })
                .collect(),
            children: p.children.iter().map(|c| scaled_part(c, factor)).collect(),
        }
    }
    CubeModel {
        texture: model.texture.clone(),
        tex_width: model.tex_width,
        tex_height: model.tex_height,
        root: scaled_part(&model.root, factor),
    }
}

/// The process-wide library, loaded from `haste_core::paths::oracle_dir()` on first use.
/// A missing dump is logged once here and means `None` forever after (boxes for everything).
pub fn library() -> Option<&'static ModelLibrary> {
    static LIB: OnceLock<Option<ModelLibrary>> = OnceLock::new();
    LIB.get_or_init(|| {
        let path = haste_core::paths::oracle_dir().join("entity_models.json");
        match std::fs::read(&path) {
            Ok(bytes) => match ModelLibrary::parse(&bytes) {
                Ok(l) => {
                    tracing::info!(
                        "entity model library: {} layers from {}",
                        l.len(),
                        path.display()
                    );
                    Some(l)
                }
                Err(e) => {
                    tracing::warn!(
                        "entity model library: {}: {e:#}; using boxes",
                        path.display()
                    );
                    None
                }
            },
            Err(_) => {
                tracing::warn!(
                    "entity model library: {} missing (recorder mod dump); using boxes",
                    path.display()
                );
                None
            }
        }
    })
    .as_ref()
}
