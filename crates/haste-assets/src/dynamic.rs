//! Textures registered at runtime instead of read from the jar or a pack (vanilla
//! `TextureManager.register` with a `DynamicTexture`): downloaded player skins and capes.
//! [`crate::Assets::texture_any`] answers them first, so every atlas that loads textures by id
//! (the entity atlas, the GUI atlas) picks them up on first use.
use std::sync::Arc;

use ahash::AHashMap;
use haste_core::Identifier;

/// The registered images, by texture id.
#[derive(Clone, Default)]
pub struct DynamicTextures {
    images: AHashMap<Identifier, Arc<image::RgbaImage>>,
}

impl DynamicTextures {
    pub fn register(&mut self, id: Identifier, image: Arc<image::RgbaImage>) {
        self.images.insert(id, image);
    }

    pub fn get(&self, id: &Identifier) -> Option<&Arc<image::RgbaImage>> {
        self.images.get(id)
    }

    pub fn contains(&self, id: &Identifier) -> bool {
        self.images.contains_key(id)
    }
}

impl crate::Assets {
    /// Registers `image` as the texture `id` (it shadows any jar or pack texture of that id).
    pub fn register_texture(&mut self, id: Identifier, image: Arc<image::RgbaImage>) {
        self.dynamic.register(id, image);
    }

    /// Whether `id` was registered with [`Self::register_texture`].
    pub fn has_registered_texture(&self, id: &Identifier) -> bool {
        self.dynamic.contains(id)
    }
}
