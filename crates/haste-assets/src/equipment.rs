//! `assets/minecraft/equipment/<asset>.json` (`EquipmentClientInfo`): the textures an equipment
//! asset draws, by layer type. Vanilla's `EquipmentLayerRenderer.renderLayers` reads these and
//! draws each layer through the armour model; the texture path is
//! `textures/entity/equipment/<layer type>/<texture path>.png`.
//!
//! Only the fields the renderer needs are kept: the layer's texture, whether it is dyeable (and
//! the colour vanilla falls back to when undyed), and `use_player_texture` (a player's own cape
//! or elytra skin). Trims are not modelled yet.

use anyhow::{Context, Result};

use haste_core::Identifier;

use crate::assets::Assets;

/// One equipment asset's layers (`EquipmentClientInfo`).
#[derive(Clone, Debug, Default)]
pub struct EquipmentInfo {
    pub layers: Vec<EquipmentLayer>,
}

impl EquipmentInfo {
    /// The layers of one `EquipmentClientInfo.LayerType` (`"humanoid"`, `"humanoid_leggings"`,
    /// `"wings"`, ...), in file order.
    pub fn layers_of<'a>(&'a self, layer_type: &'a str) -> impl Iterator<Item = &'a EquipmentLayer> {
        self.layers.iter().filter(move |l| l.layer_type == layer_type)
    }
}

/// One `EquipmentClientInfo.Layer`.
#[derive(Clone, Debug)]
pub struct EquipmentLayer {
    /// `EquipmentClientInfo.LayerType.getSerializedName()` (`humanoid`, `humanoid_leggings`, ...).
    pub layer_type: String,
    /// The resolved texture id (`textures/entity/equipment/<type>/<path>.png`).
    pub texture: Identifier,
    /// `dyeable.color_when_undyed` (ARGB, opaque) when the layer is dyeable; `None` otherwise.
    pub dyeable: Option<u32>,
    /// `use_player_texture`: a player's cape/elytra skin overrides the layer texture.
    pub use_player_texture: bool,
}

impl Assets {
    /// The equipment info of an asset id (`minecraft:iron` -> `equipment/iron.json`). An unknown
    /// asset is an error the caller turns into "draw nothing".
    pub fn equipment_info(&mut self, asset: &Identifier) -> Result<EquipmentInfo> {
        let key = crate::assets::asset_key(asset, "equipment", ".json");
        let bytes = self.read_asset(&key)?;
        parse_equipment_info(&bytes).with_context(|| format!("parsing {key}"))
    }
}

/// Parses an `EquipmentClientInfo` JSON body (split out so it is testable without the jar).
pub fn parse_equipment_info(bytes: &[u8]) -> Result<EquipmentInfo> {
    let json: serde_json::Value = serde_json::from_slice(bytes)?;
    let layers_json = json
        .get("layers")
        .and_then(|l| l.as_object())
        .context("equipment without `layers`")?;
    let mut layers = Vec::new();
    for (layer_type, list) in layers_json {
        let Some(list) = list.as_array() else { continue };
        for l in list {
            let Some(texture) = l.get("texture").and_then(|t| t.as_str()) else {
                continue;
            };
            let Ok(texture) = Identifier::parse(texture) else {
                continue;
            };
            let path = format!(
                "textures/entity/equipment/{}/{}.png",
                layer_type,
                texture.path()
            );
            let texture = Identifier::new(texture.namespace(), &path);
            let dyeable = l
                .get("dyeable")
                .and_then(|d| d.get("color_when_undyed"))
                .and_then(|c| c.as_i64())
                .map(|c| (c as u32) | 0xFF00_0000);
            let use_player_texture = l
                .get("use_player_texture")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            layers.push(EquipmentLayer {
                layer_type: layer_type.clone(),
                texture,
                dyeable,
                use_player_texture,
            });
        }
    }
    Ok(EquipmentInfo { layers })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser reads the layer map, resolves the texture path and keeps the dye fallback.
    #[test]
    fn parses_layers_and_texture_paths() {
        // A hand-written sample in the shape `EquipmentAssetProvider` writes.
        let json = r#"{
          "layers": {
            "humanoid": [ { "texture": "minecraft:leather", "dyeable": { "color_when_undyed": -6265536 } } ],
            "humanoid_leggings": [ { "texture": "minecraft:leather", "dyeable": { "color_when_undyed": -6265536 } } ]
          }
        }"#;
        let info = parse_equipment_info(json.as_bytes()).unwrap();
        assert_eq!(info.layers.len(), 2);
        let humanoid: Vec<&EquipmentLayer> = info.layers_of("humanoid").collect();
        assert_eq!(humanoid.len(), 1);
        assert_eq!(
            humanoid[0].texture,
            Identifier::minecraft("textures/entity/equipment/humanoid/leather.png")
        );
        assert_eq!(humanoid[0].dyeable, Some((-6265536i64 as u32) | 0xFF00_0000));
        let leggings: Vec<&EquipmentLayer> = info.layers_of("humanoid_leggings").collect();
        assert_eq!(
            leggings[0].texture,
            Identifier::minecraft("textures/entity/equipment/humanoid_leggings/leather.png")
        );
    }

    #[test]
    fn unknown_asset_is_an_error() {
        assert!(parse_equipment_info(b"{}").is_err());
    }
}
