//! The map renderer (`MapRenderer`) and the map texture manager (`MapTextureManager`): the
//! contents of a `filled_map` — one 128x128 texture per map id, its pixels from
//! [`super::map_color`], uploaded into the entity atlas when the map's data changed — plus
//! the decorations (player markers, banners, structure icons) over it, and the map's two
//! first-person poses (`FirstPersonHandsAndItemsRenderer`'s one- and two-handed maps).

use super::*;

/// One map decoration (`MapDecoration`): the type's `minecraft:map_decoration_type`
/// registry id and where it sits on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct MapMarker {
    /// The `minecraft:map_decoration_type` registry id (the packet's icon type).
    pub kind: u32,
    /// `MapDecoration.x`: -128..127, half-pixels from the map's centre (drawn at `x/2+64`).
    pub x: i8,
    /// `MapDecoration.y`.
    pub y: i8,
    /// `MapDecoration.rot`: 0..15, a 16-point compass (drawn at `rot * 360/16` degrees).
    pub rot: u8,
    /// The decoration's optional label, drawn under the icon (plain text; vanilla renders
    /// the name component's visual order).
    pub name: Option<String>,
}

/// One map's client-side data (`MapItemSavedData` as the packets fill it): the colour grid,
/// the decorations and a version that bumps on every change (the texture re-uploads only
/// then, like vanilla's `MapTextureManager.MapInstance.requiresUpload`).
#[derive(Clone, Debug, PartialEq)]
pub struct MapData {
    /// The map's scale 0..4 (`MapItemSavedData.scale`; kept for completeness, the pixels
    /// arrive already scaled).
    pub scale: i8,
    /// `MapItemSavedData.locked`.
    pub locked: bool,
    /// Bumped whenever the colours or decorations changed.
    pub version: u32,
    /// 128*128 packed map colours (`MapItemSavedData.colors`).
    pub colors: Vec<u8>,
    /// `MapItemSavedData.getDecorations`, in the packet's order.
    pub decorations: Vec<MapMarker>,
}

/// One map's atlas entry: where its texture sits and which data version it holds
/// (`MapTextureManager.MapInstance`).
pub(super) struct MapTexture {
    /// The atlas rectangle, `None` until first use or after a repack.
    pub(super) rect: Option<[u32; 4]>,
    /// The data version the texture was last uploaded from.
    pub(super) version: u32,
}

/// The whole map pose a framed map draws with (`ItemFrameRenderer.submit`'s map branch):
/// forward to the frame's face (`0.5` when invisible, else `0.4375`, like the item
/// transform), then `rotation % 4` quarter turns and vanilla's 180-degree flip, into the
/// map's 128x128 pixel space with the origin at the map's centre and the contents pushed
/// one pixel back.
pub(super) fn framed_map_transform(rotation: u8, invisible: bool) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, 0.0, if invisible { 0.5 } else { 0.4375 }))
        * Mat4::from_rotation_z(((rotation % 4) as f32 * 90.0).to_radians())
        * Mat4::from_rotation_z(180.0f32.to_radians())
        * Mat4::from_scale(Vec3::splat(1.0 / 128.0))
        * Mat4::from_translation(Vec3::new(-64.0, -64.0, 0.0))
        * Mat4::from_translation(Vec3::new(0.0, 0.0, -1.0))
}

/// `FirstPersonHandsAndItemsRenderer.calculateMapTilt`: how far the two-handed map tilts
/// down with the view pitch, eased over the 45 degrees around level.
pub(super) fn map_tilt(x_rot: f32) -> f32 {
    let tilt = (1.0 - x_rot / 45.0 + 0.1).clamp(0.0, 1.0);
    -(tilt * std::f32::consts::PI).cos() * 0.5 + 0.5
}

impl EntityRenderer {
    /// The maps to draw, published by the client when one changed (an `Arc` swap, no
    /// per-frame work). A drawn map whose id is missing draws nothing.
    pub fn set_maps(&mut self, maps: Arc<HashMap<i32, MapData>>) {
        self.maps = maps;
    }

    /// `minecraft:map_decoration_type` entries by protocol id (for the map decorations'
    /// sprites), like `set_item_names` for items.
    pub fn set_map_decoration_names(&mut self, names: Vec<Identifier>) {
        self.map_decoration_names = names;
        self.map_decoration_rects.clear();
    }

    /// The held map in first person, following vanilla's map branch of
    /// `FirstPersonHandsAndItemsRenderer.submitArmWithItem` (`itemStack.has(MAP_ID)`): the
    /// two-handed pose with both arms when the off hand is empty, else the one-handed pose
    /// with the swinging arm. Both end in `renderMap`: the map background (checkerboard
    /// behind received data, plain without) with the map renderer's contents over it. Only
    /// the main hand is modelled (like the rest of the first-person hand).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_held_map(
        &mut self,
        out: &mut Vec<EntityVertex>,
        root: Mat4,
        fp: &FirstPerson,
        view: (f32, f32),
        partial: f32,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let Some(id) = fp.map else {
            return;
        };
        let attack = fp.attack.at(partial);
        let sqrt_attack = attack.max(0.0).sqrt();
        let inverse_arm_height = 1.0 - fp.equip.at(partial);
        let mut m = root;
        if fp.offhand_empty {
            // `renderTwoHandedMap`.
            let y_swing_position = -0.2 * (attack * std::f32::consts::PI).sin();
            let z_swing_position = -0.4 * (sqrt_attack * std::f32::consts::PI).sin();
            m *= Mat4::from_translation(Vec3::new(0.0, -y_swing_position / 2.0, z_swing_position));
            let tilt = map_tilt(view.1);
            m *= Mat4::from_translation(Vec3::new(
                0.0,
                0.04 + inverse_arm_height * -1.2 + tilt * -0.5,
                -0.72,
            ));
            m *= Mat4::from_rotation_x((tilt * -85.0).to_radians());
            // Both arms hold the map up (`renderMapHand`, each behind a 90-degree yaw).
            let steve = Identifier::minecraft("entity/player/wide/steve");
            let skin = fp.skin.as_ref().unwrap_or(&steve).clone();
            let rect = self.texture_rect(&skin, assets);
            let arms = m * Mat4::from_rotation_y(90.0f32.to_radians());
            for left in [false, true] {
                let invert = if left { -1.0 } else { 1.0 };
                let hand = arms
                    * Mat4::from_rotation_y(92.0f32.to_radians())
                    * Mat4::from_rotation_x(45.0f32.to_radians())
                    * Mat4::from_rotation_z((invert * -41.0f32).to_radians())
                    * Mat4::from_translation(Vec3::new(invert * 0.3, -1.1, 0.45));
                let model = self.arm_model_side(fp.slim, fp.sleeve, left);
                let part = hand * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
                emit_part_tinted(
                    out,
                    &model,
                    &model.root,
                    &PoseOverrides::default(),
                    part,
                    rect,
                    light,
                    0.0,
                    [1.0; 3],
                );
            }
            let xz_swing_rotation = (sqrt_attack * std::f32::consts::PI).sin();
            m *= Mat4::from_rotation_x((xz_swing_rotation * 20.0).to_radians());
            m *= Mat4::from_scale(Vec3::splat(2.0));
        } else {
            // `renderOneHandedMap` (the main arm; a left main hand is not modelled yet).
            let invert = 1.0;
            m *= Mat4::from_translation(Vec3::new(invert * 0.125, -0.125, 0.0));
            let arm =
                m * Mat4::from_rotation_z((invert * 10.0).to_radians()) * arm_matrix(fp, partial);
            let steve = Identifier::minecraft("entity/player/wide/steve");
            let skin = fp.skin.as_ref().unwrap_or(&steve).clone();
            let rect = self.texture_rect(&skin, assets);
            let model = self.arm_model(fp.slim, fp.sleeve);
            let part = arm * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
            emit_part_tinted(
                out,
                &model,
                &model.root,
                &PoseOverrides::default(),
                part,
                rect,
                light,
                0.0,
                [1.0; 3],
            );
            m *= Mat4::from_translation(Vec3::new(
                invert * 0.51,
                -0.08 + inverse_arm_height * -1.2,
                -0.75,
            ));
            let x_swing = (sqrt_attack * std::f32::consts::PI).sin();
            let x_swing_position = -0.5 * x_swing;
            let y_swing_position = 0.4 * (sqrt_attack * std::f32::consts::PI * 2.0).sin();
            let z_swing_position = -0.3 * (attack * std::f32::consts::PI).sin();
            m *= Mat4::from_translation(Vec3::new(
                invert * x_swing_position,
                y_swing_position - 0.3 * x_swing,
                z_swing_position,
            ));
            m *= Mat4::from_rotation_x((x_swing * -45.0).to_radians());
            m *= Mat4::from_rotation_y((invert * x_swing * -30.0).to_radians());
        }
        // `renderMap`: into the map's pixel space, then the background quad (-7..135, the
        // 7-pixel border around the 128-pixel contents) with the contents over its middle.
        m *= Mat4::from_rotation_y(180.0f32.to_radians());
        m *= Mat4::from_rotation_z(180.0f32.to_radians());
        m *= Mat4::from_scale(Vec3::splat(0.38));
        m *= Mat4::from_translation(Vec3::new(-0.5, -0.5, 0.0));
        m *= Mat4::from_scale(Vec3::splat(1.0 / 128.0));
        let has_data = self.maps.contains_key(&id);
        let background = if has_data {
            Identifier::minecraft("map/map_background_checkerboard")
        } else {
            Identifier::minecraft("map/map_background")
        };
        let rect = self.texture_rect(&background, assets);
        push_flat_quad(
            out,
            m,
            [
                Vec3::new(-7.0, 135.0, 0.0),
                Vec3::new(135.0, 135.0, 0.0),
                Vec3::new(135.0, -7.0, 0.0),
                Vec3::new(-7.0, -7.0, 0.0),
            ],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            light,
            rect,
        );
        if has_data {
            self.emit_map(out, id, m, false, light, assets);
        }
    }

    /// One map's contents and decorations through `root` (the map's 128x128 pixel space),
    /// following vanilla `MapRenderer.render`: the colour quad at `z -0.01`, then each
    /// decoration at `x/2+64, y/2+64`, turned by its 16-point rotation, an 8-pixel icon
    /// with each further decoration one thousandth of a pixel behind the last. With
    /// `show_only_frame` only the types that also draw on a framed map appear (the held map
    /// shows all of them). An unresolvable decoration type (a mod's) draws nothing and logs
    /// once; nothing panics.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_map(
        &mut self,
        out: &mut Vec<EntityVertex>,
        id: i32,
        root: Mat4,
        show_only_frame: bool,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let maps = self.maps.clone();
        let Some(data) = maps.get(&id) else {
            return;
        };
        let rect = self.map_texture_rect(id, data);
        // Vanilla's quad: (0,128)-(128,0) at z -0.01, v flipped so image row 0 (north) is
        // the top of the drawn map.
        push_flat_quad(
            out,
            root,
            [
                Vec3::new(0.0, 128.0, -0.01),
                Vec3::new(128.0, 128.0, -0.01),
                Vec3::new(128.0, 0.0, -0.01),
                Vec3::new(0.0, 0.0, -0.01),
            ],
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            light,
            rect,
        );
        for (count, d) in data.decorations.iter().enumerate() {
            let skip = (show_only_frame && !self.map_decoration_on_frame(d.kind))
                || self.map_decoration_rect(d.kind, assets).is_none();
            if !skip {
                // `MapRenderer.render`'s decoration pose: onto the icon's centre, turned by
                // the 16-point rotation, scaled to an 8-pixel icon (the quad spans -4..4 in
                // map pixels), offset by vanilla's (-0.125, 0.125) and the per-count z.
                let m =
                    root * Mat4::from_translation(Vec3::new(
                        d.x as f32 / 2.0 + 64.0,
                        d.y as f32 / 2.0 + 64.0,
                        -0.02,
                    )) * Mat4::from_rotation_z((d.rot as f32 * 360.0 / 16.0).to_radians())
                        * Mat4::from_scale(Vec3::new(4.0, 4.0, 3.0))
                        * Mat4::from_translation(Vec3::new(-0.125, 0.125, 0.0));
                let z = count as f32 * -0.001;
                let sprite = self
                    .map_decoration_rect(d.kind, assets)
                    .unwrap_or([0, 0, 1, 1]);
                push_flat_quad(
                    out,
                    m,
                    [
                        Vec3::new(-1.0, 1.0, z),
                        Vec3::new(1.0, 1.0, z),
                        Vec3::new(1.0, -1.0, z),
                        Vec3::new(-1.0, -1.0, z),
                    ],
                    [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                    light,
                    sprite,
                );
                if let Some(name) = &d.name {
                    self.emit_map_decoration_name(out, root, d, name, light);
                }
            }
        }
    }

    /// A decoration's optional label under its icon (`MapRenderer.render`'s name branch):
    /// the text clamped to `25 / width` (at most 6/9), drawn flat white 4 pixels below the
    /// icon, each glyph like the sign text's quads but unshaded (vanilla's text render
    /// types carry no normal).
    fn emit_map_decoration_name(
        &mut self,
        out: &mut Vec<EntityVertex>,
        root: Mat4,
        d: &MapMarker,
        name: &str,
        light: (u8, u8),
    ) {
        let font = self.font.clone();
        let width = font.width_f32(name);
        if width <= 0.0 {
            return;
        }
        let scale = (25.0 / width).clamp(0.0, 6.0 / 9.0);
        let m =
            root * Mat4::from_translation(Vec3::new(
                d.x as f32 / 2.0 + 64.0 - width * scale / 2.0,
                d.y as f32 / 2.0 + 64.0 + 4.0,
                -0.025,
            )) * Mat4::from_scale(Vec3::new(scale, scale, 1.0));
        // Vanilla's `scale(z, -1)` plus the 0.1 text-space push land the glyphs at z -0.1.
        let mut x = 0.0f32;
        haste_assets::for_each_formatted(name, Default::default(), |ch, _fmt| {
            let glyph = font.glyph(ch);
            self.map_glyph_quad(out, m, glyph, x, -0.1, light);
            x += glyph.advance;
        });
    }

    /// One label glyph, positioned like the sign text's bitmap glyphs (page rectangle,
    /// `y + top`), but flat-shaded.
    fn map_glyph_quad(
        &mut self,
        out: &mut Vec<EntityVertex>,
        m: Mat4,
        glyph: haste_assets::Glyph,
        x: f32,
        z: f32,
        light: (u8, u8),
    ) {
        let (rect, draw, src, top) = match glyph.kind {
            haste_assets::GlyphKind::Empty | haste_assets::GlyphKind::Missing => return,
            haste_assets::GlyphKind::Bitmap {
                x: px,
                y: py,
                w,
                h,
                scale,
                top,
                ..
            } => {
                let Some(r) = self.glyph_page_rect(glyph.kind) else {
                    return;
                };
                (
                    r,
                    [w as f32 * scale, h as f32 * scale],
                    [
                        px as f32 / r[2] as f32,
                        py as f32 / r[3] as f32,
                        w as f32 / r[2] as f32,
                        h as f32 / r[3] as f32,
                    ],
                    top,
                )
            }
            haste_assets::GlyphKind::Hex { .. } => {
                let Some(r) = self.glyph_page_rect(glyph.kind) else {
                    return;
                };
                (
                    r,
                    [r[2] as f32 / 2.0, r[3] as f32 / 2.0],
                    [0.0, 0.0, 1.0, 1.0],
                    0.0,
                )
            }
        };
        if draw[0] <= 0.0 || draw[1] <= 0.0 || rect[2] == 0 || rect[3] == 0 {
            return;
        }
        let (x0, y0) = (x, top);
        let (x1, y1) = (x0 + draw[0], y0 + draw[1]);
        push_flat_quad(
            out,
            m,
            [
                Vec3::new(x0, y0, z),
                Vec3::new(x0, y1, z),
                Vec3::new(x1, y1, z),
                Vec3::new(x1, y0, z),
            ],
            [
                [src[0], src[1]],
                [src[0], src[1] + src[3]],
                [src[0] + src[2], src[1] + src[3]],
                [src[0] + src[2], src[1]],
            ],
            light,
            rect,
        );
    }

    /// Whether a decoration type also draws on a framed map (`showOnItemFrame`); false for
    /// types we cannot resolve (nothing draws, matching vanilla's null sprite).
    fn map_decoration_on_frame(&mut self, kind: u32) -> bool {
        match self.map_decoration_names.get(kind as usize) {
            Some(name) => {
                super::map_decoration_types::by_registry_name(name).is_some_and(|t| t.show_on_frame)
            }
            None => false,
        }
    }

    /// A decoration type's sprite in the entity atlas, resolved once per type
    /// (`textures/map/decorations/<asset>.png`, the `map_decorations` atlas's directory
    /// source). Unknown types warn once and stay sprite-less.
    fn map_decoration_rect(&mut self, kind: u32, assets: &mut Assets) -> Option<[u32; 4]> {
        if let Some(rect) = self.map_decoration_rects.get(&kind) {
            return *rect;
        }
        let rect = 'r: {
            let Some(name) = self.map_decoration_names.get(kind as usize) else {
                break 'r None;
            };
            let Some(t) = super::map_decoration_types::by_registry_name(name) else {
                break 'r None;
            };
            let id = Identifier::minecraft(&format!("map/decorations/{}", t.asset));
            Some(self.texture_rect(&id, assets))
        };
        if rect.is_none() {
            tracing::warn!("map decoration type {kind} has no sprite; not drawn");
        }
        self.map_decoration_rects.insert(kind, rect);
        rect
    }

    /// A map's 128x128 texture rectangle in the entity atlas, allocated on first use and
    /// re-uploaded only when the map's version changed (`MapInstance.updateTextureIfNeeded`,
    /// vanilla uploads the whole texture the same way). A full atlas repack drops the
    /// rectangle and the next call re-uploads it.
    fn map_texture_rect(&mut self, id: i32, data: &MapData) -> [u32; 4] {
        let up_to_date = |t: &MapTexture| t.rect.is_some_and(|_| t.version == data.version);
        if let Some(t) = self.map_textures.get(&id) {
            if up_to_date(t) {
                return t.rect.unwrap();
            }
        }
        let needs_slot = self.map_textures.get(&id).is_none_or(|t| t.rect.is_none());
        if needs_slot {
            let mut at = self.shelf.alloc_in(128, 128, ATLAS_SIZE);
            if at.is_none() {
                // Full: start over (textures re-upload on demand), the maps with them.
                self.repack_atlas();
                at = self.shelf.alloc_in(128, 128, ATLAS_SIZE);
            }
            let Some(at) = at else {
                return [0, 0, 1, 1];
            };
            self.map_textures.entry(id).or_insert(MapTexture {
                rect: None,
                version: u32::MAX,
            });
            self.map_textures.get_mut(&id).unwrap().rect = Some([at[0], at[1], 128, 128]);
        }
        let Some(t) = self.map_textures.get_mut(&id) else {
            return [0, 0, 1, 1];
        };
        let Some([x, y, _, _]) = t.rect else {
            return [0, 0, 1, 1];
        };
        let rect = [x, y, 128, 128];
        if t.version != data.version {
            t.version = data.version;
            // The pixel grid: one ARGB texel per packed colour. Rebuilt only when the
            // version moved, never per frame.
            let mut bytes = Vec::with_capacity(128 * 128 * 4);
            for py in 0..128 {
                for px in 0..128 {
                    let [r, g, b, a] =
                        super::map_color::color_from_packed_id(data.colors[px + py * 128]);
                    bytes.extend_from_slice(&[r, g, b, a]);
                }
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * 128),
                    rows_per_image: Some(128),
                },
                wgpu::Extent3d {
                    width: 128,
                    height: 128,
                    depth_or_array_layers: 1,
                },
            );
        }
        rect
    }
}
