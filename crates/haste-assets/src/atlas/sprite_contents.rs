// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See docs/THIRD_PARTY.md.
//! One sprite's level-0 contents as loaded from the packs (vanilla `SpriteContents`).
use haste_core::Identifier;

use crate::assets::{Animation, Assets};

use super::mipmap_strategy::mipmap_meta;
use super::MipmapStrategy;
pub(super) struct Source {
    pub(super) name: String,
    /// First-frame RGBA pixels (empty = failed to load).
    pub(super) data: Vec<u8>,
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) opaque: bool,
    pub(super) translucent: bool,
    /// Mip treatment (vanilla `MipmapStrategy`, `auto` resolved from pixels).
    pub(super) mipmap_strategy: MipmapStrategy,
    /// Alpha boost for the coverage match (vanilla `alpha_cutoff_bias`).
    pub(super) alpha_cutoff_bias: f32,
    /// The full animation, when the texture has an `.mcmeta` one.
    pub(super) animation: Option<Animation>,
}

/// Load a texture through `Assets`, reduced to its first animation frame
/// (Pomme `load_source`); the full animation is kept for `animated_sprites`.
pub(super) fn load_source(assets: &mut Assets, id: &Identifier) -> Source {
    let name = id.to_string();
    // `Assets::animation` reads the `.mcmeta` first and the PNG only when it
    // declares an animation; `texture_first_frame` is the static-path load.
    let animation = assets.animation(id);
    let image = match &animation {
        Some(a) => Ok(a.initial_frame().clone()),
        None => assets.texture_first_frame(id),
    };
    let Ok(image) = image else {
        tracing::warn!("Missing texture: {name}");
        return Source {
            name,
            data: Vec::new(),
            w: 0,
            h: 0,
            opaque: false,
            translucent: false,
            mipmap_strategy: MipmapStrategy::Mean,
            alpha_cutoff_bias: 0.0,
            animation: None,
        };
    };
    let (w, h) = image.dimensions();
    let data = image.into_raw();
    let (opaque, translucent) = sprite_transparency(&data);
    let (mipmap_strategy, alpha_cutoff_bias) = mipmap_meta(assets, id, &data);
    Source {
        name,
        data,
        w,
        h,
        opaque,
        translucent,
        mipmap_strategy,
        alpha_cutoff_bias,
        animation,
    }
}

/// (opaque, translucent) over every texel (Pomme `sprite_transparency`).
pub(super) fn sprite_transparency(data: &[u8]) -> (bool, bool) {
    data.as_chunks::<4>()
        .0
        .iter()
        .fold((true, false), |(opaque, translucent), px| {
            (
                opaque && px[3] == 255,
                translucent || (px[3] != 0 && px[3] != 255),
            )
        })
}
