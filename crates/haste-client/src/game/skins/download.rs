//! `SkinTextureDownloader` and `SkinManager.TextureCache`'s naming: where a profile texture is
//! cached on disk and under which texture id it is registered, the download itself (local copy
//! first, else HTTP, then the copy is written), and the legacy 64x32 skin conversion.
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use haste_core::Identifier;
use image::RgbaImage;
use sha1::{Digest as _, Sha1};

/// `MinecraftProfileTexture.Type` as far as the cache cares (the texture id's folder).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureType {
    Skin,
    Cape,
    Elytra,
}

impl TextureType {
    /// `TextureCache.getTextureLocation`'s folder.
    fn folder(self) -> &'static str {
        match self {
            TextureType::Skin => "skins",
            TextureType::Cape => "capes",
            TextureType::Elytra => "elytra",
        }
    }
}

/// `TextureCache.registerTexture`: the SHA-1 of the texture hash (Guava's `hashUnencodedChars`
/// hashes the string's UTF-16 code units, little-endian), giving the texture id
/// `minecraft:<folder>/<sha1>` and the cache file `<root>/<first two hex digits>/<sha1>`.
pub fn cache_location(root: &Path, kind: TextureType, hash: &str) -> (Identifier, PathBuf) {
    let mut sha = Sha1::new();
    for unit in hash.encode_utf16() {
        sha.update(unit.to_le_bytes());
    }
    let hex: String = sha.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let dir = if hex.len() > 2 { &hex[..2] } else { "xx" };
    let file = root.join(dir).join(&hex);
    (
        Identifier::minecraft(&format!("{}/{hex}", kind.folder())),
        file,
    )
}

/// `SkinTextureDownloader.downloadAndRegisterSkin` minus the registration: the image from the
/// local copy, else from `url` (any non-2xx status fails; the copy is written next, a failed
/// write only logged), then the legacy conversion for skins.
pub async fn download(local: PathBuf, url: String, skin: bool) -> Result<RgbaImage> {
    let bytes = match tokio::fs::read(&local).await {
        Ok(b) => b,
        Err(_) => {
            tracing::debug!("downloading HTTP texture from {url} to {}", local.display());
            let response = reqwest::get(&url)
                .await
                .with_context(|| format!("opening {url}"))?;
            let status = response.status();
            if !status.is_success() {
                bail!("failed to open {url}, HTTP error code: {}", status.as_u16());
            }
            let body = response.bytes().await?.to_vec();
            let written = match local.parent() {
                Some(dir) => tokio::fs::create_dir_all(dir).await.is_ok(),
                None => true,
            };
            if !written || tokio::fs::write(&local, &body).await.is_err() {
                tracing::warn!("failed to cache texture {url} in {}", local.display());
            }
            body
        }
    };
    tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .with_context(|| format!("decoding texture {url}"))?
            .to_rgba8();
        if skin {
            process_legacy_skin(image, &url)
        } else {
            Ok(image)
        }
    })
    .await?
}

/// `SkinTextureDownloader.processLegacySkin`: only 64x64 and legacy 64x32 skins are kept. A
/// legacy skin grows to 64x64 with its right arm and leg mirrored into the left-limb slots;
/// the base layers are made opaque and a legacy hat that is fully opaque is cleared (the
/// "Notch transparency hack").
pub fn process_legacy_skin(image: RgbaImage, url: &str) -> Result<RgbaImage> {
    let (w, h) = image.dimensions();
    if w != 64 || (h != 32 && h != 64) {
        bail!("discarding incorrectly sized ({w}x{h}) skin texture from {url}");
    }
    let legacy = h == 32;
    let mut image = image;
    if legacy {
        let mut grown = RgbaImage::new(64, 64);
        image::imageops::replace(&mut grown, &image, 0, 0);
        image = grown;
        // (source x, source y, offset x, offset y, width, height), all mirrored in x
        for (x, y, dx, dy, w, h) in [
            (4, 16, 16, 32, 4, 4),
            (8, 16, 16, 32, 4, 4),
            (0, 20, 24, 32, 4, 12),
            (4, 20, 16, 32, 4, 12),
            (8, 20, 8, 32, 4, 12),
            (12, 20, 16, 32, 4, 12),
            (44, 16, -8, 32, 4, 4),
            (48, 16, -8, 32, 4, 4),
            (40, 20, 0, 32, 4, 12),
            (44, 20, -8, 32, 4, 12),
            (48, 20, -16, 32, 4, 12),
            (52, 20, -8, 32, 4, 12),
        ] {
            copy_rect_mirrored(&mut image, x, y, dx, dy, w, h);
        }
    }
    set_no_alpha(&mut image, 0, 0, 32, 16);
    if legacy {
        notch_transparency_hack(&mut image, 32, 0, 64, 32);
    }
    set_no_alpha(&mut image, 0, 16, 64, 32);
    set_no_alpha(&mut image, 16, 48, 48, 64);
    Ok(image)
}

/// `NativeImage.copyRect(x, y, dx, dy, w, h, swapX = true, swapY = false)` within one image.
fn copy_rect_mirrored(image: &mut RgbaImage, x: u32, y: u32, dx: i32, dy: i32, w: u32, h: u32) {
    for row in 0..h {
        for col in 0..w {
            let p = *image.get_pixel(x + col, y + row);
            let tx = (x as i32 + dx) as u32 + (w - 1 - col);
            let ty = (y as i32 + dy) as u32 + row;
            image.put_pixel(tx, ty, p);
        }
    }
}

/// `setNoAlpha`: alpha 255 over `[x0, x1) x [y0, y1)`.
fn set_no_alpha(image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) {
    for y in y0..y1 {
        for x in x0..x1 {
            image.get_pixel_mut(x, y).0[3] = 255;
        }
    }
}

/// `doNotchTransparencyHack`: when no pixel of the region is below alpha 128, the whole region
/// becomes fully transparent (old skins painted their hat layer solid).
fn notch_transparency_hack(image: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) {
    for x in x0..x1 {
        for y in y0..y1 {
            if image.get_pixel(x, y).0[3] < 128 {
                return;
            }
        }
    }
    for x in x0..x1 {
        for y in y0..y1 {
            image.get_pixel_mut(x, y).0[3] = 0;
        }
    }
}
