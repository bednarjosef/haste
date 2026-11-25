// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See docs/THIRD_PARTY.md.
//! Vanilla's `MipmapStrategy` and the `.mcmeta` `texture` section that picks it.
use serde::Deserialize;

use haste_core::Identifier;

use crate::assets::Assets;
/// How a sprite's mip chain treats transparent texels (vanilla
/// `MipmapStrategy` from the texture's `.mcmeta` `texture` section; `auto`
/// picks per the sprite's own alpha).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipmapStrategy {
    /// Average everything as-is (opaque or translucent sprites).
    Mean,
    /// Solidify first, then keep the alpha-test coverage of level 0 (cutout).
    Cutout,
    /// Like `Cutout` but against a stricter alpha reference of 0.3.
    StrictCutout,
    /// Transparent texels are filled with a darkened colour first (leaves).
    DarkCutout,
}

impl MipmapStrategy {
    /// Resolve `auto` from the sprite's pixels: any fully transparent texel
    /// makes it cutout (vanilla `Transparency.hasTransparent`).
    fn auto(data: &[u8]) -> Self {
        let has_transparent = data.as_chunks::<4>().0.iter().any(|px| px[3] == 0);
        if has_transparent {
            MipmapStrategy::Cutout
        } else {
            MipmapStrategy::Mean
        }
    }

    /// Whether the strategy treats the sprite as cutout (public for the
    /// renderer's animated-sprite updates).
    pub fn is_cutout(&self) -> bool {
        !matches!(self, MipmapStrategy::Mean)
    }

    /// The alpha-test reference the coverage match aims for (vanilla
    /// `cutoutRef`: strict uses 0.3, the rest 0.5).
    pub fn cutout_ref(&self) -> f32 {
        if matches!(self, MipmapStrategy::StrictCutout) {
            0.3
        } else {
            0.5
        }
    }
}

/// `texture` section of a texture's `.mcmeta` (vanilla `TextureMetadataSection`).
#[derive(Deserialize, Default)]
struct TextureMipmapMeta {
    #[serde(default)]
    texture: Option<TextureMipmapMetaTexture>,
}

#[derive(Deserialize, Default)]
struct TextureMipmapMetaTexture {
    #[serde(default)]
    mipmap_strategy: Option<String>,
    #[serde(default)]
    alpha_cutoff_bias: Option<f32>,
}

/// The mip strategy and alpha cutoff bias of a texture, from its `.mcmeta`
/// `texture` section (vanilla `SpriteContents` reads the same). `auto`
/// resolves per the sprite's pixels.
pub(super) fn mipmap_meta(
    assets: &mut Assets,
    id: &Identifier,
    data: &[u8],
) -> (MipmapStrategy, f32) {
    let fallback = (MipmapStrategy::auto(data), 0.0);
    let Ok(Some(bytes)) =
        assets.try_read_asset(&crate::assets::asset_key(id, "textures", ".png.mcmeta"))
    else {
        return fallback;
    };
    let Ok(meta) = serde_json::from_slice::<TextureMipmapMeta>(&bytes) else {
        return fallback;
    };
    let texture = meta.texture.unwrap_or_default();
    let strategy = match texture.mipmap_strategy.as_deref() {
        Some("mean") => MipmapStrategy::Mean,
        Some("cutout") => MipmapStrategy::Cutout,
        Some("strict_cutout") => MipmapStrategy::StrictCutout,
        Some("dark_cutout") => MipmapStrategy::DarkCutout,
        _ => MipmapStrategy::auto(data),
    };
    (strategy, texture.alpha_cutoff_bias.unwrap_or(0.0))
}
