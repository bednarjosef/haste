use super::*;

fn leaf_models(json: &str, cast: bool) -> Vec<String> {
    let node: serde_json::Value = serde_json::from_str(json).unwrap();
    let mut out = Vec::new();
    collect_leaves(&node, None, cast, &mut out);
    out.into_iter().filter_map(|l| l.model).collect()
}

const ROD: &str = r#"{
    "type": "minecraft:condition",
    "property": "minecraft:fishing_rod/cast",
    "on_false": {"type": "minecraft:model", "model": "minecraft:item/fishing_rod"},
    "on_true": {"type": "minecraft:model", "model": "minecraft:item/fishing_rod_cast"}
}"#;

#[test]
fn fishing_rod_cast_takes_on_true_only_when_the_hook_is_out() {
    assert_eq!(leaf_models(ROD, true), ["minecraft:item/fishing_rod_cast"]);
    assert_eq!(leaf_models(ROD, false), ["minecraft:item/fishing_rod"]);
}

#[test]
fn other_conditions_keep_on_false_whatever_the_cast_flag() {
    let other = ROD.replace("fishing_rod/cast", "using_item");
    assert_eq!(leaf_models(&other, true), ["minecraft:item/fishing_rod"]);
}
