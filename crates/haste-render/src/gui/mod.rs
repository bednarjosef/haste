// GUI drawing: one `GuiFrame` per frame turned into a single vertex buffer of triangles over one
// lazily filled GUI atlas (sprites, textures, font pages, unifont glyphs, item textures), drawn
// in command order with one draw call per scissor change.
//
// Colour space: vanilla composites its GUI onto its main target (RGBA8_UNORM, an sRGB-unaware
// framebuffer) with the GUI_TEXTURED pipeline's BlendFunction.TRANSLUCENT on the *stored*
// sRGB-encoded bytes -- what it writes is what it blends. This renderer gives the GUI a
// non-sRGB view of its sRGB target, so the hardware keeps the stored bytes raw and blends them
// exactly the same way. Backends whose surface cannot be viewed as non-sRGB (GL) leave only the
// sRGB view: the GUI then emits linear colour, which keeps opaque pixels on vanilla's value but
// blends translucent ones in linear space instead. See `blend_gui_pixel` /
// `blend_gui_pixel_linear` and the tests below for the numbers.

mod argb;
mod atlas;
mod blend;
mod font;
mod graphics;
mod gui_entity_renderer;
mod gui_renderer;
mod item;
mod nametag;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use haste_assets::BitmapFont;
use haste_core::gui::GuiBlend;
use haste_core::Identifier;

pub(crate) use atlas::{hex_image_public, Shelf};
pub use blend::{blend_gui_alpha, blend_gui_pixel, blend_gui_pixel_linear, gui_writes_linear};
pub(crate) use item::{display_matrix, part_matrix};
pub use nametag::{project_nametag, ScreenNametag};

use argb::*;
use atlas::*;
use blend::*;
use gui_renderer::*;
use item::*;

/// Sprites the vanilla HUD draws every frame; loaded at startup.
pub const HUD_SPRITES: &[&str] = &[
    "hud/hotbar",
    "hud/hotbar_selection",
    "hud/crosshair",
    "hud/experience_bar_background",
    "hud/experience_bar_progress",
    "hud/heart/container",
    "hud/heart/full",
    "hud/heart/half",
    "hud/food_empty",
    "hud/food_full",
    "hud/food_half",
    "hud/armor_empty",
    "hud/armor_full",
    "hud/armor_half",
    "hud/air",
    "hud/air_empty",
];

/// Largest side of the square GUI atlas texture.
pub const GUI_ATLAS_SIZE: u32 = 4096;

/// The GUI atlas's first side: it doubles (keeping what it holds) when a frame's textures
/// do not fit, up to [`GUI_ATLAS_SIZE`]. The in-game HUD fits the first size, so the
/// 64 MiB of a full-size atlas are spent only where screens need them.
pub const GUI_ATLAS_START: u32 = 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GuiVertex {
    pub pos: [f32; 2],
    /// Atlas texel coordinates (wrapped into `rect` by the shader).
    pub uv: [f32; 2],
    /// RGBA, sRGB-encoded like vanilla's vertex colours.
    pub color: [u8; 4],
    /// Atlas rectangle x, y, w, h; w == 0 means an untextured (solid colour) vertex.
    pub rect: [u16; 4],
}

impl GuiVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4, 3 => Uint16x4
    ];
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GuiVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GuiUniforms {
    size: [f32; 2],
    /// x: 1 when the GUI writes linear colour (an sRGB-encoded target, no non-sRGB view).
    flags: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Batch {
    /// Framebuffer scissor x, y, w, h (None = whole target).
    scissor: Option<[u32; 4]>,
    /// The blend mode these vertices are drawn with (one pipeline per mode).
    blend: GuiBlend,
    start: u32,
    end: u32,
}

/// Counters for the last built frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuiStats {
    pub commands: u32,
    pub vertices: u32,
    pub draw_calls: u32,
    /// Commands not drawn yet (`GuiCommand::Entity`).
    pub unsupported: u32,
    /// Atlas resets this frame (the atlas filled up and was repacked).
    pub atlas_resets: u32,
}

pub struct GuiRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    /// One pipeline per blend mode, indexed like [`GuiBlend`] discriminants (normal first).
    pipelines: [wgpu::RenderPipeline; 4],
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    atlas: wgpu::Texture,
    /// The atlas texture's current side.
    atlas_size: u32,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,

    /// hashbrown so its key map can be probed through borrowed keys (`TexKeyRef`),
    /// without allocating an owned `Identifier` per command.
    keys: hashbrown::HashMap<TexKey, u32>,
    entries: Vec<TexEntry>,
    shelf: Shelf,
    atlas_full: bool,

    font: Arc<BitmapFont>,
    item_names: Vec<Identifier>,
    items: std::collections::HashMap<u32, Option<Arc<PreparedItem>>>,

    vertices: Vec<GuiVertex>,
    batches: Vec<Batch>,
    entity_scratch: Vec<crate::entity::EntityVertex>,
    translate: Vec<[f32; 2]>,
    scissors: Vec<[f32; 4]>,
    rng: u32,
    /// The target is sRGB (no non-sRGB view available): the shader outputs linear colour.
    linear_output: bool,
    /// The game tick animated GUI textures (fire, nether portal) show, so their current
    /// frame is re-uploaded; set once per tick by the client.
    animation_tick: u64,
    pub stats: GuiStats,
}
