// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/world/block/model.rs (bake_resolved_model), reused under
// GPL-3.0 and reworked for Haste: bakes per BlockStateId against a block-state
// source (haste-data's BlockStates implements BlockStateTable), atlas UV
// assignment, and a render-layer heuristic (26.3 has no render_type field in
// block models; see NOTES.md). See THIRD_PARTY.md.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use serde::Deserialize;

use haste_core::{BlockStateId, Identifier};

use crate::assets::Assets;
use crate::atlas::{AtlasBuilder, AtlasRegions};
use crate::model::ModelRef;
use crate::model::{
    apply_element_rotation, direction_from_positions, face_positions, face_uvs, rotate_positions,
    BakedQuad, Direction, ResolvedModel,
};

/// Render layers, in draw order (solid first, translucent last, distance-sorted).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RenderLayer {
    Solid,
    Cutout,
    CutoutMipped,
    Translucent,
}

/// `datagen/26.3/reports/blocks.json` as a
/// `haste_core::view::BlockStateTable` (same shape as haste-data's loader;
/// independent of the haste-data crate for tests and until it lands).
/// (block name, properties, parsed id), indexed by state id.
type JsonState = (String, Vec<(String, String)>, Option<Identifier>);

pub struct BlocksJsonStates {
    states: Vec<JsonState>,
    /// Placeholder for ids outside the table: `minecraft:stone`'s default state id.
    /// Vanilla has no "missing block"; a modded server can send ids past the end of
    /// the table, and every lookup resolves them here.
    placeholder: BlockStateId,
}

impl BlocksJsonStates {
    pub fn load(path: &std::path::Path) -> Result<BlocksJsonStates> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let blocks: HashMap<String, BlocksJsonBlock> =
            serde_json::from_slice(&bytes).with_context(|| "parsing blocks.json")?;
        let mut states: Vec<JsonState> = Vec::new();
        let mut by_id: Vec<StateEntry> = Vec::new();
        let mut placeholder = None;
        for (name, block) in &blocks {
            for state in &block.states {
                let props = state
                    .properties
                    .as_ref()
                    .map(|p| {
                        let mut kv: Vec<(String, String)> =
                            p.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                        // The properties come from a HashMap; sort them so a state's
                        // property list is the same in every process.
                        kv.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                        kv
                    })
                    .unwrap_or_default();
                by_id.push((state.id as usize, name.clone(), props));
                if name == "minecraft:stone" && state.default && placeholder.is_none() {
                    placeholder = Some(state.id);
                }
            }
        }
        by_id.sort_by_key(|(id, _, _)| *id);
        states.reserve(by_id.len());
        for (i, (id, name, props)) in by_id.into_iter().enumerate() {
            debug_assert_eq!(id, i, "blocks.json state ids must be dense");
            let ident = Identifier::parse(&name).ok();
            states.push((name, props, ident));
        }
        Ok(BlocksJsonStates {
            states,
            // No stone in a hand-written table: fall back to air (0).
            placeholder: BlockStateId(placeholder.unwrap_or(0)),
        })
    }
}

impl haste_core::view::BlockStateTable for BlocksJsonStates {
    fn len(&self) -> usize {
        self.states.len()
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn placeholder_state(&self) -> BlockStateId {
        self.placeholder
    }
    fn block_of(&self, s: BlockStateId) -> &Identifier {
        self.states[s.0 as usize]
            .2
            .as_ref()
            .expect("blocks.json names are valid ids")
    }
    fn properties(&self, s: BlockStateId) -> Vec<(String, String)> {
        self.states[s.0 as usize].1.clone()
    }
    fn property_value(&self, s: BlockStateId, name: &str) -> Option<&str> {
        self.states[s.0 as usize]
            .1
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

type StateEntry = (usize, String, Vec<(String, String)>);

#[derive(Deserialize)]
struct BlocksJsonBlock {
    states: Vec<BlocksJsonState>,
}

#[derive(Deserialize)]
struct BlocksJsonState {
    id: u32,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    properties: Option<HashMap<String, String>>,
}

/// Per-block-state baked geometry, indexed by BlockStateId.
pub struct BakedBlockModels {
    /// State id -> quads (empty for states without models, e.g. air).
    quads: Vec<Vec<BakedQuad>>,
    /// State id -> full opaque cube flag (face culling in the mesher).
    full_opaque: Vec<bool>,
    /// State id -> render layer.
    layer: Vec<RenderLayer>,
    /// Placeholder for ids outside the table (modded servers): out-of-range lookups
    /// resolve to this state's entry, so unknown blocks render as the placeholder
    /// (stone) and match their collision. Set by `bake` from the table's
    /// `placeholder_state` — that is how it gets here.
    placeholder: BlockStateId,
}

impl BakedBlockModels {
    /// Bakes every state of every block in `states`, assigning atlas UVs. The
    /// atlas is rebuilt from every texture reachable from any blockstate file
    /// (all variants and all multipart applies). Blocks without a blockstate
    /// JSON or with unresolvable models bake to no quads (invisible), matching
    /// vanilla's missing-model handling closely enough for first frame milestone.
    pub fn bake(
        assets: &mut Assets,
        states: &dyn haste_core::view::BlockStateTable,
        atlas: &mut AtlasBuilder,
    ) -> Result<BakedBlockModels> {
        let n = states.len();
        // Out-of-range ids (modded servers) resolve to the placeholder state's entry.
        let placeholder = states.placeholder_state();
        let mut quads: Vec<Vec<BakedQuad>> = vec![Vec::new(); n];
        let mut full_opaque = vec![false; n];
        let mut layer = vec![RenderLayer::Solid; n];

        let mut block_names: Vec<Identifier> = assets.blockstate_names();
        block_names.sort();

        // Pass 1: every model ref reachable from each blockstate file, resolved
        // once, collecting textures (Pomme resolves per variant; we need all
        // branches for texture coverage, including non-matching multipart cases)
        // and the sprites models declare force_translucent (26.3 model field).
        let mut texture_ids: Vec<Identifier> = Vec::new();
        let mut seen_textures = HashSet::new();
        let mut force_translucent: HashSet<String> = HashSet::new();
        let mut resolved_models: HashMap<String, ResolvedModel> = HashMap::new();
        for block in &block_names {
            let Ok(def) = assets.blockstate(block) else {
                continue;
            };
            for r in def.all_model_refs() {
                if resolved_models.contains_key(&r.model) {
                    continue;
                }
                let model_id = Identifier::parse(&r.model)?;
                let resolved = match assets.resolve_model(&model_id) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!("{}: model {} failed ({e:#})", block, r.model);
                        continue;
                    }
                };
                for sprite in &resolved.force_translucent {
                    force_translucent.insert(sprite.clone());
                }
                for el in &resolved.elements {
                    for face in el.faces.values() {
                        if let Some(tex) = face_texture_id(&face.texture, &resolved.textures) {
                            if let Ok(id) = Identifier::parse(&tex) {
                                if seen_textures.insert(tex) {
                                    texture_ids.push(id);
                                }
                            }
                        }
                    }
                }
                resolved_models.insert(r.model.clone(), resolved);
            }
        }

        // Fluids have no model elements; the chunk mesher draws them with these sprites.
        for tex in [
            "minecraft:block/water_still",
            "minecraft:block/water_flow",
            "minecraft:block/water_overlay",
            "minecraft:block/lava_still",
            "minecraft:block/lava_flow",
        ] {
            if seen_textures.insert(tex.to_string()) {
                texture_ids.push(Identifier::parse(tex)?);
            }
        }
        texture_ids.sort();
        *atlas = AtlasBuilder::build(assets, &texture_ids, true)?;
        // Sprites a model declares force_translucent render translucent whatever
        // their texels say (vanilla's 26.3 model field: glass panes, redstone dust);
        // classify_layer reads the region flag.
        for sprite in &force_translucent {
            if let Ok(id) = Identifier::parse(sprite) {
                let region = atlas.region_mut(&id.to_string());
                region.translucent = true;
            }
        }

        // Pass 2: per state, evaluate variants/multipart and bake with UVs.
        // Baked geometry is cached per unique (refs) tuple: many states share
        // the same variant key.
        let mut baked_cache: HashMap<String, Arc<[BakedQuad]>> = HashMap::new();
        for state in 0..n as u32 {
            let sid = BlockStateId(state);
            let block = states.block_of(sid);
            let Ok(def) = assets.blockstate(block) else {
                continue;
            };
            let props = states.properties(sid);
            let refs = def.model_refs_for(&props);
            if refs.is_empty() {
                continue;
            }
            let cache_key = refs_cache_key(&refs);
            let baked = match baked_cache.get(&cache_key) {
                Some(b) => b.clone(),
                None => {
                    let quads = bake_refs(&refs, &resolved_models)?;
                    let baked: Arc<[BakedQuad]> = quads.into();
                    baked_cache.insert(cache_key, baked.clone());
                    baked
                }
            };
            // Clone with atlas UVs assigned (UVs depend on the region, shared
            // per texture, so the cache could hold them too; keep it simple).
            let mut state_quads: Vec<BakedQuad> = baked.iter().cloned().collect();
            for quad in &mut state_quads {
                let region = atlas.region_of(&quad.texture);
                assign_atlas_uvs(quad, region);
            }
            if !state_quads.is_empty() {
                full_opaque[state as usize] = is_full_opaque(&state_quads, atlas);
                layer[state as usize] = classify_layer(&state_quads, block, atlas);
                quads[state as usize] = state_quads;
            }
        }

        Ok(BakedBlockModels {
            quads,
            full_opaque,
            layer,
            placeholder,
        })
    }

    /// Table row for `state`: the id itself when in range, else the placeholder's row.
    /// One bounds check, no allocation.
    fn row(&self, s: BlockStateId) -> usize {
        let id = s.0 as usize;
        if id < self.quads.len() {
            id
        } else {
            self.placeholder.0 as usize
        }
    }

    /// Quads of `s`; ids outside the table return the placeholder state's quads (stone),
    /// so unknown blocks are visible and match their collision.
    pub fn quads(&self, s: BlockStateId) -> &[BakedQuad] {
        self.quads
            .get(self.row(s))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn is_full_opaque(&self, s: BlockStateId) -> bool {
        self.full_opaque.get(self.row(s)).copied().unwrap_or(false)
    }

    pub fn render_layer(&self, s: BlockStateId) -> RenderLayer {
        self.layer
            .get(self.row(s))
            .copied()
            .unwrap_or(RenderLayer::Solid)
    }
}

/// Face texture reference through the resolved texture map. Vanilla allows
/// referring to a texture variable WITHOUT the `#` prefix (the bare name must
/// exist in the model's `textures` map, e.g. heavy_core's faces use `all`).
pub(crate) fn face_texture_id(
    face_texture: &str,
    textures: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let var = face_texture.strip_prefix('#').unwrap_or(face_texture);
    if let Some(resolved) = textures.get(var) {
        return Some(normalize_texture_id(resolved));
    }
    if face_texture.starts_with('#') || face_texture.contains(':') || face_texture.contains('/') {
        // A `#ref` that did not resolve, or a direct texture id
        // (`minecraft:block/x`): use it as written.
        Some(normalize_texture_id(face_texture))
    } else {
        None
    }
}

/// Texture references in models may omit the namespace (`block/grass_block_side`); the atlas
/// is keyed by full identifiers (`minecraft:block/grass_block_side`).
pub(crate) fn normalize_texture_id(t: &str) -> String {
    if t.starts_with('#') || t.contains(':') {
        t.to_string()
    } else {
        format!("minecraft:{t}")
    }
}

fn refs_cache_key(refs: &[ModelRef]) -> String {
    let mut key = String::new();
    for r in refs {
        key.push_str(&r.model);
        key.push(':');
        key.push_str(&r.x.to_string());
        key.push(':');
        key.push_str(&r.y.to_string());
        key.push(':');
        key.push_str(&r.uvlock.to_string());
        key.push(';');
    }
    key
}

/// Bake the geometry of a list of model refs (Pomme `bake_resolved_model` loop,
/// without per-block tints: tint resolution happens in the mesher via tintindex).
fn bake_refs(
    refs: &[ModelRef],
    resolved: &HashMap<String, ResolvedModel>,
) -> Result<Vec<BakedQuad>> {
    let mut quads = Vec::new();
    for r in refs {
        let Some(res) = resolved.get(&r.model) else {
            continue;
        };
        for el in &res.elements {
            let from = [el.from[0] / 16.0, el.from[1] / 16.0, el.from[2] / 16.0];
            let to = [el.to[0] / 16.0, el.to[1] / 16.0, el.to[2] / 16.0];
            // `faces` is a BTreeMap keyed by direction in vanilla's order, so the
            // quads of an element always come out in the same order.
            for (dir, face) in &el.faces {
                let Some(tex) = face_texture_id(&face.texture, &res.textures) else {
                    continue;
                };
                let positions = face_positions(*dir, from, to);
                let uvs = face_uvs(*dir, from, to, face.uv.as_ref(), face.rotation);
                let mut positions = apply_element_rotation(positions, &el.rotation);
                let mut cullface = face.cullface.as_deref().and_then(Direction::parse);
                if r.x != 0 || r.y != 0 {
                    positions = rotate_positions(positions, r.x, r.y);
                    cullface = cullface.map(|d| d.rotate_x(r.x).rotate_y(r.y));
                }
                let shade_face = el
                    .shade
                    .then(|| direction_from_positions(&positions).unwrap_or(Direction::Up));
                quads.push(BakedQuad {
                    positions,
                    uvs,
                    texture: tex,
                    cullface,
                    shade_face,
                    tint_index: face.tint_index.and_then(|i| u32::try_from(i).ok()),
                    direction: *dir,
                });
            }
        }
    }
    Ok(quads)
}

/// Map the quad's face-local 0..1 UVs into its texture's atlas region.
fn assign_atlas_uvs(quad: &mut BakedQuad, region: crate::atlas::AtlasRegion) {
    let [x, y, w, h] = region.pixel_rect;
    for uv in &mut quad.uvs {
        uv[0] = x as f32 + uv[0] * w as f32;
        uv[1] = y as f32 + uv[1] * h as f32;
    }
}

/// Full opaque cube: every one of the six faces is covered by a full-size quad with that
/// cullface and a fully opaque texture. Extra quads (the grass block's tinted side overlay)
/// do not change that: they sit on top of an already opaque face.
fn is_full_opaque(quads: &[BakedQuad], atlas: &dyn AtlasRegions) -> bool {
    let mut dirs = [false; 6];
    for q in quads {
        let Some(cull) = q.cullface else { continue };
        let full = q.positions.iter().all(|pos| {
            pos.iter()
                .all(|&v| v.abs() < 1e-4 || (v - 1.0).abs() < 1e-4)
        });
        if full && atlas.region_of(&q.texture).opaque {
            dirs[cull.index()] = true;
        }
    }
    dirs.iter().all(|&d| d)
}

/// Render layer heuristic. 26.3 block models carry no `render_type` field, so
/// the layer comes from the known vanilla `ItemBlockRenderTypes` sets (by block
/// name) with a texture-alpha fallback following vanilla's per-sprite rule
/// (`NativeImage.computeTransparency` -> `ChunkSectionLayer.byTransparency`):
/// any sprite with partial-alpha texels -> translucent, any with alpha-0
/// texels -> cutout, else solid. Documented as approximate in NOTES.md.
fn classify_layer(
    quads: &[BakedQuad],
    block: &Identifier,
    atlas: &dyn AtlasRegions,
) -> RenderLayer {
    let path = block.path();
    let name = path.rsplit('/').next().unwrap_or(path);
    let translucent = matches!(
        name,
        "glass"
            | "ice"
            | "water"
            | "nether_portal"
            | "end_portal"
            | "slime_block"
            | "honey_block"
            | "barrier"
            | "tinted_glass"
            | "beacon_beam"
    ) || name.ends_with("_glass")
        || name.ends_with("_glass_pane");
    if translucent {
        return RenderLayer::Translucent;
    }
    if name.ends_with("_leaves")
        || matches!(
            name,
            "mangrove_roots" | "vine" | "glow_lichen" | "sculk_vein" | "pale_oak_leaves"
        )
    {
        return RenderLayer::CutoutMipped;
    }
    if quads
        .iter()
        .any(|q| atlas.region_of(&q.texture).translucent)
    {
        return RenderLayer::Translucent;
    }
    if quads.iter().any(|q| !atlas.region_of(&q.texture).opaque) {
        return RenderLayer::Cutout;
    }
    RenderLayer::Solid
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
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
            head_display: None,
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
            quads: vec![Vec::new(), vec![quad.clone()]],
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
}
