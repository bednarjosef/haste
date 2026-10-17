//! The shulker's colour texture (`ShulkerRenderer.getTextureLocation`).
use haste_core::entity::MetadataValue;
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
