//! Item default data components, from the data generator's `RegistryComponentsReport`
//! (`reports/components/item/<name>.json`, one file per item that has any non-empty default
//! components). The client needs the item's `equippable` component to know which equipment
//! asset an armour piece draws and in which slot (`HumanoidArmorLayer` / `WingsLayer` read it
//! from the stack, whose default components the built-in item registry carries). Haste does not
//! run vanilla's Java item registry, so it reads the report instead.

use std::path::Path;

use ahash::AHashMap;

use haste_core::model::EquipmentSlot;
use haste_core::{Identifier, RegistryIndex};

use crate::registries::Registries;

/// The `equippable` component of one item: the slot it goes in and the equipment asset it draws.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemEquippable {
    pub slot: EquipmentSlot,
    /// `equippable.asset_id` (`minecraft:iron` -> `equipment/iron.json`); `None` for items whose
    /// equippable has no asset (saddles on some mobs, the `equippable` marker without a texture).
    pub asset: Option<Identifier>,
}

/// Item default components by `minecraft:item` protocol id. Items without an `equippable` (or
/// unknown ids) map to `None`.
#[derive(Clone, Debug, Default)]
pub struct ItemComponents {
    equippable: Vec<Option<ItemEquippable>>,
    /// The `minecraft:dyed_color` data-component-type id, from the registry report. `None` when
    /// the report is absent.
    dyed_color_type: Option<i32>,
}

impl ItemComponents {
    /// The `equippable` of item `i`, if the report named one.
    pub fn equippable(&self, i: RegistryIndex) -> Option<&ItemEquippable> {
        self.equippable.get(i.0 as usize)?.as_ref()
    }

    /// The `dyed_color` (ARGB) a stack's component patch sets, if it is the patch's first added
    /// component (`DyedItemColor.STREAM_CODEC` is a plain int; vanilla's `getOrDefault` falls
    /// back to the layer's `color_when_undyed` when absent). A patch whose first add is a
    /// component Haste cannot skip returns `None` (dye is then simply not applied).
    pub fn dyed_color(&self, components_raw: &[u8]) -> Option<u32> {
        let dyed = self.dyed_color_type?;
        let mut r = PatchReader::new(components_raw);
        let adds = r.var_int()?;
        let _removes = r.var_int()?;
        if adds <= 0 {
            return None;
        }
        let ty = r.var_int()?;
        if ty != dyed {
            return None;
        }
        r.i32().map(|c| c as u32)
    }

    /// Loads every `reports/components/item/*.json` under `components_dir` (the report's
    /// `components/item` directory), mapping each file's name through `registries`. A missing
    /// directory or unreadable file yields an empty table (armour then draws nothing), never an
    /// error: the components report is optional data.
    pub fn load(components_dir: &Path, registries: &Registries) -> ItemComponents {
        let mut by_name: AHashMap<String, ItemEquippable> = AHashMap::new();
        let Ok(entries) = std::fs::read_dir(components_dir) else {
            return ItemComponents {
                dyed_color_type: dyed_color_type(registries),
                ..ItemComponents::default()
            };
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if let Some(eq) = parse_equippable(&bytes) {
                by_name.insert(stem.to_owned(), eq);
            }
        }
        let len = registries.len("minecraft:item");
        let mut equippable = vec![None; len];
        for (name, eq) in by_name {
            if let Some(i) = registries.index("minecraft:item", &Identifier::minecraft(&name)) {
                equippable[i.0 as usize] = Some(eq);
            }
        }
        ItemComponents {
            equippable,
            dyed_color_type: dyed_color_type(registries),
        }
    }
}

fn dyed_color_type(registries: &Registries) -> Option<i32> {
    registries
        .index(
            "minecraft:data_component_type",
            &Identifier::minecraft("dyed_color"),
        )
        .map(|i| i.0 as i32)
}

/// A minimal varint/int reader over a component patch's raw bytes (only what
/// [`ItemComponents::dyed_color`] needs).
struct PatchReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> PatchReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        PatchReader { bytes, at: 0 }
    }

    fn var_int(&mut self) -> Option<i32> {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            value |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Some(value as i32);
            }
            shift += 7;
            if shift >= 32 {
                return None;
            }
        }
    }

    fn i32(&mut self) -> Option<i32> {
        let b = self.bytes.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// The `minecraft:equippable` component of one `{"components": {...}}` report file, or `None`
/// when the item has none (or the file is malformed).
pub fn parse_equippable(bytes: &[u8]) -> Option<ItemEquippable> {
    let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let eq = json.get("components")?.get("minecraft:equippable")?;
    let slot = eq.get("slot")?.as_str()?;
    let slot = match slot {
        "mainhand" => EquipmentSlot::MainHand,
        "offhand" => EquipmentSlot::OffHand,
        "feet" => EquipmentSlot::Feet,
        "legs" => EquipmentSlot::Legs,
        "chest" => EquipmentSlot::Chest,
        "head" => EquipmentSlot::Head,
        "body" => EquipmentSlot::Body,
        "saddle" => EquipmentSlot::Saddle,
        _ => return None,
    };
    let asset = eq
        .get("asset_id")
        .and_then(|a| a.as_str())
        .and_then(|a| Identifier::parse(a).ok());
    Some(ItemEquippable { slot, asset })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_equippable_component() {
        let json = br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#;
        let eq = parse_equippable(json).unwrap();
        assert_eq!(eq.slot, EquipmentSlot::Chest);
        assert_eq!(eq.asset, Some(Identifier::minecraft("iron")));
    }

    #[test]
    fn missing_equippable_is_none() {
        assert_eq!(parse_equippable(br#"{"components":{}}"#), None);
        assert_eq!(parse_equippable(b"not json"), None);
    }

    #[test]
    fn loads_by_registry_name_and_ignores_unknown_items() {
        let dir = std::env::temp_dir().join(format!(
            "haste-components-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let item_dir = dir.join("item");
        std::fs::create_dir_all(&item_dir).unwrap();
        std::fs::write(
            item_dir.join("iron_chestplate.json"),
            br#"{"components":{"minecraft:equippable":{"slot":"chest","asset_id":"minecraft:iron"}}}"#,
        )
        .unwrap();
        std::fs::write(item_dir.join("unknown_item.json"), br#"{"components":{}}"#).unwrap();
        let registries_json = r#"{
  "minecraft:item": {
    "entries": {
      "minecraft:air": { "protocol_id": 0 },
      "minecraft:iron_chestplate": { "protocol_id": 1 }
    }
  },
  "minecraft:data_component_type": {
    "entries": {
      "minecraft:damage": { "protocol_id": 0 },
      "minecraft:dyed_color": { "protocol_id": 1 }
    }
  }
}"#;
        let regs = Registries::parse(registries_json).unwrap();
        let components = ItemComponents::load(&item_dir, &regs);
        let eq = components.equippable(RegistryIndex(1)).unwrap();
        assert_eq!(eq.asset, Some(Identifier::minecraft("iron")));
        assert!(components.equippable(RegistryIndex(0)).is_none());
        // `dyed_color` as the patch's first add (type id 1, a big-endian int) decodes.
        let patch = [1u8, 0, 1, 0x00, 0x11, 0x22, 0x33];
        assert_eq!(components.dyed_color(&patch), Some(0x0011_2233));
        // A patch whose first add is another component gives up (dye not applied).
        assert_eq!(components.dyed_color(&[1, 0, 0, 5]), None);
        assert_eq!(components.dyed_color(&[0, 0]), None);
        // A missing directory is empty, not an error.
        let empty = ItemComponents::load(&dir.join("nope"), &regs);
        assert!(empty.equippable(RegistryIndex(1)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
