//! The lazily filled GUI atlas: texture keys and entries, the shelf packer (vanilla
//! `DynamicAtlasAllocator`), residency, animated textures and the generated images.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use hashbrown::Equivalent;
use haste_assets::{frame_at, Animation, Assets, SpriteMeta};
use haste_core::Identifier;

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TexKey {
    /// A gui atlas sprite (`hud/hotbar`, `mob_effect/regeneration`, ...), wherever
    /// the atlas definition's directory sources put its texture.
    Sprite(Identifier),
    /// A `GuiCommand::Texture` texture (full `textures/...png` path or short id).
    Texture(Identifier),
    /// A texture an item model samples (`minecraft:block/stone`, `minecraft:item/apple`).
    ItemTexture(String),
    FontPage(u16),
    Hex(u32),
    Missing,
}

/// A [`TexKey`] for map probes. Sprite and texture lookups run once per GUI command,
/// so they must not allocate an owned `Identifier` just to probe the key map; the
/// owned key is only built on a miss, when the entry is inserted.
pub(super) enum TexKeyRef<'a> {
    /// Probes a `TexKey::Sprite`.
    Sprite(&'a Identifier),
    /// Probes a `TexKey::Texture`.
    Texture(&'a Identifier),
    /// Probes any owned key (the cold paths: font pages, unihex glyphs, item textures).
    Key(&'a TexKey),
}

impl TexKeyRef<'_> {
    /// The owned key this probe stands for (the miss path's insert).
    fn owned(&self) -> TexKey {
        match *self {
            TexKeyRef::Sprite(id) => TexKey::Sprite(id.clone()),
            TexKeyRef::Texture(id) => TexKey::Texture(id.clone()),
            TexKeyRef::Key(k) => k.clone(),
        }
    }
}

/// The owned key's hash; [`TexKeyRef`]'s mirrors it tag for tag (see its comment).
impl Hash for TexKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            TexKey::Sprite(id) => {
                state.write_u8(0);
                id.hash(state);
            }
            TexKey::Texture(id) => {
                state.write_u8(1);
                id.hash(state);
            }
            TexKey::ItemTexture(s) => {
                state.write_u8(2);
                s.hash(state);
            }
            TexKey::FontPage(p) => {
                state.write_u8(3);
                p.hash(state);
            }
            TexKey::Hex(i) => {
                state.write_u8(4);
                i.hash(state);
            }
            TexKey::Missing => state.write_u8(5),
        }
    }
}

// A borrowed probe must hash exactly like the owned key it stands for, so both hash
// through the same tag; reordering the variants would otherwise silently mismatch
// (wrong or missing textures), not fail to compile.
impl Hash for TexKeyRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            TexKeyRef::Sprite(id) => {
                state.write_u8(0);
                id.hash(state);
            }
            TexKeyRef::Texture(id) => {
                state.write_u8(1);
                id.hash(state);
            }
            TexKeyRef::Key(k) => k.hash(state),
        }
    }
}

impl Equivalent<TexKey> for TexKeyRef<'_> {
    fn equivalent(&self, key: &TexKey) -> bool {
        match (self, key) {
            (TexKeyRef::Sprite(a), TexKey::Sprite(b))
            | (TexKeyRef::Texture(a), TexKey::Texture(b)) => *a == b,
            (TexKeyRef::Key(a), b) => *a == b,
            _ => false,
        }
    }
}

pub(super) struct TexEntry {
    /// The pixels while waiting for `rect` (dropped once uploaded: the GPU atlas is the
    /// only copy; `dimensions` keeps what the UV math needs). Animated entries keep
    /// theirs — `animate` re-uploads the current frame from them each tick.
    pub(super) image: Option<Arc<image::RgbaImage>>,
    /// The image's size (kept after the upload; the UV math in `image` commands needs
    /// it to scale face-local coordinates to the atlas rectangle).
    pub(super) dims: (u32, u32),
    pub(super) meta: SpriteMeta,
    /// Atlas rectangle when resident.
    pub(super) rect: Option<[u32; 4]>,
    /// The `.mcmeta` animation of an animated texture (fire, nether portal); the atlas holds
    /// one frame and the current frame is re-uploaded each game tick. `None` when static.
    pub(super) animation: Option<Animation>,
    /// The animation frame currently uploaded to `rect` (re-uploaded when it changes).
    pub(super) frame: u32,
}

/// Shelf packer over a square atlas.
#[derive(Default)]
pub(crate) struct Shelf {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) row_h: u32,
}

impl Shelf {
    pub(crate) fn alloc_in(&mut self, w: u32, h: u32, size: u32) -> Option<[u32; 2]> {
        if w > size || h > size {
            return None;
        }
        if self.x + w > size {
            self.x = 0;
            self.y += self.row_h;
            self.row_h = 0;
        }
        if self.y + h > size {
            return None;
        }
        let at = [self.x, self.y];
        self.x += w;
        self.row_h = self.row_h.max(h);
        Some(at)
    }
}

impl GuiRenderer {
    pub(super) fn insert_entry(
        &mut self,
        key: TexKey,
        image: Arc<image::RgbaImage>,
        meta: SpriteMeta,
        animation: Option<Animation>,
    ) -> u32 {
        let index = self.entries.len() as u32;
        let dims = image.dimensions();
        self.entries.push(TexEntry {
            image: Some(image),
            dims,
            meta,
            rect: None,
            animation,
            frame: 0,
        });
        self.keys.insert(key, index);
        index
    }

    /// Entry index for a key, loading the image on first use (missing texture on failure).
    pub(super) fn entry(&mut self, key: &TexKey, assets: &mut Assets) -> u32 {
        if let Some(&i) = self.keys.get(key) {
            return i;
        }
        let loaded: anyhow::Result<(image::RgbaImage, SpriteMeta, Option<Animation>)> = match key {
            TexKey::Sprite(id) => assets.gui_sprite(id).map(|(i, m)| (i, m, None)),
            TexKey::Texture(id) => {
                // An animated texture (fire, nether portal): pack one frame and re-upload the
                // current frame each tick, like vanilla's sprite animation.
                match assets.animation(id) {
                    Some(anim) => Ok((
                        anim.initial_frame().clone(),
                        SpriteMeta::Stretch,
                        Some(anim),
                    )),
                    None => assets
                        .texture_any(id)
                        .map(|i| (i, SpriteMeta::Stretch, None)),
                }
            }
            TexKey::ItemTexture(id) => Identifier::parse(id)
                .map_err(anyhow::Error::from)
                .and_then(|id| assets.texture_first_frame(&id))
                .map(|i| (i, SpriteMeta::Stretch, None)),
            TexKey::FontPage(p) => self
                .font
                .page(*p as usize)
                .map(|img| (img.as_ref().clone(), SpriteMeta::Stretch, None))
                .ok_or_else(|| anyhow::anyhow!("no font page {p}")),
            TexKey::Hex(index) => self
                .font
                .hex_glyph(*index)
                .map(|g| (hex_image(g), SpriteMeta::Stretch, None))
                .ok_or_else(|| anyhow::anyhow!("no unihex glyph {index}")),
            TexKey::Missing => Ok((missing_image(), SpriteMeta::Stretch, None)),
        };
        match loaded {
            Ok((image, meta, animation)) => {
                self.insert_entry(key.clone(), Arc::new(image), meta, animation)
            }
            Err(e) => {
                tracing::warn!("GUI texture {key:?} unavailable: {e:#}");
                self.keys.insert(key.clone(), 0);
                0
            }
        }
    }

    /// Atlas rectangle of an entry, uploading it if it is not resident.
    pub(super) fn resident(&mut self, index: u32) -> [u32; 4] {
        let e = &self.entries[index as usize];
        if let Some(r) = e.rect {
            return r;
        }
        let (w, h) = e.dims;
        let (w1, h1) = (w.max(1), h.max(1));
        let mut at = self.shelf.alloc_in(w1, h1, self.atlas_size);
        while at.is_none() && self.grow_atlas() {
            at = self.shelf.alloc_in(w1, h1, self.atlas_size);
        }
        let Some([x, y]) = at else {
            self.atlas_full = true;
            return self.entries[0].rect.unwrap_or([0, 0, 0, 0]);
        };
        if w > 0 && h > 0 {
            let e = &self.entries[index as usize];
            let pixels = e
                .image
                .as_deref()
                .map(|i| i.as_raw().clone())
                .unwrap_or_else(|| vec![0u8; (w * h * 4) as usize]);
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let rect = [x, y, w, h];
        let e = &mut self.entries[index as usize];
        e.rect = Some(rect);
        // The initial frame just went in; `animate` re-uploads a different current frame.
        e.frame = e
            .animation
            .as_ref()
            .and_then(|a| a.schedule.frames.first().map(|f| f.index))
            .unwrap_or(0);
        // The GPU atlas is now the only copy a static entry needs: drop the CPU-side
        // pixels (animated entries keep theirs for the per-tick re-upload).
        if e.animation.is_none() {
            e.image = None;
        }
        rect
    }

    /// Looks a key up by reference (no `Identifier` clone per command), only building
    /// the owned key when the entry is missing.
    pub(super) fn lookup_ref(
        &mut self,
        key: TexKeyRef<'_>,
        assets: &mut Assets,
    ) -> ([u32; 4], u32) {
        let i = match self.keys.get(&key) {
            Some(&i) => i,
            None => self.entry(&key.owned(), assets),
        };
        (self.resident(i), i)
    }

    /// Re-uploads the current frame of every resident animated texture (fire, nether portal)
    /// when the tick's frame changed, like vanilla's sprite animation.
    pub(super) fn animate(&mut self) {
        let tick = self.animation_tick;
        for i in 0..self.entries.len() {
            let Some(rect) = self.entries[i].rect else {
                continue;
            };
            let e = &self.entries[i];
            let Some(anim) = &e.animation else {
                continue;
            };
            // The screen-effect animations (fire, nether portal) do not interpolate, so the
            // exact frame is what vanilla shows (`frame_at`'s blend is 0 for them).
            let (index, _, _) = frame_at(&anim.schedule, tick);
            if e.frame == index {
                continue;
            }
            let Some(frame) = anim.frames.get(index as usize) else {
                continue;
            };
            let pixels = frame.as_raw();
            let (w, h) = (rect[2], rect[3]);
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: rect[0],
                        y: rect[1],
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            self.entries[i].frame = index;
        }
    }

    /// Doubles the atlas texture, keeping its contents at the same texel positions (the
    /// shader reads texels, so every resident rectangle stays valid). False at the
    /// largest size.
    fn grow_atlas(&mut self) -> bool {
        if self.atlas_size >= GUI_ATLAS_SIZE {
            return false;
        }
        let size = self.atlas_size * 2;
        let atlas = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui-atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // Queue writes to the old atlas run before this submit, so the copy carries them.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gui-atlas-grow"),
            });
        encoder.copy_texture_to_texture(
            self.atlas.as_image_copy(),
            atlas.as_image_copy(),
            wgpu::Extent3d {
                width: self.atlas_size,
                height: self.atlas_size,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        self.bind_group = gui_renderer::gui_bind_group(
            &self.device,
            &self.bind_layout,
            &self.uniforms,
            &atlas.create_view(&Default::default()),
            &self.glint,
        );
        self.atlas = atlas;
        self.atlas_size = size;
        tracing::debug!("GUI atlas grown to {size}x{size}");
        true
    }

    pub(super) fn reset_atlas(&mut self) {
        self.shelf = Shelf::default();
        for e in &mut self.entries {
            e.rect = None;
        }
        self.atlas_full = false;
        // Keep the missing texture resident first.
        self.resident(0);
    }

    /// Pre-load sprites the HUD needs every frame so the first frame does not hitch.
    pub fn preload_sprites(&mut self, assets: &mut Assets, sprites: &[&str]) {
        for s in sprites {
            let key = TexKey::Sprite(Identifier::minecraft(s));
            let i = self.entry(&key, assets);
            self.resident(i);
        }
    }

    // ---- frame building ----
}
/// Vanilla's missing texture: 16x16 magenta/black checker of 8x8 squares.
pub(super) fn missing_image() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 16, |x, y| {
        if (x < 8) ^ (y < 8) {
            image::Rgba([248, 0, 248, 255])
        } else {
            image::Rgba([0, 0, 0, 255])
        }
    })
}

/// A unihex glyph's trimmed columns as a white-on-transparent image (16 rows).
pub(super) fn hex_image(g: &haste_assets::HexGlyph) -> image::RgbaImage {
    let w = (g.trimmed as u32).max(1);
    image::RgbaImage::from_fn(w, 16, |x, y| {
        if g.pixel(g.left as u32 + x, y) {
            image::Rgba([255, 255, 255, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    })
}

/// [`hex_image`] for callers outside the GUI (the entity renderer's sign text).
pub(crate) fn hex_image_public(g: &haste_assets::HexGlyph) -> image::RgbaImage {
    hex_image(g)
}
