//! The decoration kind of an entity type: which special renderer (if any) draws it (see
//! `registry`).

use std::collections::HashMap;
use std::sync::OnceLock;

use haste_core::ids::RegistryIndex;

use super::load_entity_type_names;

/// What special renderer (if any) an entity type takes: `ItemFrame(glow)`, `Painting` or
/// `LightningBolt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecorationKind {
    ItemFrame {
        glow: bool,
    },
    Painting,
    LightningBolt,
    ExperienceOrb,
    /// `DisplayRenderer.BlockDisplayRenderer` / `ItemDisplayRenderer` / `TextDisplayRenderer`.
    Display(crate::display::DisplayKind),
}

/// The decoration kind of an entity type (`minecraft:item_frame`, `glow_item_frame`,
/// `painting`, `lightning_bolt`); `None` for every other type. Warm lookups are a map probe,
/// never an allocation (called per entity per frame).
pub fn decoration_kind(kind: RegistryIndex) -> Option<DecorationKind> {
    static NAMES: OnceLock<HashMap<RegistryIndex, &'static str>> = OnceLock::new();
    let name = *NAMES
        .get_or_init(|| load_entity_type_names().unwrap_or_default())
        .get(&kind)?;
    match name {
        "minecraft:item_frame" => Some(DecorationKind::ItemFrame { glow: false }),
        "minecraft:glow_item_frame" => Some(DecorationKind::ItemFrame { glow: true }),
        "minecraft:painting" => Some(DecorationKind::Painting),
        "minecraft:lightning_bolt" => Some(DecorationKind::LightningBolt),
        "minecraft:experience_orb" => Some(DecorationKind::ExperienceOrb),
        name => crate::display::DisplayKind::for_name(name).map(DecorationKind::Display),
    }
}
