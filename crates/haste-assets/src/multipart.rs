// This file is derived from Pomme (https://github.com/josefbednar/pomme),
// pomme-client/src/world/block/model.rs (blockstate variant/multipart parsing),
// reused under GPL-3.0 and reworked for Haste: full `when` condition trees with
// OR/AND and `a|b` value alternation. See THIRD_PARTY.md.

use std::collections::HashMap;

use serde::Deserialize;

use crate::model::ModelRef;

/// `blockstates/<path>.json`: either `variants` or `multipart`.
#[derive(Deserialize, Debug, Clone)]
pub struct BlockStateDefinition {
    #[serde(default)]
    pub variants: Option<HashMap<String, VariantEntry>>,
    #[serde(default)]
    pub multipart: Option<Vec<MultipartCase>>,
}

impl BlockStateDefinition {
    /// The quads' model refs for a state's property set: for `variants`, the entry
    /// whose key matches (all key properties must match; properties absent from
    /// the key are wildcards — 26.3 keys omit render-irrelevant state properties
    /// like `waterlogged`/`powered`); for `multipart`, every case whose `when`
    /// passes. Returns an empty Vec when the state has no models (e.g. air).
    pub fn model_refs_for(&self, props: &[(String, String)]) -> Vec<ModelRef> {
        if let Some(variants) = &self.variants {
            // Exact key first (fast path for the "" and fully-specified keys).
            let key = variant_key(props);
            if let Some(entry) = variants.get(&key) {
                return entry.first().into_iter().cloned().collect();
            }
            // Wildcard match: every property named in the variant key must match.
            let mut matches: Vec<(&String, &VariantEntry)> = variants
                .iter()
                .filter(|(k, _)| {
                    k.split(',')
                        .filter(|p| !p.is_empty())
                        .all(|part| {
                            let (k, want) = part.split_once('=').unwrap_or((part, ""));
                            props.iter().any(|(pk, pv)| pk == k && pv == want)
                        })
                })
                .collect();
            match matches.len() {
                0 => Vec::new(),
                1 => matches.remove(0).1.first().into_iter().cloned().collect(),
                // Multiple keys match: prefer the one naming the most properties.
                _ => {
                    matches.sort_by_key(|(k, _)| {
                        std::cmp::Reverse(k.split(',').filter(|p| !p.is_empty()).count())
                    });
                    matches.remove(0).1.first().into_iter().cloned().collect()
                }
            }
        } else if let Some(multipart) = &self.multipart {
            multipart
                .iter()
                .filter(|case| case.when_matches(props))
                .flat_map(|case| case.apply.first().into_iter().cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Every model ref mentioned anywhere in this definition (all variants, all
    /// multipart applies regardless of conditions), for atlas texture coverage.
    pub fn all_model_refs(&self) -> Vec<ModelRef> {
        let mut refs = Vec::new();
        if let Some(variants) = &self.variants {
            for entry in variants.values() {
                if let Some(r) = entry.first() {
                    refs.push(r.clone());
                }
            }
        } else if let Some(multipart) = &self.multipart {
            for case in multipart {
                if let Some(r) = case.apply.first() {
                    refs.push(r.clone());
                }
            }
        }
        refs
    }
}

/// A `variants` entry: a single model ref or an array (random variants; we take
/// the first, per the brief).
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum VariantEntry {
    Single(ModelRef),
    Array(Vec<ModelRef>),
}

impl VariantEntry {
    pub fn first(&self) -> Option<&ModelRef> {
        match self {
            VariantEntry::Single(r) => Some(r),
            VariantEntry::Array(arr) => arr.first(),
        }
    }
}

/// One `multipart` case: `apply` (+ optional `when`).
#[derive(Deserialize, Clone, Debug)]
pub struct MultipartCase {
    pub apply: MultipartApply,
    pub when: Option<MultipartCondition>,
}

/// `apply`: a single model ref or an array.
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MultipartApply {
    Single(ModelRef),
    Array(Vec<ModelRef>),
}

impl MultipartApply {
    pub fn first(&self) -> Option<&ModelRef> {
        match self {
            MultipartApply::Single(r) => Some(r),
            MultipartApply::Array(arr) => arr.first(),
        }
    }
}

/// A `when` condition. Property values match with `|` alternation (`side|up`);
/// `OR`/`AND` keys nest conditions (vanilla `PropertyValueCondition` / `ConditionList`).
#[derive(Clone, Debug)]
pub enum MultipartCondition {
    Or(Vec<MultipartCondition>),
    And(Vec<MultipartCondition>),
    Properties(HashMap<String, String>),
}

impl<'de> Deserialize<'de> for MultipartCondition {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(de)?;
        let Some(map) = value.as_object() else {
            return Err(serde::de::Error::custom("when condition must be an object"));
        };
        if map.len() == 1 {
            let (key, list) = map.iter().next().unwrap();
            match key.as_str() {
                "OR" | "or" => {
                    let conditions = parse_condition_list(list).map_err(serde::de::Error::custom)?;
                    return Ok(MultipartCondition::Or(conditions));
                }
                "AND" | "and" => {
                    let conditions = parse_condition_list(list).map_err(serde::de::Error::custom)?;
                    return Ok(MultipartCondition::And(conditions));
                }
                _ => {}
            }
        }
        // Otherwise: a property -> value map.
        let mut properties = HashMap::new();
        for (k, v) in map {
            let value = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            properties.insert(k.clone(), value);
        }
        Ok(MultipartCondition::Properties(properties))
    }
}

fn parse_condition_list(
    value: &serde_json::Value,
) -> Result<Vec<MultipartCondition>, String> {
    let list = value.as_array().ok_or("OR/AND value must be an array")?;
    list.iter()
        .map(|entry| {
            serde_json::from_value::<MultipartCondition>(entry.clone())
                .map_err(|e| format!("condition entry: {e}"))
        })
        .collect()
}

impl MultipartCondition {
    pub fn matches(&self, props: &[(String, String)]) -> bool {
        match self {
            MultipartCondition::Or(conditions) => {
                conditions.iter().any(|c| c.matches(props))
            }
            MultipartCondition::And(conditions) => {
                conditions.iter().all(|c| c.matches(props))
            }
            MultipartCondition::Properties(expected) => expected.iter().all(|(k, want)| {
                props
                    .iter()
                    .find(|(pk, _)| pk == k)
                    .is_some_and(|(_, got)| value_matches(want, got))
            }),
        }
    }
}

/// `side|up` matches either alternative (vanilla `Condition.comparison`).
fn value_matches(want: &str, got: &str) -> bool {
    want.split('|').any(|alt| alt == got)
}

impl MultipartCase {
    pub fn when_matches(&self, props: &[(String, String)]) -> bool {
        self.when.as_ref().is_none_or(|w| w.matches(props))
    }
}

/// The sorted `k=v,k=v` variant key of a property list (vanilla writes them sorted).
pub(crate) fn variant_key(props: &[(String, String)]) -> String {
    let mut parts: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    parts.sort();
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(json: &str, props: &[(&str, &str)]) -> Vec<ModelRef> {
        let def: BlockStateDefinition = serde_json::from_str(json).unwrap();
        let props: Vec<(String, String)> =
            props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        def.model_refs_for(&props)
    }

    const MULTIPART: &str = r#"{
        "multipart": [
            { "apply": { "model": "minecraft:block/post" } },
            { "apply": { "model": "minecraft:block/side", "y": 90, "uvlock": true },
              "when": { "east": "true" } },
            { "apply": { "model": "minecraft:block/side_alt" },
              "when": { "OR": [ { "north": "side|up" }, { "south": "side|up", "west": "false" } ] } }
        ]
    }"#;

    #[test]
    fn multipart_unconditional_always_applies() {
        let r = refs(MULTIPART, &[("north", "none")]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].model, "minecraft:block/post");
    }

    #[test]
    fn multipart_simple_when() {
        let r = refs(MULTIPART, &[("east", "true")]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].model, "minecraft:block/side");
        assert_eq!(r[1].y, 90);
        assert!(r[1].uvlock);
    }

    #[test]
    fn multipart_or_and_alternation() {
        // OR: first alternative matches via side|up alternation
        let r = refs(MULTIPART, &[("north", "side")]);
        assert_eq!(r.len(), 2);
        // OR: second alternative with two properties, one fails
        let r = refs(MULTIPART, &[("south", "up"), ("west", "true")]);
        assert_eq!(r.len(), 1);
        // OR: second alternative both match
        let r = refs(MULTIPART, &[("south", "side"), ("west", "false")]);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn and_condition() {
        let json = r#"{
            "multipart": [
                { "apply": { "model": "m" }, "when": { "AND": [ {"a": "1"}, {"b": "2"} ] } }
            ]
        }"#;
        assert_eq!(refs(json, &[("a", "1"), ("b", "2")]).len(), 1);
        assert_eq!(refs(json, &[("a", "1"), ("b", "3")]).len(), 0);
    }

    #[test]
    fn variants_exact_key_and_first_of_random() {
        let json = r#"{
            "variants": {
                "facing=east,half=bottom": { "model": "minecraft:block/e" },
                "facing=east,half=top": [
                    { "model": "minecraft:block/t1" },
                    { "model": "minecraft:block/t2" }
                ]
            }
        }"#;
        let r = refs(json, &[("facing", "east"), ("half", "bottom")]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].model, "minecraft:block/e");
        // random variant array: take the first
        let r = refs(json, &[("facing", "east"), ("half", "top")]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].model, "minecraft:block/t1");
        // key must match exactly
        assert!(refs(json, &[("facing", "west"), ("half", "bottom")]).is_empty());
    }

    #[test]
    fn empty_state_variant_key() {
        let json = r#"{ "variants": { "": { "model": "minecraft:block/x" } } }"#;
        let r = refs(json, &[]);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn variant_key_properties_are_wildcards_for_the_rest() {
        // 26.3 keys omit render-irrelevant properties (waterlogged, powered):
        // a state carrying extra properties must still match the key.
        let json = r#"{
            "variants": {
                "facing=east,half=bottom,open=false": { "model": "minecraft:block/closed" },
                "facing=east,half=bottom,open=true": { "model": "minecraft:block/open" }
            }
        }"#;
        let r = refs(
            json,
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("open", "true"),
                ("waterlogged", "true"),
                ("powered", "false"),
            ],
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].model, "minecraft:block/open");
    }

    #[test]
    fn variant_key_wildcard_prefers_most_specific() {
        let json = r#"{
            "variants": {
                "": { "model": "minecraft:block/generic" },
                "lit=true": { "model": "minecraft:block/lit" }
            }
        }"#;
        let r = refs(json, &[("lit", "true")]);
        assert_eq!(r[0].model, "minecraft:block/lit");
        let r = refs(json, &[("lit", "false")]);
        assert_eq!(r[0].model, "minecraft:block/generic");
    }
}
