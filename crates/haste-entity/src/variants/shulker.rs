//! The shulker's colour texture (`ShulkerRenderer.getTextureLocation`).
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use haste_core::entity::MetadataValue;
use haste_core::model::CubeModel;
use haste_core::Identifier;

use crate::metadata::Metadata;

use super::metadata_index;
/// The base texture of a shulker (`ShulkerRenderer.getTextureLocation`): the dye's sheet
/// sprite `entity/shulker/shulker_<name>`, or the plain `shulker` when the colour byte is
/// 16 (or absent).
pub fn shulker_texture(metadata: &Metadata) -> Option<Identifier> {
    let color = match metadata
        .get(metadata_index::SHULKER_COLOR)
        .map(|e| &e.value)
    {
        Some(MetadataValue::Byte(b)) => *b,
        _ => 16,
    };
    if color == 16 || !(0..=15).contains(&color) {
        return None;
    }
    const NAMES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    Some(Identifier::minecraft(&format!(
        "entity/shulker/shulker_{}",
        NAMES[color as usize]
    )))
}

/// The shulker base model with a colour texture swapped in, cached per texture (the model is
/// cloned once per colour, never per frame; there are 16).
pub fn shulker_model_for(texture: &Identifier) -> Option<Arc<CubeModel>> {
    static CACHE: OnceLock<Mutex<HashMap<Identifier, Option<Arc<CubeModel>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(texture) {
        return hit.clone();
    }
    let model = crate::registry::model_for_name("minecraft:shulker").map(|m| {
        if m.texture == *texture {
            m
        } else {
            let mut c = (*m).clone();
            c.texture = texture.clone();
            Arc::new(c)
        }
    });
    cache.lock().unwrap().insert(texture.clone(), model.clone());
    model
}
