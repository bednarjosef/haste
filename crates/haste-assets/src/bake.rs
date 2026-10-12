// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/world/block/model.rs (bake_resolved_model), reused under
// GPL-3.0 and reworked for Haste: bakes per BlockStateId against a block-state
// source (haste-data's BlockStates implements `BlockStatesSource`), atlas UV
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

/// The block-state queries baking needs. `haste_data::BlockStates` (WS2) is the
/// production implementation; `BlocksJsonStates` below reads blocks.json directly
/// for tests and until haste-data lands.
pub trait BlockStatesSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn block_of(&self, s: BlockStateId) -> Identifier;
    fn properties(&self, s: BlockStateId) -> Vec<(String, String)>;
}

/// `datagen/26.3/reports/blocks.json` as a [BlockStatesSource] (same shape as
/// haste-data's loader; independent of the haste-data crate).
pub struct BlocksJsonStates {
    /// State id -> (block name, properties).
    states: Vec<(String, Vec<(String, String)>)>,
}

impl BlocksJsonStates {
    pub fn load(path: &std::path::Path) -> Result<BlocksJsonStates> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let blocks: HashMap<String, BlocksJsonBlock> =
            serde_json::from_slice(&bytes).with_context(|| "parsing blocks.json")?;
        let mut states: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut by_id: Vec<StateEntry> = Vec::new();
        for (name, block) in &blocks {
            for state in &block.states {
                let props = state
                    .properties
                    .as_ref()
                    .map(|p| p.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                by_id.push((state.id as usize, name.clone(), props));
            }
        }
        by_id.sort_by_key(|(id, _, _)| *id);
        states.reserve(by_id.len());
        for (i, (id, name, props)) in by_id.into_iter().enumerate() {
            debug_assert_eq!(id, i, "blocks.json state ids must be dense");
            states.push((name, props));
        }
        Ok(BlocksJsonStates { states })
    }
}

impl BlockStatesSource for BlocksJsonStates {
    fn len(&self) -> usize {
        self.states.len()
    }
    fn block_of(&self, s: BlockStateId) -> Identifier {
        Identifier::parse(&self.states[s.0 as usize].0).expect("blocks.json names are valid ids")
    }
    fn properties(&self, s: BlockStateId) -> Vec<(String, String)> {
        self.states[s.0 as usize].1.clone()
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
}

impl BakedBlockModels {
    /// Bakes every state of every block in `states`, assigning atlas UVs. The
    /// atlas is rebuilt from every texture reachable from any blockstate file
    /// (all variants and all multipart applies). Blocks without a blockstate
    /// JSON or with unresolvable models bake to no quads (invisible), matching
    /// vanilla's missing-model handling closely enough for M2.
    pub fn bake(
        assets: &mut Assets,
        states: &dyn BlockStatesSource,
        atlas: &mut AtlasBuilder,
    ) -> Result<BakedBlockModels> {
        let n = states.len();
        let mut quads: Vec<Vec<BakedQuad>> = vec![Vec::new(); n];
        let mut full_opaque = vec![false; n];
        let mut layer = vec![RenderLayer::Solid; n];

        let mut block_names: Vec<Identifier> = assets.blockstate_names();
        block_names.sort();

        // Pass 1: every model ref reachable from each blockstate file, resolved
        // once, collecting textures (Pomme resolves per variant; we need all
        // branches for texture coverage, including non-matching multipart cases).
        let mut texture_ids: Vec<Identifier> = Vec::new();
        let mut seen_textures = HashSet::new();
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

        texture_ids.sort();
        *atlas = AtlasBuilder::build(assets, &texture_ids, true)?;

        // Pass 2: per state, evaluate variants/multipart and bake with UVs.
        // Baked geometry is cached per unique (refs) tuple: many states share
        // the same variant key.
        let mut baked_cache: HashMap<String, Arc<[BakedQuad]>> = HashMap::new();
        for state in 0..n as u32 {
            let sid = BlockStateId(state);
            let block = states.block_of(sid);
            let Ok(def) = assets.blockstate(&block) else {
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
                layer[state as usize] = classify_layer(&state_quads, &block, atlas);
                quads[state as usize] = state_quads;
            }
        }

        Ok(BakedBlockModels {
            quads,
            full_opaque,
            layer,
        })
    }

    pub fn quads(&self, s: BlockStateId) -> &[BakedQuad] {
        self.quads
            .get(s.0 as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn is_full_opaque(&self, s: BlockStateId) -> bool {
        self.full_opaque.get(s.0 as usize).copied().unwrap_or(false)
    }

    pub fn render_layer(&self, s: BlockStateId) -> RenderLayer {
        self.layer
            .get(s.0 as usize)
            .copied()
            .unwrap_or(RenderLayer::Solid)
    }
}

/// Face texture reference through the resolved texture map. Vanilla allows
/// referring to a texture variable WITHOUT the `#` prefix (the bare name must
/// exist in the model's `textures` map, e.g. heavy_core's faces use `all`).
fn face_texture_id(
    face_texture: &str,
    textures: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let var = face_texture.strip_prefix('#').unwrap_or(face_texture);
    if let Some(resolved) = textures.get(var) {
        return Some(resolved.clone());
    }
    if face_texture.starts_with('#') || face_texture.contains(':') || face_texture.contains('/') {
        // A `#ref` that did not resolve, or a direct texture id
        // (`minecraft:block/x`): use it as written.
        Some(face_texture.to_string())
    } else {
        None
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
            for (face_name, face) in &el.faces {
                let Some(dir) = Direction::parse(face_name) else {
                    continue;
                };
                let Some(tex) = face_texture_id(&face.texture, &res.textures) else {
                    continue;
                };
                let positions = face_positions(dir, from, to);
                let uvs = face_uvs(dir, from, to, face.uv.as_ref(), face.rotation);
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
                    direction: dir,
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

/// Full opaque cube: exactly six cullfaces, each spanning the full block face,
/// all textures fully opaque (Pomme `check_full_cube` + atlas alpha).
fn is_full_opaque(quads: &[BakedQuad], atlas: &dyn AtlasRegions) -> bool {
    if quads.len() != 6 {
        return false;
    }
    let mut dirs = [false; 6];
    for q in quads {
        let Some(cull) = q.cullface else { return false };
        dirs[cull.index()] = true;
        for pos in &q.positions {
            if pos
                .iter()
                .any(|&v| (v - 0.0).abs() > 1e-4 && (v - 1.0).abs() > 1e-4)
            {
                return false;
            }
        }
        if !atlas.region_of(&q.texture).opaque {
            return false;
        }
    }
    dirs.iter().all(|&d| d)
}

/// Render layer heuristic. 26.3 block models carry no `render_type` field, so
/// the layer comes from the known vanilla `ItemBlockRenderTypes` sets (by block
/// name) with a texture-alpha fallback: any non-opaque texture -> cutout.
/// Documented as approximate in NOTES.md.
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

    struct TestRegions {
        opaque: bool,
    }

    impl AtlasRegions for TestRegions {
        fn region_of(&self, _texture_id: &str) -> AtlasRegion {
            AtlasRegion {
                pixel_rect: [0, 0, 16, 16],
                sprite: 1,
                opaque: self.opaque,
                translucent: !self.opaque,
            }
        }
    }

    fn quad() -> BakedQuad {
        BakedQuad::from_element(Direction::Up, [0.0; 3], [1.0; 3], None, None)
    }

    #[test]
    fn classify_layer_by_block_name_and_alpha() {
        let opaque = TestRegions { opaque: true };
        let transparent = TestRegions { opaque: false };
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
                &transparent
            ),
            RenderLayer::Cutout
        );
    }

    #[test]
    fn full_opaque_requires_six_full_faces() {
        let opaque = TestRegions { opaque: true };
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
        let transparent = TestRegions { opaque: false };
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
