//! Prints every entity type id in the data-generator reports (registry coverage check).
fn main() {
    let path = haste_core::paths::datagen_reports().join("registries.json");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("no registries.json at {}", path.display());
        return;
    };
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let Some(entries) = parsed["minecraft:entity_type"]["entries"].as_object() else {
        eprintln!("no entity_type registry");
        return;
    };
    let mut names: Vec<&str> = entries.keys().map(|s| s.as_str()).collect();
    names.sort_unstable();
    for n in names {
        println!("{n}");
    }
}
