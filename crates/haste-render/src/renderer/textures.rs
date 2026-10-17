//! Runtime texture registration on the renderer (vanilla `TextureManager.register`): downloaded
//! skins and capes become textures every atlas loads by id (see `haste_assets::dynamic`).
use std::sync::Arc;

use haste_core::Identifier;

use super::Renderer;

impl Renderer {
    /// Registers `image` as texture `id`; entities and the GUI pick it up on first use. A
    /// renderer rebuilt for a resource pack reload starts without them (register again).
    pub fn register_texture(&mut self, id: Identifier, image: Arc<image::RgbaImage>) {
        self.assets.register_texture(id, image);
    }

    /// Whether `id` was registered on this renderer.
    pub fn has_registered_texture(&self, id: &Identifier) -> bool {
        self.assets.has_registered_texture(id)
    }
}
