//! The leaves of an item definition's `model` tree: the models a default stack draws, with the
//! composite transformations on the way down. Follows the `ItemModelResolver` choices a GUI
//! slot makes (see the module doc of `item`).

use super::{parse_transformation, PartTransformation};

#[cfg(test)]
mod tests;

pub(super) struct Leaf {
    pub(super) model: Option<String>,
    pub(super) tints: Vec<serde_json::Value>,
    pub(super) special: bool,
    pub(super) transformation: Option<PartTransformation>,
}

fn node_type(node: &serde_json::Value) -> &str {
    node.get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim_start_matches("minecraft:")
}

/// A condition's `property`, without its `minecraft:` namespace.
fn property_name(node: &serde_json::Value) -> Option<&str> {
    node.get("property")
        .and_then(|p| p.as_str())
        .map(|p| p.trim_start_matches("minecraft:"))
}

/// `cast` is the `minecraft:fishing_rod/cast` property: whether the holder's hook is out
/// (vanilla `FishingRodCast.get`). Only that property is answered; the others take `on_false`.
pub(super) fn collect_leaves(
    node: &serde_json::Value,
    transformation: Option<PartTransformation>,
    cast: bool,
    out: &mut Vec<Leaf>,
) {
    let transformation = node
        .get("transformation")
        .and_then(parse_transformation)
        .or(transformation);
    match node_type(node) {
        "model" => out.push(Leaf {
            model: node.get("model").and_then(|m| m.as_str()).map(String::from),
            tints: node
                .get("tints")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default(),
            special: false,
            transformation,
        }),
        "special" => out.push(Leaf {
            model: node.get("base").and_then(|m| m.as_str()).map(String::from),
            tints: Vec::new(),
            special: true,
            transformation,
        }),
        "composite" => {
            for m in node
                .get("models")
                .and_then(|m| m.as_array())
                .into_iter()
                .flatten()
            {
                collect_leaves(m, transformation, cast, out);
            }
        }
        "condition" => {
            let holds = cast && property_name(node) == Some("fishing_rod/cast");
            let branch = if holds { "on_true" } else { "on_false" };
            if let Some(n) = node.get(branch) {
                collect_leaves(n, transformation, cast, out);
            }
        }
        "select" => {
            let first_case = node
                .get("cases")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("model"));
            if let Some(n) = node.get("fallback").or(first_case) {
                collect_leaves(n, transformation, cast, out);
            }
        }
        "range_dispatch" => {
            // Property value 0 for a default stack: the entry with the largest threshold <= 0.
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
            if let Some(n) = entry.or(node.get("fallback")) {
                collect_leaves(n, transformation, cast, out);
            }
        }
        // `empty`, `bundle/selected_item`, unknown types: nothing to draw.
        _ => {}
    }
}
