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
    /// State id -> quads (None for states without models, e.g. air). States whose
    /// blockstates resolve to the same model refs share one `Arc` (bake dedup).
    quads: Vec<Option<Arc<[BakedQuad]>>>,
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
    /// Heap bytes of the baked quads and per-state tables (live allocations; the
    /// `String` texture ids count their capacity). Shared `Arc`s count once.
    pub fn memory_bytes(&self) -> u64 {
        let mut seen: Vec<*const [BakedQuad]> = Vec::new();
        let mut quads: u64 = 0;
        for q in &self.quads {
            let Some(q) = q else { continue };
            let ptr: *const [BakedQuad] = Arc::as_ptr(q);
            if seen.contains(&ptr) {
                continue;
            }
            seen.push(ptr);
            quads += q.len() as u64 * std::mem::size_of::<BakedQuad>() as u64
                + q.iter()
                    .map(|q| q.texture.capacity() as u64 + 24)
                    .sum::<u64>();
        }
        quads
            + self.quads.capacity() as u64 * std::mem::size_of::<Option<Arc<[BakedQuad]>>>() as u64
            + (self.full_opaque.capacity() + self.layer.capacity()) as u64
    }

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
        let mut quads: Vec<Option<Arc<[BakedQuad]>>> = vec![None; n];
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
        // the same variant key, and the UV assignment depends only on the
        // referenced textures, so the cache holds the finished quads and every
        // state with the same key shares one allocation.
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
                    let mut state_quads: Vec<BakedQuad> = bake_refs(&refs, &resolved_models)?;
                    for quad in &mut state_quads {
                        let region = atlas.region_of(&quad.texture);
                        assign_atlas_uvs(quad, region);
                    }
                    let baked: Arc<[BakedQuad]> = state_quads.into();
                    baked_cache.insert(cache_key, baked.clone());
                    baked
                }
            };
            if !baked.is_empty() {
                full_opaque[state as usize] = is_full_opaque(&baked, atlas);
                layer[state as usize] = classify_layer(&baked, block, atlas);
                quads[state as usize] = Some(baked);
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
            .and_then(|q| q.as_deref())
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
                // Cardinal-lighting face: the element's `shade_direction_override`
                // (vanilla `BakedQuad.MaterialInfo.shadeDirectionOverride`, e.g.
                // the cross models' "up") wins over the derived facing; the
                // legacy `shade: false` (dropped in 26.3) keeps its old meaning.
                let shade_face = el.shade_direction_override.or_else(|| {
                    el.shade
                        .then(|| direction_from_positions(&positions).unwrap_or(Direction::Up))
                });
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
mod tests;
