//! `blocks.json`: every block, its properties and its states, in dense tables indexed by
//! `BlockStateId`.
//!
//! Layout (all indexed by state id, no per-state allocations):
//! - `state_block`: index into `blocks` (the owning block)
//! - `state_props`: flattened `(property name id, value id)` pairs; `state_prop_range` slices them
//! - per-block: name, property descriptors (sorted by name), first state id, state count,
//!   default state, `minecraft:block` registry index
//!
//! Property names and values are interned in one string pool so a state's property list is a
//! `&[(u32, u32)]` slice with no per-call allocation.
//!
//! State-id layout, verified against the 26.3 report for all 35723 states: within a block, ids
//! run `first .. first + product(len(values))` in mixed-radix order over the *name-sorted*
//! properties with the last one varying fastest (blocks.json lists property definitions sorted
//! for all but 12 blocks; those 12 match once sorted). The 12 unsorted-definition blocks are
//! still handled by the same code because we sort property descriptors by name at load.

use std::collections::BTreeMap;

use ahash::HashMapExt as _;

use haste_core::{BlockStateId, Identifier, RegistryIndex};

use crate::registries::Registries;

/// Interned property names and values shared by all states.
#[derive(Default)]
struct Strings {
    pool: Vec<Box<str>>,
    lookup: ahash::HashMap<Box<str>, u32>,
}

impl Strings {
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.lookup.get(s) {
            return id;
        }
        let id = self.pool.len() as u32;
        self.pool.push(s.into());
        self.lookup.insert(s.into(), id);
        id
    }
    fn get(&self, id: u32) -> &str {
        &self.pool[id as usize]
    }
}

/// One property of a block: interned name plus its allowed values, interned.
struct PropertyDesc {
    name: u32,
    values: Vec<u32>,
}

struct BlockDesc {
    name: Identifier,
    /// Sorted by property name (the state-id radix order).
    properties: Vec<PropertyDesc>,
    first_state: u32,
    state_count: u32,
    default_state: u32,
    /// Index in the `minecraft:block` registry.
    registry_index: Option<RegistryIndex>,
}

/// Dense block-state tables built from `blocks.json`. See docs/INTERFACES.md.
pub struct BlockStates {
    strings: Strings,
    blocks: Vec<BlockDesc>,
    /// State id -> index into `blocks`.
    state_block: Vec<u32>,
    /// State id -> (start, len) into `state_props`.
    state_prop_range: Vec<(u32, u32)>,
    /// Flattened (property name id, value id) pairs per state, in blocks.json order.
    state_props: Vec<(u32, u32)>,
    /// `ns:path` -> index into `blocks`.
    by_name: ahash::HashMap<Box<str>, u32>,
}

impl BlockStates {
    pub(crate) fn parse(json: &str, registries: Option<&Registries>) -> anyhow::Result<Self> {
        let raw: BTreeMap<&str, RawBlock> =
            serde_json::from_str(json).map_err(|e| anyhow::anyhow!("blocks.json: {e}"))?;

        // Max state id + 1 (ids may start above 0 in malformed input; density is checked later).
        let table_len: u32 = raw
            .values()
            .flat_map(|b| b.states.iter().map(|s| s.id))
            .max()
            .map(|max| max + 1)
            .unwrap_or(0);
        let total_states: u32 = raw.values().map(|b| b.states.len() as u32).sum();
        let mut strings = Strings::default();
        let mut blocks: Vec<BlockDesc> = Vec::with_capacity(raw.len());
        let mut by_name: ahash::HashMap<Box<str>, u32> = ahash::HashMap::with_capacity(raw.len());
        // Indexed by state id, not append order: blocks.json may list blocks in any order.
        let mut state_block: Vec<u32> = vec![u32::MAX; table_len as usize];
        let mut state_prop_range: Vec<(u32, u32)> = vec![(0, 0); table_len as usize];
        let mut state_props: Vec<(u32, u32)> = Vec::new();

        for (name, block) in &raw {
            let ident =
                Identifier::parse(name).map_err(|e| anyhow::anyhow!("blocks.json: {e}"))?;

            // Property descriptors sorted by name (state-id radix order).
            let mut properties: Vec<PropertyDesc> = Vec::new();
            if let Some(props) = &block.properties {
                for (pname, values) in props {
                    let pid = strings.intern(pname);
                    let vals: Vec<u32> = values.iter().map(|v| strings.intern(v)).collect();
                    properties.push(PropertyDesc { name: pid, values: vals });
                }
            }

            let first_state = block
                .states
                .first()
                .map(|s| s.id)
                .ok_or_else(|| anyhow::anyhow!("blocks.json: {name} has no states"))?;
            let state_count = block.states.len() as u32;
            let default_state = block
                .states
                .iter()
                .find(|s| s.default)
                .map(|s| s.id)
                .ok_or_else(|| anyhow::anyhow!("blocks.json: {name} has no default state"))?;

            let registry_index = registries.and_then(|r| r.index("minecraft:block", &ident));
            let block_index = blocks.len() as u32;
            blocks.push(BlockDesc {
                name: ident,
                properties,
                first_state,
                state_count,
                default_state,
                registry_index,
            });
            by_name.insert((*name).into(), block_index);

            for (i, state) in block.states.iter().enumerate() {
                // State ids within a block are contiguous and ascending (verified for 26.3).
                let expected = first_state + i as u32;
                if state.id != expected {
                    anyhow::bail!(
                        "blocks.json: {name}: state id {} where {} was expected \
                         (ids must be contiguous ascending within a block)",
                        state.id,
                        expected
                    );
                }
                let props = state.properties.as_deref().unwrap_or(&[]);
                let start = state_props.len() as u32;
                let n_props = props.len();
                state_props.extend(props.iter().map(|&(k, v)| (strings.intern(k), strings.intern(v))));
                if n_props != blocks[block_index as usize].properties.len() {
                    anyhow::bail!(
                        "blocks.json: {name}: state {} has {} properties, expected {}",
                        state.id,
                        n_props,
                        blocks[block_index as usize].properties.len()
                    );
                }
                let slot = &mut state_block[state.id as usize];
                if *slot != u32::MAX {
                    anyhow::bail!("blocks.json: duplicate state id {}", state.id);
                }
                *slot = block_index;
                state_prop_range[state.id as usize] = (start, n_props as u32);
            }
        }

        // Dense ids 0..n with minecraft:air = 0.
        if total_states > 0 {
            if table_len != total_states || state_block.contains(&u32::MAX) {
                anyhow::bail!("blocks.json: state ids are not dense 0..{total_states}");
            }
            if let Some(&air) = by_name.get("minecraft:air") {
                let air_desc = &blocks[air as usize];
                if air_desc.first_state != 0 || air_desc.state_count != 1 {
                    anyhow::bail!(
                        "blocks.json: minecraft:air must be exactly state 0, is {}..{}",
                        air_desc.first_state,
                        air_desc.first_state + air_desc.state_count - 1
                    );
                }
            } else {
                anyhow::bail!("blocks.json: minecraft:air is missing");
            }
        }

        Ok(Self {
            strings,
            blocks,
            state_block,
            state_prop_range,
            state_props,
            by_name,
        })
    }

    /// Number of block states.
    pub fn len(&self) -> usize {
        self.state_block.len()
    }

    /// Whether there are no states (empty/failed load).
    pub fn is_empty(&self) -> bool {
        self.state_block.is_empty()
    }

    /// Block owning state `s`. Panics if `s` is out of range (ids are dense at load).
    pub fn block_of(&self, s: BlockStateId) -> &Identifier {
        &self.blocks[self.state_block[s.0 as usize] as usize].name
    }

    /// Properties of state `s` as interned `(name id, value id)` pairs in blocks.json order
    /// (which is name-sorted for all but 12 blocks).
    fn prop_ids(&self, s: BlockStateId) -> &[(u32, u32)] {
        let (start, len) = self.state_prop_range[s.0 as usize];
        &self.state_props[start as usize..(start + len) as usize]
    }

    /// Properties of state `s`, sorted by name, as `&[(String, String)]` per INTERFACES.md.
    ///
    /// The interface asks for owned strings; returning `&[(String, String)]` without allocation
    /// is impossible with interned storage, so this allocates per call. Hot code should use
    /// [`BlockStates::property`] / [`BlockStates::property_value`] instead, which borrow.
    pub fn properties(&self, s: BlockStateId) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self
            .prop_ids(s)
            .iter()
            .map(|&(k, val)| (self.strings.get(k).to_owned(), self.strings.get(val).to_owned()))
            .collect();
        v.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Value of property `name` on state `s`, if the block has it.
    pub fn property_value<'a>(&'a self, s: BlockStateId, name: &str) -> Option<&'a str> {
        let (_, v) = *self
            .prop_ids(s)
            .iter()
            .find(|&&(k, _)| self.strings.get(k) == name)?;
        Some(self.strings.get(v))
    }

    /// Default state of `block`.
    pub fn default_state(&self, block: &Identifier) -> Option<BlockStateId> {
        let &b = self.by_name.get(block.to_string().as_str())?;
        Some(BlockStateId(self.blocks[b as usize].default_state))
    }

    /// State of `block` with exactly `props` set (all other properties at their default? no:
    /// `props` must list every property of the block, matching vanilla's block-state syntax).
    /// Blocks without properties resolve when `props` is empty.
    pub fn state(&self, block: &Identifier, props: &[(&str, &str)]) -> Option<BlockStateId> {
        let &b = self.by_name.get(block.to_string().as_str())?;
        let desc = &self.blocks[b as usize];
        // Mixed-radix offset over name-sorted properties.
        let mut offset: u32 = 0;
        for prop in &desc.properties {
            let pname = self.strings.get(prop.name);
            let value = props.iter().find(|(k, _)| *k == pname).map(|(_, v)| *v)?;
            // Radix digit: the value's *position* in the property's value list, not its
            // interned string id.
            let digit = prop
                .values
                .iter()
                .position(|&v| self.strings.get(v) == value)?;
            offset = offset * prop.values.len() as u32 + digit as u32;
        }
        if desc.properties.len() != props.len() {
            return None;
        }
        Some(BlockStateId(desc.first_state + offset))
    }

    /// State `s` with property `name` set to `value` (the other properties unchanged).
    pub fn with_property(
        &self,
        s: BlockStateId,
        name: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        if self.property_value(s, name) == Some(value) {
            return Some(s);
        }
        let b = self.state_block[s.0 as usize] as usize;
        let desc = &self.blocks[b];
        // The block must actually have the property.
        desc.properties
            .iter()
            .find(|p| self.strings.get(p.name) == name)?;
        // Rebuild the full property map of `s`, then radix-encode.
        let mut offset: u32 = 0;
        for p in &desc.properties {
            let pname = self.strings.get(p.name);
            let cur = self.property_value(s, pname)?;
            let want = if pname == name { value } else { cur };
            let digit = p.values.iter().position(|&v| self.strings.get(v) == want)?;
            offset = offset * p.values.len() as u32 + digit as u32;
        }
        Some(BlockStateId(desc.first_state + offset))
    }

    /// Index of `block` in the `minecraft:block` registry.
    pub fn block_index(&self, block: &Identifier) -> Option<RegistryIndex> {
        let &b = self.by_name.get(block.to_string().as_str())?;
        self.blocks[b as usize].registry_index
    }
}

#[derive(serde::Deserialize)]
struct RawBlock<'a> {
    #[serde(borrow, default)]
    properties: Option<BTreeMap<&'a str, Vec<&'a str>>>,
    states: Vec<RawState<'a>>,
}

#[derive(serde::Deserialize)]
struct RawState<'a> {
    id: u32,
    #[serde(default)]
    default: bool,
    /// A JSON map deserialized as ordered pairs (key order as in the file); a BTreeMap per
    /// state would cost ~35k allocations.
    #[serde(borrow, default, with = "ordered_pairs")]
    properties: Option<Vec<(&'a str, &'a str)>>,
}

/// Deserializes a JSON object into `Vec<(&str, &str)>` pairs, preserving file key order.
#[allow(clippy::type_complexity)]
mod ordered_pairs {
    use serde::de::{Deserializer, MapAccess, Visitor};
    use std::fmt;

    pub fn deserialize<'de, D>(
        d: D,
    ) -> Result<Option<Vec<(&'de str, &'de str)>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Option<Vec<(&'de str, &'de str)>>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map of property names to values")
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_some<D>(self, d: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct Inner;
                impl<'de> Visitor<'de> for Inner {
                    type Value = Vec<(&'de str, &'de str)>;
                    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        f.write_str("a map of property names to values")
                    }
                    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                        let mut out = Vec::with_capacity(map.size_hint().unwrap_or(0));
                        while let Some((k, v)) = map.next_entry()? {
                            out.push((k, v));
                        }
                        Ok(out)
                    }
                }
                d.deserialize_map(Inner).map(Some)
            }
        }
        d.deserialize_option(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone_json() -> String {
        r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": {
    "states": [ { "id": 1, "default": true } ]
  },
  "minecraft:oak_stairs": {
    "properties": {
      "facing": ["north", "south", "west", "east"],
      "half": ["top", "bottom"],
      "shape": ["straight", "inner_left", "inner_right", "outer_left", "outer_right"],
      "waterlogged": ["true", "false"]
    },
    "states": [
      { "id": 2, "properties": { "facing": "north", "half": "top", "shape": "straight", "waterlogged": "true" } },
      { "id": 3, "properties": { "facing": "north", "half": "top", "shape": "straight", "waterlogged": "false" } },
      { "id": 4, "default": true, "properties": { "facing": "north", "half": "top", "shape": "inner_left", "waterlogged": "true" } },
      { "id": 5, "properties": { "facing": "north", "half": "top", "shape": "inner_left", "waterlogged": "false" } }
    ]
  }
}"#
        .to_owned()
    }

    #[test]
    fn parses_and_looks_up() {
        let bs = BlockStates::parse(&stone_json(), None).unwrap();
        assert_eq!(bs.len(), 6);
        assert_eq!(bs.block_of(BlockStateId(0)), &Identifier::minecraft("air"));
        assert_eq!(bs.block_of(BlockStateId(1)), &Identifier::minecraft("stone"));
        assert_eq!(bs.block_of(BlockStateId(2)), &Identifier::minecraft("oak_stairs"));
        assert_eq!(
            bs.default_state(&Identifier::minecraft("oak_stairs")),
            Some(BlockStateId(4))
        );
    }

    #[test]
    fn state_lookup_radix() {
        let bs = BlockStates::parse(&stone_json(), None).unwrap();
        let stairs = Identifier::minecraft("oak_stairs");
        assert_eq!(
            bs.state(
                &stairs,
                &[("facing", "north"), ("half", "top"), ("shape", "straight"), ("waterlogged", "true")]
            ),
            Some(BlockStateId(2))
        );
        assert_eq!(
            bs.state(
                &stairs,
                &[("facing", "north"), ("half", "top"), ("shape", "straight"), ("waterlogged", "false")]
            ),
            Some(BlockStateId(3))
        );
        assert_eq!(
            bs.state(
                &stairs,
                &[("facing", "north"), ("half", "top"), ("shape", "inner_left"), ("waterlogged", "true")]
            ),
            Some(BlockStateId(4))
        );
        assert_eq!(
            bs.with_property(BlockStateId(2), "waterlogged", "false"),
            Some(BlockStateId(3))
        );
        assert_eq!(
            bs.with_property(BlockStateId(2), "shape", "inner_left"),
            Some(BlockStateId(4))
        );
        assert_eq!(bs.property_value(BlockStateId(2), "facing"), Some("north"));
        assert_eq!(bs.property_value(BlockStateId(2), "nope"), None);
        let props = bs.properties(BlockStateId(2));
        assert_eq!(props.len(), 4);
        assert_eq!(props[0], ("facing".to_string(), "north".to_string()));
    }

    #[test]
    fn air_must_be_state_zero() {
        let json = r#"{
  "minecraft:air": { "states": [ { "id": 1, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 0, "default": true } ] }
}"#;
        assert!(BlockStates::parse(json, None).is_err());
    }

    #[test]
    fn non_dense_ids_error() {
        let json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:stone": { "states": [ { "id": 5, "default": true } ] }
}"#;
        assert!(BlockStates::parse(json, None).is_err());
    }

    #[test]
    fn unsorted_property_block_parses() {
        // chest-like: definitions listed out of name order
        let json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:chest": {
    "properties": { "type": ["single", "left", "right"], "facing": ["north", "south", "west", "east"], "waterlogged": ["true", "false"] },
    "states": [
      { "id": 1, "properties": { "type": "single", "facing": "north", "waterlogged": "true" } },
      { "id": 2, "properties": { "type": "single", "facing": "north", "waterlogged": "false" } },
      { "id": 3, "properties": { "type": "left", "facing": "north", "waterlogged": "true" } },
      { "id": 4, "properties": { "type": "left", "facing": "north", "waterlogged": "false" } },
      { "id": 5, "properties": { "type": "right", "facing": "north", "waterlogged": "true" } },
      { "id": 6, "properties": { "type": "right", "facing": "north", "waterlogged": "false" } },
      { "id": 7, "default": true, "properties": { "type": "single", "facing": "south", "waterlogged": "true" } },
      { "id": 8, "properties": { "type": "single", "facing": "south", "waterlogged": "false" } },
      { "id": 9, "properties": { "type": "left", "facing": "south", "waterlogged": "true" } },
      { "id": 10, "properties": { "type": "left", "facing": "south", "waterlogged": "false" } },
      { "id": 11, "properties": { "type": "right", "facing": "south", "waterlogged": "true" } },
      { "id": 12, "properties": { "type": "right", "facing": "south", "waterlogged": "false" } },
      { "id": 13, "properties": { "type": "single", "facing": "west", "waterlogged": "true" } },
      { "id": 14, "properties": { "type": "single", "facing": "west", "waterlogged": "false" } },
      { "id": 15, "properties": { "type": "left", "facing": "west", "waterlogged": "true" } },
      { "id": 16, "properties": { "type": "left", "facing": "west", "waterlogged": "false" } },
      { "id": 17, "properties": { "type": "right", "facing": "west", "waterlogged": "true" } },
      { "id": 18, "properties": { "type": "right", "facing": "west", "waterlogged": "false" } },
      { "id": 19, "properties": { "type": "single", "facing": "east", "waterlogged": "true" } },
      { "id": 20, "properties": { "type": "single", "facing": "east", "waterlogged": "false" } },
      { "id": 21, "properties": { "type": "left", "facing": "east", "waterlogged": "true" } },
      { "id": 22, "properties": { "type": "left", "facing": "east", "waterlogged": "false" } },
      { "id": 23, "properties": { "type": "right", "facing": "east", "waterlogged": "true" } },
      { "id": 24, "properties": { "type": "right", "facing": "east", "waterlogged": "false" } }
    ]
  }
}"#;
        let bs = BlockStates::parse(json, None).unwrap();
        // with property definitions sorted by name: facing(4) * type(3) * waterlogged(2)
        // state(type=left, facing=east, waterlogged=true): radix over sorted props:
        // facing=east (3) * 6 + type=left (1) * 2 + waterlogged=true (0) = 20 -> id 21
        assert_eq!(
            bs.state(
                &Identifier::minecraft("chest"),
                &[("facing", "east"), ("type", "left"), ("waterlogged", "true")]
            ),
            Some(BlockStateId(21))
        );
    }
}
