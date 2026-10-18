//! The item particle sprites (vanilla `BreakingItemParticle.ItemParticleProvider
//! .getSprite` -> `ItemStackRenderState.pickParticleMaterial`): the model layers an
//! item's `minecraft:item` particles draw from, resolved from the item definition of a
//! default stack and packed into the shared particle atlas like the block sprites, so
//! they draw in the same batched pass.
//!
//! A leaf model's particle sprite is its `particle` texture (element models: block items
//! like cake); a `builtin/generated` model instead contributes one layer per `layerN`
//! texture (flat items like bread) — vanilla's `Material.Baked` per render-state layer.
//! `pickParticleMaterial` picks one active layer at random, so the table keeps them all.
//! Items with no model (special renderers), unreadable definitions or textures beyond
//! the packed set draw the missing sprite, vanilla's fallback.

use haste_assets::{Assets, AtlasRegions};
use haste_core::{Identifier, RegistryIndex};

/// The per-item particle sprites: the model layers' atlas rectangles by `minecraft:item`
/// registry index, plus the missing sprite's rect (the fallback for unknown indices).
#[derive(Clone, Debug, Default)]
pub(crate) struct ItemParticleTable {
    /// Sprite rectangles per layer, by registry index (an empty entry: no model).
    sprites: Vec<Vec<[u16; 4]>>,
    /// The atlas's missing sprite, for indices past the table.
    missing: [[u16; 4]; 1],
}

impl ItemParticleTable {
    /// The layers' sprite rectangles of item `i` (the missing sprite's single layer for
    /// unknown indices, vanilla's missing-sprite fallback).
    pub(crate) fn sprites(&self, i: RegistryIndex) -> &[[u16; 4]] {
        match self.sprites.get(i.0 as usize) {
            Some(layers) if !layers.is_empty() => layers,
            _ => &self.missing,
        }
    }

    /// A table over `items`' layers, whose texture ids `atlas` placed (unknown textures
    /// resolve to its missing region).
    pub(crate) fn build(
        items: &[(RegistryIndex, Identifier)],
        layers: &[Vec<String>],
        atlas: &dyn AtlasRegions,
        missing: [u16; 4],
    ) -> ItemParticleTable {
        let len = items
            .iter()
            .map(|(i, _)| i.0 as usize + 1)
            .max()
            .unwrap_or(0);
        let mut sprites = vec![Vec::new(); len];
        for ((index, _), ids) in items.iter().zip(layers) {
            sprites[index.0 as usize] = ids
                .iter()
                .map(|t| atlas.region_of(t).pixel_rect.map(|v| v as u16))
                .collect();
        }
        ItemParticleTable {
            sprites,
            missing: [missing],
        }
    }
}

/// Resolves every item's particle texture ids (see the module docs): one list per entry
/// of `items`, and the distinct ids for the atlas.
pub(crate) fn item_particle_textures(
    assets: &mut Assets,
    items: &[(RegistryIndex, Identifier)],
) -> (Vec<Vec<String>>, Vec<Identifier>) {
    let mut all: Vec<Vec<String>> = Vec::with_capacity(items.len());
    let mut distinct: Vec<Identifier> = Vec::new();
    for (_, item) in items {
        let mut textures = Vec::new();
        // The item definition (`assets/<ns>/items/<path>.json`) of a default stack.
        let key = format!("assets/{}/items/{}.json", item.namespace(), item.path());
        if let Ok(bytes) = assets.read_asset(&key) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(root) = json.get("model") {
                    let mut models = Vec::new();
                    leaf_models(root, &mut models);
                    for model in models {
                        model_particle_textures(assets, &model, &mut textures);
                    }
                }
            }
        }
        for texture in &textures {
            if let Ok(id) = Identifier::parse(texture) {
                if !distinct.contains(&id) {
                    distinct.push(id);
                }
            }
        }
        all.push(textures);
    }
    (all, distinct)
}

/// The model ids a default stack's item definition resolves to (vanilla
/// `ItemModelResolver.updateForTopItem`; the same walk `haste_assets::item_model` does):
/// a `model` leaf's model, a composite's parts, a condition's `on_false`, a select's
/// fallback (or first case), a range dispatch's entry for property value 0. `special`
/// renderers (chests, shields) have no particle sprite.
pub(super) fn leaf_models(node: &serde_json::Value, out: &mut Vec<String>) {
    let kind = node
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim_start_matches("minecraft:");
    match kind {
        "model" => {
            if let Some(m) = node.get("model").and_then(|m| m.as_str()) {
                out.push(m.to_owned());
            }
        }
        "composite" => {
            for m in node
                .get("models")
                .and_then(|m| m.as_array())
                .into_iter()
                .flatten()
            {
                leaf_models(m, out);
            }
        }
        "condition" => {
            if let Some(n) = node.get("on_false") {
                leaf_models(n, out);
            }
        }
        "select" => {
            let first_case = node
                .get("cases")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("model"));
            if let Some(n) = node.get("fallback").or(first_case) {
                leaf_models(n, out);
            }
        }
        "range_dispatch" => {
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
            if let Some(n) = entry.or_else(|| node.get("fallback")) {
                leaf_models(n, out);
            }
        }
        _ => {}
    }
}

/// One leaf model's particle texture ids: its `particle` texture, else every `layerN`.
fn model_particle_textures(assets: &mut Assets, model: &str, out: &mut Vec<String>) {
    let Ok(id) = Identifier::parse(model) else {
        return;
    };
    let Ok(resolved) = assets.model(&id) else {
        return;
    };
    // Texture ids may omit the namespace (Pomme `normalize_texture_id`).
    let normalize = |t: &str| {
        if t.contains(':') {
            t.to_owned()
        } else {
            format!("minecraft:{t}")
        }
    };
    if let Some(p) = resolved.textures.get("particle") {
        out.push(normalize(p));
    } else {
        for i in 0.. {
            match resolved.textures.get(&format!("layer{i}")) {
                Some(t) => out.push(normalize(t)),
                None => break,
            }
        }
    }
}
