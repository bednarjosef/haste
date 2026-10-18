//! Reading one key from every layer of the stack (vanilla
//! `FallbackResourceManager.getResourceStack`, without the jar): used by the atlas
//! sprite lookup (`crate::assets::sprite_sources`), which stacks every pack's atlas
//! definition on the jar's own.

impl super::PackStack {
    /// Every layer's copy of `assets/<ns>/<path>`, bottom first (the layer order of
    /// [`super::PackStack::open`]; the jar's own copy is not included).
    pub(crate) fn read_all_layers(&mut self, key: &str) -> Vec<Vec<u8>> {
        self.packs
            .iter_mut()
            .filter_map(|p| p.file.read(key))
            .collect()
    }
}
