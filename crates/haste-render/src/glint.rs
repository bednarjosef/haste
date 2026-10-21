//! The enchantment glint (vanilla's `*_glint` render types in `RenderTypes`, the `GLINT`
//! shader define of `RenderPipelines.GLINT_SNIPPET` and `TextureTransform`'s glint texturing).
//!
//! In 26.3 the glint is not a second pass: the item, entity and armour pipelines with the
//! `GLINT` define sample `GlintSampler` in the same draw and add it to the lit colour
//! (`item.fsh`: `glint = GlintAlpha * texture(GlintSampler, texCoordGlint)`,
//! `color.rgb += glint.rgb * glint.rgb`, the shader form of `BlendFunction.GLINT`, before the
//! fog; and `color.a = max(color.a, GlintAlpha)`). Haste does the same: a glinting vertex
//! carries a [`GlintMode`] (in `EntityVertex::light[2]` and `GuiVertex::glint`), and the
//! entity and GUI shaders add the glint for those vertices inside their one existing draw, so
//! the glint costs no extra draw call, no extra vertices and no per-frame allocation.
//!
//! The glint coordinates are the vertex's texture coordinates in vanilla's own space through
//! `TextureTransform.setupGlintTexturing(scale)`: translate by the two scrolling layer
//! offsets, rotate by pi/18, scale by 8 for items (`GLINT_TEXTURING`), 0.5 for entity models
//! (`ENTITY_GLINT_TEXTURING`) and 0.16 for armour (`ARMOR_ENTITY_GLINT_TEXTURING`). An item
//! sprite's coordinates in vanilla are in its atlas (`textures/atlas/items.png` for `item/`
//! sprites, `blocks.png` for block textures), so the glint's size on an item follows that
//! atlas's size; an entity or armour texture is its own texture (0..1).

use std::sync::Arc;

use glam::{Mat4, Vec3};
use haste_assets::Assets;
use haste_core::model::FoilType;
use haste_core::Identifier;

use crate::entity::EntityVertex;

/// The entity shader with the glint's coordinate code appended (`glint.wgsl`), for every
/// pipeline built from `entity.wgsl`.
pub const ENTITY_SHADER: &str = concat!(include_str!("entity.wgsl"), include_str!("glint.wgsl"));
/// The GUI shader with the glint's coordinate code appended.
pub const GUI_SHADER: &str = concat!(include_str!("gui.wgsl"), include_str!("glint.wgsl"));

/// The item glint texture (`ItemFeatureRenderer.ENCHANTED_GLINT_ITEM`).
pub const ENCHANTED_GLINT_ITEM: &str = "textures/misc/enchanted_glint_item.png";
/// The armour glint texture (`ItemFeatureRenderer.ENCHANTED_GLINT_ARMOR`).
pub const ENCHANTED_GLINT_ARMOR: &str = "textures/misc/enchanted_glint_armor.png";

/// `TextureTransform.MAX_ENCHANTMENT_GLINT_SPEED_MILLIS`: the glint clock runs at
/// `glintSpeed * 8` milliseconds per millisecond.
pub const MAX_ENCHANTMENT_GLINT_SPEED_MILLIS: f64 = 8.0;
/// `GLINT_TEXTURING`'s scale (items).
pub const ITEM_GLINT_SCALE: f32 = 8.0;
/// `ENTITY_GLINT_TEXTURING`'s scale (shields, tridents).
pub const ENTITY_GLINT_SCALE: f32 = 0.5;
/// `ARMOR_ENTITY_GLINT_TEXTURING`'s scale (worn armour).
pub const ARMOR_GLINT_SCALE: f32 = 0.16;
/// The glint's rotation, `rotateZ(PI / 18)`.
pub const GLINT_ROTATION: f32 = std::f32::consts::PI / 18.0;

/// Vanilla's item atlas size with the vanilla resources (`textures/atlas/items.png-atlas`,
/// 1024x512 as 26.3 creates it): the space an `item/` sprite's coordinates are in.
pub const ITEMS_SHEET_SIZE: [f32; 2] = [1024.0, 512.0];
/// Vanilla's block atlas size (`textures/atlas/blocks.png-atlas`, 2048x2048 in 26.3): the
/// space a block texture on a block item is in.
pub const BLOCKS_SHEET_SIZE: [f32; 2] = [2048.0, 2048.0];

/// The default `glintSpeed` and `glintStrength` options (`Options`: 0.5 and 0.75).
pub const DEFAULT_GLINT_SPEED: f64 = 0.5;
pub const DEFAULT_GLINT_STRENGTH: f64 = 0.75;

/// Which glint a vertex draws, by the texture space its coordinates are in and the glint
/// texturing (`EntityVertex::light[2]`, `GuiVertex::glint`). The shaders' `glint.wgsl` reads
/// the same numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum GlintMode {
    /// No glint.
    None = 0,
    /// An item quad on an `item/` sprite (`GLINT_TEXTURING` over the items sheet).
    ItemsSheet = 1,
    /// An item quad on a block texture (`GLINT_TEXTURING` over the blocks sheet).
    BlocksSheet = 2,
    /// An entity model drawn as an item (`entitySolidGlint`, `ENTITY_GLINT_TEXTURING`).
    Entity = 3,
    /// Worn armour (`armorCutoutNoCullGlint`, `ARMOR_ENTITY_GLINT_TEXTURING`, the armour
    /// glint texture).
    Armor = 4,
}

/// The glint mode of an item quad drawn with `foil` from `texture` (a sprite id such as
/// `minecraft:item/diamond_sword`): vanilla's `BakedQuad.MaterialInfo` picks the block or the
/// item sheet's glint render type by the sprite's atlas, and the items atlas holds the
/// `item/` directory and the armour trim item sprites (`atlases/items.json`); a special foil
/// draws through the same texturing here (its projected coordinates are not modelled).
pub fn item_glint_mode(foil: FoilType, texture: &str) -> GlintMode {
    if !foil.has_foil() {
        return GlintMode::None;
    }
    let path = texture.split_once(':').map_or(texture, |(_, p)| p);
    if path.starts_with("item/") || path.starts_with("trims/items/") {
        GlintMode::ItemsSheet
    } else {
        GlintMode::BlocksSheet
    }
}

/// Marks every vertex in `vertices` with `mode` (the vertex range one glinting draw emitted).
pub fn mark(vertices: &mut [EntityVertex], mode: GlintMode) {
    for v in vertices {
        v.light[2] = mode as u16;
    }
}

/// `TextureTransform.setupGlintTexturing`'s two layer offsets at `millis` (vanilla
/// `Util.getMillis()`) with the `glintSpeed` option: the clock in whole milliseconds
/// (`(long)(millis * glintSpeed * 8.0)`), then its phase in the 110 s and 30 s cycles.
pub fn glint_layer_offsets(millis: u64, glint_speed: f64) -> [f32; 2] {
    let t = (millis as f64 * glint_speed * MAX_ENCHANTMENT_GLINT_SPEED_MILLIS) as i64;
    [
        (t % 110_000) as f32 / 110_000.0,
        (t % 30_000) as f32 / 30_000.0,
    ]
}

/// The glint texture matrix vanilla builds (`new Matrix4f().translation(-offset0, offset1, 0)
/// .rotateZ(PI / 18).scale(scale)`), for the tests and as the reference the shaders follow.
pub fn glint_texture_matrix(scale: f32, offsets: [f32; 2]) -> Mat4 {
    Mat4::from_translation(Vec3::new(-offsets[0], offsets[1], 0.0))
        * Mat4::from_rotation_z(GLINT_ROTATION)
        * Mat4::from_scale(Vec3::splat(scale))
}

/// The glint coordinates of a vertex at atlas texel `uv` inside `rect` drawn with `mode`: the
/// CPU mirror of `glint.wgsl`'s `glint_coords` (the shaders compute them per vertex), kept
/// here so the tests hold the shader's formula against vanilla's matrix.
pub fn glint_coords(
    mode: GlintMode,
    uv: [f32; 2],
    rect: [u32; 4],
    params: &GlintParams,
) -> [f32; 2] {
    let local = [uv[0] - rect[0] as f32, uv[1] - rect[1] as f32];
    let (base, scale) = match mode {
        GlintMode::None => return [0.0, 0.0],
        GlintMode::ItemsSheet => (
            [
                local[0] / ITEMS_SHEET_SIZE[0],
                local[1] / ITEMS_SHEET_SIZE[1],
            ],
            ITEM_GLINT_SCALE,
        ),
        GlintMode::BlocksSheet => (
            [
                local[0] / BLOCKS_SHEET_SIZE[0],
                local[1] / BLOCKS_SHEET_SIZE[1],
            ],
            ITEM_GLINT_SCALE,
        ),
        GlintMode::Entity | GlintMode::Armor => (
            [local[0] / rect[2] as f32, local[1] / rect[3] as f32],
            if mode == GlintMode::Entity {
                ENTITY_GLINT_SCALE
            } else {
                ARMOR_GLINT_SCALE
            },
        ),
    };
    let s = [base[0] * scale, base[1] * scale];
    let (n, c) = GLINT_ROTATION.sin_cos();
    [
        c * s[0] - n * s[1] + params.translate[0],
        n * s[0] + c * s[1] + params.translate[1],
    ]
}

/// The glint uniform the entity and GUI shaders read (`glint.wgsl`'s `GlintParams`): the
/// texture matrix's translation and `GlintAlpha` (the `glintStrength` option,
/// `GlobalSettingsUniform`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlintParams {
    pub translate: [f32; 2],
    pub alpha: f32,
    pub pad: f32,
}

impl GlintParams {
    pub fn at(millis: u64, glint_speed: f64, glint_strength: f64) -> GlintParams {
        let o = glint_layer_offsets(millis, glint_speed);
        GlintParams {
            translate: [-o[0], o[1]],
            alpha: glint_strength as f32,
            pad: 0.0,
        }
    }
}

/// The glint textures, their sampler and the per-frame uniform, shared by the entity and GUI
/// renderers' bind groups.
pub struct Glint {
    pub item: wgpu::TextureView,
    pub armor: wgpu::TextureView,
    /// The textures' sampler: their `.mcmeta` sets `blur` (linear) and they repeat
    /// (`ReloadableTexture.setSampler`).
    pub sampler: wgpu::Sampler,
    pub uniform: wgpu::Buffer,
    /// Vanilla's `Util.getMillis` clock origin (only the phase depends on it).
    start: std::time::Instant,
}

impl Glint {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, assets: &mut Assets) -> Arc<Glint> {
        let item = glint_texture(device, queue, assets, ENCHANTED_GLINT_ITEM);
        let armor = glint_texture(device, queue, assets, ENCHANTED_GLINT_ARMOR);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glint-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glint-uniform"),
            size: std::mem::size_of::<GlintParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glint = Glint {
            item,
            armor,
            sampler,
            uniform,
            start: std::time::Instant::now(),
        };
        glint.update(queue, DEFAULT_GLINT_SPEED, DEFAULT_GLINT_STRENGTH);
        Arc::new(glint)
    }

    /// Writes this frame's glint uniform (once per frame, before the entity and GUI passes).
    pub fn update(&self, queue: &wgpu::Queue, glint_speed: f64, glint_strength: f64) {
        let millis = self.start.elapsed().as_millis() as u64;
        let params = GlintParams::at(millis, glint_speed, glint_strength);
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
    }

    /// The layout entries the glint occupies in a bind group, from `first` on: the item and
    /// armour textures, the sampler and the uniform.
    pub fn layout_entries(first: u32) -> [wgpu::BindGroupLayoutEntry; 4] {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        [
            texture(first),
            texture(first + 1),
            wgpu::BindGroupLayoutEntry {
                binding: first + 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: first + 3,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]
    }

    /// The bind group entries matching [`Self::layout_entries`].
    pub fn entries(&self, first: u32) -> [wgpu::BindGroupEntry<'_>; 4] {
        [
            wgpu::BindGroupEntry {
                binding: first,
                resource: wgpu::BindingResource::TextureView(&self.item),
            },
            wgpu::BindGroupEntry {
                binding: first + 1,
                resource: wgpu::BindingResource::TextureView(&self.armor),
            },
            wgpu::BindGroupEntry {
                binding: first + 2,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
            wgpu::BindGroupEntry {
                binding: first + 3,
                resource: self.uniform.as_entire_binding(),
            },
        ]
    }
}

/// Uploads one glint texture as raw RGBA8 (vanilla's textures are `RGBA8_UNORM`, so the
/// glint adds its stored values), one mip like vanilla's. A texture that fails to load is
/// black, which adds nothing.
fn glint_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    assets: &mut Assets,
    path: &str,
) -> wgpu::TextureView {
    let img = assets
        .texture_any(&Identifier::minecraft(path))
        .unwrap_or_else(|e| {
            tracing::warn!("glint texture {path}: {e:#}");
            image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]))
        });
    let (w, h) = img.dimensions();
    let size = wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glint-texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        img.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * w),
            rows_per_image: Some(h),
        },
        size,
    );
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests;
