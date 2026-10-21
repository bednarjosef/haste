//! The entity texture atlas: textures and font pages uploaded on first use into the shared
//! 2048x2048 atlas (a shelf packer hands out rectangles; a full atlas repacks).

use super::*;

impl EntityRenderer {
    /// A font page's rectangle in the entity atlas (page images are 256x256, uploaded on first
    /// use like entity textures). Unihex glyphs are rasterised to white bitmaps and uploaded
    /// the same way, keyed by their index.
    pub(super) fn glyph_page_rect(&mut self, kind: haste_assets::GlyphKind) -> Option<[u32; 4]> {
        let key = match kind {
            haste_assets::GlyphKind::Empty | haste_assets::GlyphKind::Missing => return None,
            haste_assets::GlyphKind::Bitmap { page, .. } => {
                Identifier::new("haste", &format!("font/{page}"))
            }
            haste_assets::GlyphKind::Hex { index } => {
                Identifier::new("haste", &format!("hex/{index}"))
            }
        };
        if !self.textures.contains_key(&key) {
            let image = match kind {
                haste_assets::GlyphKind::Bitmap { page, .. } => {
                    self.font.page(page as usize)?.as_ref().clone()
                }
                haste_assets::GlyphKind::Hex { index, .. } => {
                    crate::gui::hex_image_public(self.font.hex_glyph(index)?)
                }
                _ => return None,
            };
            self.textures.insert(
                key.clone(),
                EntityTexture {
                    rect: None,
                    image: Arc::new(image),
                },
            );
        }
        // Already resident (or freshly inserted): make it so and return the rectangle.
        Some(self.resident_texture_rect(&key))
    }

    pub(super) fn texture_rect(&mut self, id: &Identifier, assets: &mut Assets) -> [u32; 4] {
        if !self.textures.contains_key(id) {
            let image = if id.namespace() == "haste" {
                if id.path() == "missing" {
                    missing_texture()
                } else if id.path() == "white" {
                    white_texture()
                } else {
                    placeholder_texture()
                }
            } else {
                assets.texture_any(id).unwrap_or_else(|e| {
                    tracing::warn!("entity texture {id}: {e:#}");
                    placeholder_texture()
                })
            };
            self.textures.insert(
                id.clone(),
                EntityTexture {
                    rect: None,
                    image: Arc::new(image),
                },
            );
        }
        let t = self.textures.get_mut(id).unwrap();
        if let Some(r) = t.rect {
            return r;
        }
        self.resident_texture_rect(id)
    }

    /// [`Self::texture_rect`] for animated textures: the first frame of the `.mcmeta`
    /// animation, not the whole sheet (vanilla's atlas sprites play one frame at a time;
    /// the entity atlas has no animation ticker, so the fire overlay and friends draw the
    /// first frame, like the fluid and particle atlases).
    pub(super) fn texture_rect_first_frame(
        &mut self,
        id: &Identifier,
        assets: &mut Assets,
    ) -> [u32; 4] {
        if !self.textures.contains_key(id) {
            let image = assets.texture_first_frame(id).unwrap_or_else(|e| {
                tracing::warn!("entity texture {id}: {e:#}");
                placeholder_texture()
            });
            self.textures.insert(
                id.clone(),
                EntityTexture {
                    rect: None,
                    image: Arc::new(image),
                },
            );
        }
        self.texture_rect(id, assets)
    }

    /// Uploads `id`'s stored image into the atlas if it is not resident yet and returns its
    /// rectangle. The entry must exist (insert by `texture_rect` / `glyph_page_rect` first).
    pub(super) fn resident_texture_rect(&mut self, id: &Identifier) -> [u32; 4] {
        let t = self.textures.get_mut(id).unwrap();
        if let Some(r) = t.rect {
            return r;
        }
        let (w, h) = t.image.dimensions();
        let at = match self.shelf.alloc_in(w, h, ATLAS_SIZE) {
            Some(at) => at,
            None => {
                // Full: start over (textures re-upload on demand).
                self.repack_atlas();
                match self.shelf.alloc_in(w, h, ATLAS_SIZE) {
                    Some(at) => at,
                    None => return [0, 0, 1, 1],
                }
            }
        };
        let t = self.textures.get_mut(id).unwrap();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: at[0],
                    y: at[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            t.image.as_raw(),
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
        let r = [at[0], at[1], w, h];
        t.rect = Some(r);
        r
    }

    /// Starts the atlas shelf over (a full atlas): every entity texture, map texture and map
    /// decoration sprite forgets its rectangle and re-uploads on demand.
    pub(super) fn repack_atlas(&mut self) {
        tracing::warn!("entity atlas full; repacking");
        self.shelf = Shelf::default();
        for t in self.textures.values_mut() {
            t.rect = None;
        }
        for t in self.map_textures.values_mut() {
            t.rect = None;
        }
        self.map_decoration_rects.clear();
    }
}
