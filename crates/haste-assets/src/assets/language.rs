// Derived from Pomme (GPL-3.0); see docs/THIRD_PARTY.md.

use std::collections::HashMap;

use anyhow::{Context, Result};

use super::Assets;

impl Assets {
    /// `assets/minecraft/lang/<code>.json`.
    pub fn language(&mut self, code: &str) -> Result<HashMap<String, String>> {
        let key = format!("assets/minecraft/lang/{code}.json");
        let bytes = self.read_asset(&key)?;
        let map: HashMap<String, String> =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        Ok(map)
    }
}
