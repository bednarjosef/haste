// Derived from Pomme (GPL-3.0); see THIRD_PARTY.md.

use std::collections::HashMap;
use std::sync::Arc;

use crate::model::{ModelFile, ResolvedModel};
use crate::multipart::BlockStateDefinition;
use anyhow::{Context, Result};
use haste_core::Identifier;

use super::Assets;

impl Assets {
    /// `blockstates/<path>.json` of a block, variants and multipart as written.
    pub fn blockstate(&mut self, block: &Identifier) -> Result<BlockStateDefinition> {
        let key = super::asset_key(block, "blockstates", ".json");
        let bytes = self.read_asset(&key)?;
        let def: BlockStateDefinition =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        Ok(def)
    }

    /// `models/<path>.json` with the `parent` chain resolved: `textures` variables
    /// (`#all` etc.) resolved to texture identifiers, elements inherited from the
    /// nearest ancestor that has any, `ambientocclusion` inherited (default true).
    pub fn model(&mut self, id: &Identifier) -> Result<ModelJson> {
        let resolved = self.resolve_model(id)?;
        Ok(ModelJson {
            textures: resolved.textures,
            elements: resolved.elements,
            ambientocclusion: resolved.ambientocclusion,
            gui_light: resolved.gui_light,
        })
    }

    /// All `assets/minecraft/blockstates/*.json` block names (paths without the
    /// directory and extension). For `BakedBlockModels::bake`.
    pub fn blockstate_names(&mut self) -> Vec<Identifier> {
        self.names_under("assets/minecraft/blockstates/", ".json")
            .map(|p| Identifier::minecraft(&p))
            .collect()
    }

    pub(crate) fn resolve_model(&mut self, id: &Identifier) -> Result<ResolvedModel> {
        let mut texture_map: HashMap<String, String> = HashMap::new();
        // Sprite ids any model in the parent chain declares force_translucent; their
        // atlas regions are marked translucent after the build (26.3 model field).
        let mut force_translucent: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut elements: Option<Vec<crate::model::ElementDef>> = None;
        let mut ambientocclusion: Option<bool> = None;
        let mut gui_light: Option<bool> = None;
        let mut gui_display: Option<crate::model::DisplayTransform> = None;
        let mut firstperson_display: Option<crate::model::DisplayTransform> = None;
        let mut thirdperson_display: Option<crate::model::DisplayTransform> = None;
        let mut thirdperson_left_display: Option<crate::model::DisplayTransform> = None;
        let mut head_display: Option<crate::model::DisplayTransform> = None;
        let mut ground_display: Option<crate::model::DisplayTransform> = None;
        let mut fixed_display: Option<crate::model::DisplayTransform> = None;
        let mut on_shelf_display: Option<crate::model::DisplayTransform> = None;
        let mut current = id.clone();

        for _ in 0..20 {
            let model = self.load_model(&current)?;
            for sprite in &model.textures.force_translucent {
                force_translucent.insert(sprite.clone());
            }
            for (key, value) in &model.textures {
                texture_map
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            if elements.is_none() && !model.elements.is_empty() {
                elements = Some(model.elements.clone());
            }
            if ambientocclusion.is_none() && model.ambientocclusion.is_some() {
                ambientocclusion = model.ambientocclusion;
            }
            if firstperson_display.is_none() {
                firstperson_display = model
                    .display
                    .get("firstperson_righthand")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if gui_display.is_none() {
                gui_display = model
                    .display
                    .get("gui")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if thirdperson_display.is_none() {
                thirdperson_display = model
                    .display
                    .get("thirdperson_righthand")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            // `ItemTransforms.getTransform(THIRD_PERSON_LEFT_HAND)`: the file's left entry,
            // falling back to its own right entry when it names none (then the left-hand fix
            // is applied on top at draw time; the entry and the fix are not the same thing).
            if thirdperson_left_display.is_none() {
                thirdperson_left_display = model
                    .display
                    .get("thirdperson_lefthand")
                    .and_then(crate::model::DisplayTransform::parse)
                    .or_else(|| {
                        model
                            .display
                            .get("thirdperson_righthand")
                            .and_then(crate::model::DisplayTransform::parse)
                    });
            }
            if head_display.is_none() {
                head_display = model
                    .display
                    .get("head")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if ground_display.is_none() {
                ground_display = model
                    .display
                    .get("ground")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if fixed_display.is_none() {
                fixed_display = model
                    .display
                    .get("fixed")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if on_shelf_display.is_none() {
                on_shelf_display = model
                    .display
                    .get("on_shelf")
                    .and_then(crate::model::DisplayTransform::parse);
            }
            if gui_light.is_none() && model.gui_light.is_some() {
                gui_light = Some(model.gui_light.as_deref() == Some("front"));
            }
            match &model.parent {
                // `builtin/generated` (flat items) and `builtin/entity` have no file.
                Some(parent)
                    if parent
                        .trim_start_matches("minecraft:")
                        .starts_with("builtin/") =>
                {
                    break
                }
                Some(parent) => current = Identifier::parse(parent)?,
                None => break,
            }
        }

        // Resolve `#ref` texture variables (bounded depth, like Pomme's resolve_ref).
        let mut resolved = HashMap::with_capacity(texture_map.len());
        for (key, value) in &texture_map {
            resolved.insert(key.clone(), resolve_ref(value, &texture_map, 0));
        }

        Ok(ResolvedModel {
            textures: resolved,
            force_translucent,
            elements: elements.unwrap_or_default(),
            ambientocclusion: ambientocclusion.unwrap_or(true),
            gui_light: gui_light.unwrap_or(false),
            gui_display,
            firstperson_display,
            thirdperson_display,
            thirdperson_left_display,
            head_display,
            ground_display,
            fixed_display,
            on_shelf_display,
        })
    }

    pub(crate) fn load_model(&mut self, id: &Identifier) -> Result<Arc<ModelFile>> {
        if let Some(cached) = self.model_cache.get(&id.to_string()) {
            return Ok(cached.clone());
        }
        let key = super::asset_key(id, "models", ".json");
        let bytes = self.read_asset(&key)?;
        let model: ModelFile =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        let model = Arc::new(model);
        self.model_cache.insert(id.to_string(), model.clone());
        Ok(model)
    }
}

/// Texture references resolve through `#variable` chains with bounded depth
/// (Pomme `resolve_ref`, vanilla `Material` resolution).
fn resolve_ref(value: &str, map: &HashMap<String, String>, depth: u32) -> String {
    if depth > 10 {
        return value.to_string();
    }
    if let Some(ref_name) = value.strip_prefix('#') {
        if let Some(target) = map.get(ref_name) {
            return resolve_ref(target, map, depth + 1);
        }
    }
    value.to_string()
}

/// The model with parent chain resolved, as returned by `Assets::model`.
/// (Pomme's `ResolvedModel` shape, restricted to what block baking needs.)
#[derive(Clone, Debug)]
pub struct ModelJson {
    /// Texture variable name (`all`, `up`, ...) -> resolved texture id.
    pub textures: HashMap<String, String>,
    pub elements: Vec<crate::model::ElementDef>,
    pub ambientocclusion: bool,
    /// `gui_light`: true = `front` (flat items). Only used by item rendering later.
    pub gui_light: bool,
}
