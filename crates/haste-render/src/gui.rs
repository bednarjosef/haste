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
//
// The GUI item lighting (`items_3d_lights`, `items_flat_lights`, `lambert_shade`,
// `transform_gui_normal`) is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/world/block/model.rs, reused under GPL-3.0; see THIRD_PARTY.md.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat3, Mat4, Quat, Vec3};
use haste_assets::{
    for_each_formatted, frame_at, Animation, Assets, BitmapFont, GlyphKind, ItemGeometry, ItemModel,
    SpriteMeta, TextFormat,
};
use haste_core::gui::{GuiBlend, GuiCommand, GuiFrame, TextStyle};
use haste_core::Identifier;

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

/// Side of the square GUI atlas texture.
pub const GUI_ATLAS_SIZE: u32 = 4096;

/// Whether the GUI's shader has to emit linear colour for `format`: only when the target itself is
/// sRGB-encoded (no non-sRGB view on this backend), because then the hardware encodes whatever
/// the GUI writes. On a non-sRGB view nothing converts and the GUI's colours are written raw,
/// which is what vanilla does (its main target and GUI atlas are both RGBA8_UNORM).
pub fn gui_writes_linear(format: wgpu::TextureFormat) -> bool {
    format.is_srgb()
}

/// The sRGB transfer function (WGSL `to_linear`), byte in, linear in 0..1.
fn srgb_to_linear(byte: u8) -> f32 {
    let c = byte as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse of [`srgb_to_linear`], linear in 0..1 to a byte.
fn linear_to_srgb(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let c = if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Vanilla's GUI blend of the sprite pixel `src` over the background pixel `dst`, in 0..255
/// bytes: the GUI_TEXTURED pipeline's BlendFunction.TRANSLUCENT (colour SRC_ALPHA,
/// ONE_MINUS_SRC_ALPHA; alpha ONE, ONE_MINUS_SRC_ALPHA) applied to the *stored* values, which is
/// what the GUI pipelines do on the non-sRGB view of the target.
pub fn blend_gui_pixel(src: [u8; 4], dst: [u8; 3]) -> [u8; 4] {
    let a = src[3] as u32;
    let ia = 255 - a;
    let rgb: [u8; 3] =
        std::array::from_fn(|i| ((a * src[i] as u32 + ia * dst[i] as u32 + 127) / 255) as u8);
    [rgb[0], rgb[1], rgb[2], 255]
}

/// Vanilla's GUI alpha blend (alpha equation ONE, ONE_MINUS_SRC_ALPHA) of the sprite alpha `src`
/// over the background alpha `dst`, both 0..255. Colour-space independent: alpha is not encoded.
pub fn blend_gui_alpha(src: u8, dst: u8) -> u8 {
    let a = src as u32;
    ((255 * a + (255 - a) * dst as u32 + 127) / 255) as u8
}

/// What the same blend produces when the GUI's target is sRGB-encoded (the GL fallback): the
/// hardware decodes the destination, blends in linear space and encodes the result. Opaque pixels
/// are unchanged (encode(decode(x)) == x), translucent ones drift towards the destination's own
/// encoding; multiplicative and inverting pipelines are worse still, so `fs_main_multiply` and
/// `fs_main_invert` do not linearize.
pub fn blend_gui_pixel_linear(src: [u8; 4], dst: [u8; 3]) -> [u8; 4] {
    let a = src[3] as f32 / 255.0;
    let ia = 1.0 - a;
    let mix = |i: usize| linear_to_srgb(a * srgb_to_linear(src[i]) + ia * srgb_to_linear(dst[i]));
    [mix(0), mix(1), mix(2), 255]
}

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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TexKey {
    /// `textures/gui/sprites/<path>.png`.
    Sprite(Identifier),
    /// A `GuiCommand::Texture` texture (full `textures/...png` path or short id).
    Texture(Identifier),
    /// A texture an item model samples (`minecraft:block/stone`, `minecraft:item/apple`).
    ItemTexture(String),
    FontPage(u16),
    Hex(u32),
    Missing,
}

struct TexEntry {
    image: Arc<image::RgbaImage>,
    meta: SpriteMeta,
    /// Atlas rectangle when resident.
    rect: Option<[u32; 4]>,
    /// The `.mcmeta` animation of an animated texture (fire, nether portal); the atlas holds
    /// one frame and the current frame is re-uploaded each game tick. `None` when static.
    animation: Option<Animation>,
    /// The animation frame currently uploaded to `rect` (re-uploaded when it changes).
    frame: u32,
}

/// A slot item ready to draw: quads in GUI pixels relative to the slot's top-left corner,
/// back to front.
struct PreparedItem {
    quads: Vec<PreparedQuad>,
}

struct PreparedQuad {
    pos: [[f32; 2]; 4],
    /// Face-local 0..1 texture coordinates.
    uv: [[f32; 2]; 4],
    tex: TexKey,
    color: [u8; 4],
}

/// A name tag in GUI space: anchor (top centre of the text) and GUI pixels per text pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenNametag {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub text: String,
}

/// Projects a world name tag to GUI space (vanilla draws tags at 0.025 blocks per text pixel).
/// None when it is behind the camera or would be smaller than a pixel.
pub fn project_nametag(
    view_proj: &Mat4,
    camera: &crate::Camera,
    tag: &crate::entity::Nametag,
    target: (u32, u32),
    gui_scale: f32,
) -> Option<ScreenNametag> {
    // Relative to the camera in f64 first, so far coordinates keep precision.
    let p = view_proj.mul_vec4(glam::Vec4::new(
        tag.anchor.x as f32,
        tag.anchor.y as f32,
        tag.anchor.z as f32,
        1.0,
    ));
    if p.w <= 0.05 {
        return None;
    }
    let ndc = p.truncate() / p.w;
    if !(-1.5..=1.5).contains(&ndc.x) || !(-1.5..=1.5).contains(&ndc.y) {
        return None;
    }
    let (w, h) = (target.0 as f32, target.1 as f32);
    let fx = (ndc.x + 1.0) * 0.5 * w / gui_scale;
    let fy = (1.0 - ndc.y) * 0.5 * h / gui_scale;
    let focal = h / (2.0 * (camera.fov_deg.to_radians() * 0.5).tan());
    let scale = 0.025 * focal / p.w / gui_scale;
    if scale * 8.0 < 1.0 / gui_scale {
        return None;
    }
    Some(ScreenNametag {
        x: fx,
        y: fy,
        scale,
        text: tag.text.clone(),
    })
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
    uniforms: wgpu::Buffer,
    atlas: wgpu::Texture,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,

    keys: HashMap<TexKey, u32>,
    entries: Vec<TexEntry>,
    shelf: Shelf,
    atlas_full: bool,

    font: Arc<BitmapFont>,
    item_names: Vec<Identifier>,
    items: HashMap<u32, Option<Arc<PreparedItem>>>,

    vertices: Vec<GuiVertex>,
    batches: Vec<Batch>,
    entity_scratch: Vec<crate::entity::EntityVertex>,
    rng: u32,
    /// The target is sRGB (no non-sRGB view available): the shader outputs linear colour.
    linear_output: bool,
    /// The game tick animated GUI textures (fire, nether portal) show, so their current
    /// frame is re-uploaded; set once per tick by the client.
    animation_tick: u64,
    pub stats: GuiStats,
}

/// Shelf packer over a square atlas.
#[derive(Default)]
pub(crate) struct Shelf {
    x: u32,
    y: u32,
    row_h: u32,
}

impl Shelf {
    fn alloc(&mut self, w: u32, h: u32) -> Option<[u32; 2]> {
        self.alloc_in(w, h, GUI_ATLAS_SIZE)
    }

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

/// ARGB (vanilla) to RGBA bytes.
fn rgba(argb: u32) -> [u8; 4] {
    [
        (argb >> 16) as u8,
        (argb >> 8) as u8,
        argb as u8,
        (argb >> 24) as u8,
    ]
}

/// Multiply two RGBA colours.
fn mul(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    std::array::from_fn(|i| ((a[i] as u32 * b[i] as u32 + 127) / 255) as u8)
}

/// Vanilla's text shadow colour: RGB scaled by 0.25, alpha kept.
fn shadow_color(argb: u32) -> u32 {
    let c = |s: u32| (((argb >> s) & 0xFF) / 4) << s;
    (argb & 0xFF00_0000) | c(16) | c(8) | c(0)
}

/// Sprite/texture tints: 0 (unset) draws untinted.
fn tint_or_white(tint: u32) -> u32 {
    if tint == 0 {
        0xFFFF_FFFF
    } else {
        tint
    }
}

/// The wgpu blend state of a GUI blend mode. Normal is vanilla's `gui_textured` (straight alpha),
/// Multiply the vignette's (dst * (1 - src colour)), Invert the crosshair's (the source inverts the
/// colour behind it) and Additive the text selection highlight's (src ONE / dst ONE, both equations:
/// `BlendFunction.ADDITIVE`).
fn blend_state(mode: GuiBlend) -> wgpu::BlendState {
    let equation =
        |src_factor: wgpu::BlendFactor, dst_factor: wgpu::BlendFactor| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
    match mode {
        GuiBlend::Normal => wgpu::BlendState {
            color: equation(
                wgpu::BlendFactor::SrcAlpha,
                wgpu::BlendFactor::OneMinusSrcAlpha,
            ),
            alpha: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
        },
        GuiBlend::Multiply => wgpu::BlendState {
            color: equation(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrc),
            alpha: equation(wgpu::BlendFactor::Zero, wgpu::BlendFactor::One),
        },
        GuiBlend::Invert => wgpu::BlendState {
            color: equation(
                wgpu::BlendFactor::OneMinusDst,
                wgpu::BlendFactor::OneMinusSrc,
            ),
            // vanilla BlendFunction.INVERT: alpha ONE, ZERO
            alpha: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::Zero),
        },
        GuiBlend::Additive => wgpu::BlendState {
            color: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::One),
            alpha: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::One),
        },
    }
}

impl GuiRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        font: Arc<BitmapFont>,
    ) -> GuiRenderer {
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gui-atlas"),
            size: wgpu::Extent3d {
                width: GUI_ATLAS_SIZE,
                height: GUI_ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui-uniforms"),
            size: std::mem::size_of::<GuiUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gui-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gui-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gui.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gui-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // One pipeline per blend mode: vanilla's GUI pipelines (RenderPipelines.GUI_TEXTURED and
        // friends blend normally; VIGNETTE multiplies; CROSSHAIR inverts the destination;
        // GUI_TEXT_HIGHLIGHT adds to it). Each has its own fragment entry point so only the
        // normal blend converts to linear on an sRGB-encoded target (`gui_color` in gui.wgsl).
        let make_pipeline = |label, blend: Option<wgpu::BlendState>, fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(GuiVertex::LAYOUT)],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The vignette texture is an opaque grayscale radial gradient (black centre, bright
        // edges), so its blend multiplies the destination by it: unchanged in the centre,
        // darkened towards the edges.
        let pipelines = [
            make_pipeline("gui-normal", Some(blend_state(GuiBlend::Normal)), "fs_main"),
            make_pipeline(
                "gui-multiply",
                Some(blend_state(GuiBlend::Multiply)),
                "fs_main_multiply",
            ),
            make_pipeline(
                "gui-invert",
                Some(blend_state(GuiBlend::Invert)),
                "fs_main_invert",
            ),
            make_pipeline(
                "gui-additive",
                Some(blend_state(GuiBlend::Additive)),
                "fs_main_additive",
            ),
        ];
        let vertex_capacity = 16 * 1024;
        let vertex_buffer = Self::make_vertex_buffer(&device, vertex_capacity);
        // An sRGB target (no non-sRGB view on this backend) forces the shader to emit linear
        // colour; warn with the concrete deviation that causes for translucent GUI pixels.
        let linear_output = gui_writes_linear(target_format);
        if linear_output {
            let src = [128u8, 128, 128, 140];
            let dst = [200u8, 80, 40];
            let vanilla = blend_gui_pixel(src, dst);
            let fallback = blend_gui_pixel_linear(src, dst);
            tracing::warn!(
                "no non-sRGB view of the GUI target {:?}: translucent GUI pixels blend in linear space \
                 (a 55% grey over rgb({},{},{}) lands on {:?} instead of vanilla's {:?})",
                target_format,
                dst[0],
                dst[1],
                dst[2],
                fallback,
                vanilla
            );
        }
        let mut gui = GuiRenderer {
            device,
            queue,
            pipelines,
            bind_group,
            uniforms,
            atlas,
            vertex_buffer,
            vertex_capacity,
            keys: HashMap::new(),
            entries: Vec::new(),
            shelf: Shelf::default(),
            atlas_full: false,
            font,
            item_names: Vec::new(),
            items: HashMap::new(),
            vertices: Vec::new(),
            batches: Vec::new(),
            entity_scratch: Vec::new(),
            rng: 0x9E37_79B9,
            linear_output,
            animation_tick: 0,
            stats: GuiStats::default(),
        };
        // The missing texture is entry 0 so lookups can always fall back to it.
        gui.insert_entry(
            TexKey::Missing,
            Arc::new(missing_image()),
            SpriteMeta::Stretch,
            None,
        );
        gui
    }

    /// The game tick the animated GUI textures (fire, nether portal) show. The next `prepare`
    /// re-uploads every changed frame.
    pub fn set_animation_tick(&mut self, tick: u64) {
        self.animation_tick = tick;
    }

    fn make_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gui-vertices"),
            size: (capacity * std::mem::size_of::<GuiVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn font(&self) -> &Arc<BitmapFont> {
        &self.font
    }

    /// Item registry names by protocol id (`minecraft:item`), for `GuiCommand::Item`.
    pub fn set_item_names(&mut self, names: Vec<Identifier>) {
        self.item_names = names;
        self.items.clear();
    }

    // ---- textures ----

    fn insert_entry(
        &mut self,
        key: TexKey,
        image: Arc<image::RgbaImage>,
        meta: SpriteMeta,
        animation: Option<Animation>,
    ) -> u32 {
        let index = self.entries.len() as u32;
        self.entries.push(TexEntry {
            image,
            meta,
            rect: None,
            animation,
            frame: 0,
        });
        self.keys.insert(key, index);
        index
    }

    /// Entry index for a key, loading the image on first use (missing texture on failure).
    fn entry(&mut self, key: &TexKey, assets: &mut Assets) -> u32 {
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
    fn resident(&mut self, index: u32) -> [u32; 4] {
        let e = &self.entries[index as usize];
        if let Some(r) = e.rect {
            return r;
        }
        let (w, h) = e.image.dimensions();
        let Some([x, y]) = self.shelf.alloc(w.max(1), h.max(1)) else {
            self.atlas_full = true;
            return self.entries[0].rect.unwrap_or([0, 0, 0, 0]);
        };
        if w > 0 && h > 0 {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                e.image.as_raw(),
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
        rect
    }

    fn lookup(&mut self, key: &TexKey, assets: &mut Assets) -> ([u32; 4], u32) {
        let i = self.entry(key, assets);
        (self.resident(i), i)
    }

    /// Re-uploads the current frame of every resident animated texture (fire, nether portal)
    /// when the tick's frame changed, like vanilla's sprite animation.
    fn animate(&mut self) {
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

    fn reset_atlas(&mut self) {
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

    /// Turns the frame into vertices and batches and uploads them. `target` is the framebuffer
    /// size in pixels.
    pub fn prepare(
        &mut self,
        frame: &GuiFrame,
        entities: &[haste_core::model::EntityInstance],
        nametags: &[ScreenNametag],
        target: (u32, u32),
        assets: &mut Assets,
    ) {
        self.stats = GuiStats::default();
        if self.entries[0].rect.is_none() {
            self.resident(0);
        }
        for attempt in 0..2 {
            self.build(frame, entities, nametags, target, assets);
            if !self.atlas_full {
                break;
            }
            // The atlas filled up mid-frame: repack with only what this frame needs.
            self.stats.atlas_resets += 1;
            self.reset_atlas();
            if attempt == 1 {
                tracing::warn!("GUI atlas too small for one frame's textures");
            }
        }
        // Animated textures (fire, nether portal): bring the resident frames up to date.
        self.animate();
        let scale = frame.scale.max(1) as f32;
        let uniforms = GuiUniforms {
            size: [target.0 as f32 / scale, target.1 as f32 / scale],
            flags: [if self.linear_output { 1.0 } else { 0.0 }, 0.0],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        if self.vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = Self::make_vertex_buffer(&self.device, self.vertex_capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
        self.stats.commands = frame.commands.len() as u32;
        self.stats.vertices = self.vertices.len() as u32;
        self.stats.draw_calls = self.batches.len() as u32;
    }

    /// Records the prepared frame into a render pass on the target.
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>, target: (u32, u32)) {
        if self.batches.is_empty() {
            return;
        }
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        for b in &self.batches {
            pass.set_pipeline(&self.pipelines[b.blend as usize]);
            match b.scissor {
                Some([x, y, w, h]) => pass.set_scissor_rect(x, y, w, h),
                None => pass.set_scissor_rect(0, 0, target.0, target.1),
            }
            pass.draw(b.start..b.end, 0..1);
        }
    }

    fn build(
        &mut self,
        frame: &GuiFrame,
        entities: &[haste_core::model::EntityInstance],
        nametags: &[ScreenNametag],
        target: (u32, u32),
        assets: &mut Assets,
    ) {
        self.vertices.clear();
        self.batches.clear();
        // Name tags first, under the HUD: a 25% black background and white text, drawn at the
        // origin and then moved and scaled into place.
        for tag in nametags {
            let start = self.vertices.len();
            let w = self.font.width_f32(&tag.text);
            let x = -(w / 2.0).floor();
            self.solid(
                [0.0; 2],
                [x - 1.0, -1.0, x + w, 8.0],
                [0, 0, 0, 64],
                [0, 0, 0, 64],
            );
            self.text(
                [0.0; 2],
                x,
                0.0,
                &tag.text,
                0xFFFF_FFFF,
                false,
                TextStyle::default(),
                assets,
            );
            for v in &mut self.vertices[start..] {
                v.pos = [tag.x + v.pos[0] * tag.scale, tag.y + v.pos[1] * tag.scale];
            }
        }
        let scale = frame.scale.max(1) as f32;
        let mut translate: Vec<[f32; 2]> = Vec::new();
        let mut offset = [0.0f32; 2];
        let mut scissors: Vec<[f32; 4]> = Vec::new();
        let mut scissor: Option<[u32; 4]> = None;
        let mut blend = GuiBlend::Normal;
        let mut batch_start = 0u32;
        for cmd in &frame.commands {
            match cmd {
                GuiCommand::Fill {
                    x1,
                    y1,
                    x2,
                    y2,
                    color,
                } => {
                    let (x1, x2) = ((*x1).min(*x2), (*x1).max(*x2));
                    let (y1, y2) = ((*y1).min(*y2), (*y1).max(*y2));
                    self.solid(
                        offset,
                        [x1 as f32, y1 as f32, x2 as f32, y2 as f32],
                        rgba(*color),
                        rgba(*color),
                    );
                }
                GuiCommand::FillGradient {
                    x1,
                    y1,
                    x2,
                    y2,
                    top,
                    bottom,
                } => {
                    let (x1, x2) = ((*x1).min(*x2), (*x1).max(*x2));
                    let (y1, y2) = ((*y1).min(*y2), (*y1).max(*y2));
                    self.solid(
                        offset,
                        [x1 as f32, y1 as f32, x2 as f32, y2 as f32],
                        rgba(*top),
                        rgba(*bottom),
                    );
                }
                GuiCommand::Text {
                    x,
                    y,
                    text,
                    color,
                    shadow,
                } => self.text(
                    offset,
                    *x as f32,
                    *y as f32,
                    text,
                    *color,
                    *shadow,
                    TextStyle::default(),
                    assets,
                ),
                GuiCommand::StyledText {
                    x,
                    y,
                    text,
                    color,
                    shadow,
                    style,
                } => self.text(
                    offset, *x as f32, *y as f32, text, *color, *shadow, *style, assets,
                ),
                GuiCommand::Texture {
                    texture,
                    x,
                    y,
                    w,
                    h,
                    u,
                    v,
                    uw,
                    vh,
                    tex_w,
                    tex_h,
                    tint,
                    blend: cmd_blend,
                } => {
                    let cur = scissor;
                    self.switch(&mut scissor, &mut blend, cur, *cmd_blend, &mut batch_start);
                    let (rect, index) = self.lookup(&TexKey::Texture(texture.clone()), assets);
                    let (iw, ih) = self.entries[index as usize].image.dimensions();
                    let kx = iw as f32 / (*tex_w).max(1) as f32;
                    let ky = ih as f32 / (*tex_h).max(1) as f32;
                    let u0 = rect[0] as f32 + u * kx;
                    let v0 = rect[1] as f32 + v * ky;
                    self.textured(
                        offset,
                        [*x as f32, *y as f32, (*x + *w) as f32, (*y + *h) as f32],
                        [u0, v0, u0 + uw * kx, v0 + vh * ky],
                        rect,
                        rgba(tint_or_white(*tint)),
                    );
                }
                GuiCommand::Sprite {
                    sprite,
                    x,
                    y,
                    w,
                    h,
                    tint,
                    blend: cmd_blend,
                } => {
                    let cur = scissor;
                    self.switch(&mut scissor, &mut blend, cur, *cmd_blend, &mut batch_start);
                    self.sprite(offset, sprite, *x, *y, *w, *h, *tint, assets)
                }
                GuiCommand::SpriteRegion {
                    sprite,
                    sprite_w,
                    sprite_h,
                    u,
                    v,
                    x,
                    y,
                    w,
                    h,
                    tint,
                    blend: cmd_blend,
                } => {
                    let cur = scissor;
                    self.switch(&mut scissor, &mut blend, cur, *cmd_blend, &mut batch_start);
                    let (rect, _) = self.lookup(&TexKey::Sprite(sprite.clone()), assets);
                    let kx = rect[2] as f32 / (*sprite_w).max(1) as f32;
                    let ky = rect[3] as f32 / (*sprite_h).max(1) as f32;
                    let u0 = rect[0] as f32 + *u as f32 * kx;
                    let v0 = rect[1] as f32 + *v as f32 * ky;
                    self.textured(
                        offset,
                        [*x as f32, *y as f32, (*x + *w) as f32, (*y + *h) as f32],
                        [u0, v0, u0 + *w as f32 * kx, v0 + *h as f32 * ky],
                        rect,
                        rgba(tint_or_white(*tint)),
                    );
                }
                GuiCommand::Item { item, x, y, .. } => {
                    self.item(offset, item.0, *x as f32, *y as f32, assets)
                }
                GuiCommand::Entity {
                    entity,
                    x1,
                    y1,
                    x2,
                    y2,
                    scale: size,
                    mouse_x,
                    mouse_y,
                } => match entities.iter().find(|e| e.id == *entity) {
                    None => self.stats.unsupported += 1,
                    Some(e) => {
                        // Clipped to its box like vanilla's scissored entity widget.
                        let mut r = [
                            *x1 as f32 + offset[0],
                            *y1 as f32 + offset[1],
                            *x2 as f32 + offset[0],
                            *y2 as f32 + offset[1],
                        ];
                        if let Some(p) = scissors.last() {
                            r = [
                                r[0].max(p[0]),
                                r[1].max(p[1]),
                                r[2].min(p[2]),
                                r[3].min(p[3]),
                            ];
                        }
                        let outer = scissor;
                        let cur_blend = blend;
                        self.switch(
                            &mut scissor,
                            &mut blend,
                            Some(to_framebuffer(r, scale, target)),
                            cur_blend,
                            &mut batch_start,
                        );
                        self.entity_in_box(
                            offset,
                            e,
                            [*x1 as f32, *y1 as f32, *x2 as f32, *y2 as f32],
                            *size,
                            [*mouse_x, *mouse_y],
                            assets,
                        );
                        self.switch(&mut scissor, &mut blend, outer, cur_blend, &mut batch_start);
                    }
                },
                GuiCommand::ScissorPush { x1, y1, x2, y2 } => {
                    let mut r = [
                        *x1 as f32 + offset[0],
                        *y1 as f32 + offset[1],
                        *x2 as f32 + offset[0],
                        *y2 as f32 + offset[1],
                    ];
                    if let Some(p) = scissors.last() {
                        r = [
                            r[0].max(p[0]),
                            r[1].max(p[1]),
                            r[2].min(p[2]),
                            r[3].min(p[3]),
                        ];
                    }
                    scissors.push(r);
                    let next = Some(to_framebuffer(r, scale, target));
                    let cur = blend;
                    self.switch(&mut scissor, &mut blend, next, cur, &mut batch_start);
                }
                GuiCommand::ScissorPop => {
                    scissors.pop();
                    let next = scissors.last().map(|r| to_framebuffer(*r, scale, target));
                    let cur = blend;
                    self.switch(&mut scissor, &mut blend, next, cur, &mut batch_start);
                }
                GuiCommand::TranslatePush { dx, dy, .. } => {
                    translate.push(offset);
                    offset = [offset[0] + dx, offset[1] + dy];
                }
                GuiCommand::TranslatePop => {
                    offset = translate.pop().unwrap_or([0.0; 2]);
                }
            }
        }
        let end = self.vertices.len() as u32;
        if end > batch_start {
            self.batches.push(Batch {
                scissor,
                blend,
                start: batch_start,
                end,
            });
        }
    }

    /// Ends the current batch when the scissor or the blend mode is about to change, so each
    /// batch is one draw call with one pipeline (batching by blend, never per sprite).
    #[allow(clippy::too_many_arguments)]
    fn switch(
        &mut self,
        scissor: &mut Option<[u32; 4]>,
        blend: &mut GuiBlend,
        next_scissor: Option<[u32; 4]>,
        next_blend: GuiBlend,
        batch_start: &mut u32,
    ) {
        if *scissor == next_scissor && *blend == next_blend {
            return;
        }
        let end = self.vertices.len() as u32;
        if end > *batch_start {
            self.batches.push(Batch {
                scissor: *scissor,
                blend: *blend,
                start: *batch_start,
                end,
            });
        }
        *batch_start = end;
        *scissor = next_scissor;
        *blend = next_blend;
    }

    // ---- primitives ----

    fn quad(&mut self, pos: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: [[u8; 4]; 4], rect: [u32; 4]) {
        let rect = rect.map(|v| v as u16);
        for i in [0usize, 1, 2, 0, 2, 3] {
            self.vertices.push(GuiVertex {
                pos: pos[i],
                uv: uv[i],
                color: color[i],
                rect,
            });
        }
    }

    /// Axis-aligned solid rectangle (x1, y1, x2, y2) with top and bottom colours.
    fn solid(&mut self, o: [f32; 2], r: [f32; 4], top: [u8; 4], bottom: [u8; 4]) {
        if r[0] >= r[2] || r[1] >= r[3] || (top[3] == 0 && bottom[3] == 0) {
            return;
        }
        let (x1, y1, x2, y2) = (r[0] + o[0], r[1] + o[1], r[2] + o[0], r[3] + o[1]);
        self.quad(
            [[x1, y1], [x1, y2], [x2, y2], [x2, y1]],
            [[0.0; 2]; 4],
            [top, bottom, bottom, top],
            [0; 4],
        );
    }

    /// Axis-aligned textured rectangle: `r` in GUI pixels, `uv` in atlas texels.
    fn textured(&mut self, o: [f32; 2], r: [f32; 4], uv: [f32; 4], rect: [u32; 4], color: [u8; 4]) {
        if r[0] >= r[2] || r[1] >= r[3] || color[3] == 0 || rect[2] == 0 || rect[3] == 0 {
            return;
        }
        let (x1, y1, x2, y2) = (r[0] + o[0], r[1] + o[1], r[2] + o[0], r[3] + o[1]);
        self.quad(
            [[x1, y1], [x1, y2], [x2, y2], [x2, y1]],
            [
                [uv[0], uv[1]],
                [uv[0], uv[3]],
                [uv[2], uv[3]],
                [uv[2], uv[1]],
            ],
            [color; 4],
            rect,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn sprite(
        &mut self,
        o: [f32; 2],
        sprite: &Identifier,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        tint: u32,
        assets: &mut Assets,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (rect, index) = self.lookup(&TexKey::Sprite(sprite.clone()), assets);
        let meta = self.entries[index as usize].meta.clone();
        let color = rgba(tint_or_white(tint));
        let (ax, ay, aw, ah) = (
            rect[0] as f32,
            rect[1] as f32,
            rect[2] as f32,
            rect[3] as f32,
        );
        let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
        match meta {
            SpriteMeta::Stretch => self.textured(
                o,
                [fx, fy, fx + fw, fy + fh],
                [ax, ay, ax + aw, ay + ah],
                rect,
                color,
            ),
            SpriteMeta::Tile { width, height } => {
                let kx = aw / width.max(1) as f32;
                let ky = ah / height.max(1) as f32;
                self.textured(
                    o,
                    [fx, fy, fx + fw, fy + fh],
                    [ax, ay, ax + fw * kx, ay + fh * ky],
                    rect,
                    color,
                )
            }
            SpriteMeta::NineSlice {
                width,
                height,
                border,
                stretch_inner,
            } => {
                let (sw, sh) = (width.max(1) as i32, height.max(1) as i32);
                if w == sw && h == sh {
                    self.textured(
                        o,
                        [fx, fy, fx + fw, fy + fh],
                        [ax, ay, ax + aw, ay + ah],
                        rect,
                        color,
                    );
                    return;
                }
                let kx = aw / sw as f32;
                let ky = ah / sh as f32;
                let [bl, bt, br, bb] = border.map(|b| b as i32);
                let (l, r) = (bl.min(w / 2), br.min(w / 2));
                let (t, b) = (bt.min(h / 2), bb.min(h / 2));
                // Destination and source (sprite units) spans of the three columns/rows.
                let cols = [
                    (0, l, 0, l),
                    (l, w - r, bl, sw - br),
                    (w - r, w, sw - r, sw),
                ];
                let rows = [
                    (0, t, 0, t),
                    (t, h - b, bt, sh - bb),
                    (h - b, h, sh - b, sh),
                ];
                for (ri, &(dy0, dy1, sy0, sy1)) in rows.iter().enumerate() {
                    for (ci, &(dx0, dx1, sx0, sx1)) in cols.iter().enumerate() {
                        if dx1 <= dx0 || dy1 <= dy0 || sx1 <= sx0 || sy1 <= sy0 {
                            continue;
                        }
                        let src = [
                            (ax + sx0 as f32 * kx) as u32,
                            (ay + sy0 as f32 * ky) as u32,
                            ((sx1 - sx0) as f32 * kx) as u32,
                            ((sy1 - sy0) as f32 * ky) as u32,
                        ];
                        let stretch = stretch_inner && ri == 1 && ci == 1;
                        let (span_u, span_v) = if stretch {
                            (src[2] as f32, src[3] as f32)
                        } else {
                            ((dx1 - dx0) as f32 * kx, (dy1 - dy0) as f32 * ky)
                        };
                        let (u0, v0) = (src[0] as f32, src[1] as f32);
                        self.textured(
                            o,
                            [
                                fx + dx0 as f32,
                                fy + dy0 as f32,
                                fx + dx1 as f32,
                                fy + dy1 as f32,
                            ],
                            [u0, v0, u0 + span_u, v0 + span_v],
                            src,
                            color,
                        );
                    }
                }
            }
        }
    }

    // ---- text ----

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        o: [f32; 2],
        x: f32,
        y: f32,
        text: &str,
        color: u32,
        shadow: bool,
        style: TextStyle,
        assets: &mut Assets,
    ) {
        // Vanilla (since 1.21.6) skips text whose colour has zero alpha.
        if color >> 24 == 0 || text.is_empty() {
            return;
        }
        let base = TextFormat {
            color: None,
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
            strikethrough: style.strikethrough,
            obfuscated: style.obfuscated,
        };
        if shadow {
            self.text_pass(o, x, y, text, color, base, true, assets);
        }
        self.text_pass(o, x, y, text, color, base, false, assets);
    }

    #[allow(clippy::too_many_arguments)]
    fn text_pass(
        &mut self,
        o: [f32; 2],
        x: f32,
        y: f32,
        text: &str,
        color: u32,
        base: TextFormat,
        shadow: bool,
        assets: &mut Assets,
    ) {
        let font = self.font.clone();
        let mut cursor = x;
        let alpha = color & 0xFF00_0000;
        let mut rng = self.rng;
        for_each_formatted(text, base, |ch, fmt| {
            let argb = fmt.color.map(|rgb| alpha | rgb).unwrap_or(color);
            let argb = if shadow { shadow_color(argb) } else { argb };
            let col = rgba(argb);
            let mut glyph = font.glyph(ch);
            if fmt.obfuscated && ch != ' ' {
                let pool = font.chars_with_advance(glyph.advance);
                if !pool.is_empty() {
                    rng ^= rng << 13;
                    rng ^= rng >> 17;
                    rng ^= rng << 5;
                    glyph = font.glyph(pool[rng as usize % pool.len()]);
                }
            }
            let sh = if shadow { glyph.shadow_offset } else { 0.0 };
            let gx = cursor + sh;
            let gy = y + sh;
            let passes: &[f32] = if fmt.bold {
                &[0.0, glyph.bold_offset]
            } else {
                &[0.0]
            };
            for &bold in passes {
                self.glyph(o, glyph.kind, gx + bold, gy, fmt.italic, col, assets);
            }
            let advance = glyph.advance + if fmt.bold { glyph.bold_offset } else { 0.0 };
            if fmt.strikethrough {
                self.solid(o, [gx - 1.0, gy + 3.5, gx + advance, gy + 4.5], col, col);
            }
            if fmt.underline {
                self.solid(o, [gx - 1.0, gy + 8.0, gx + advance, gy + 9.0], col, col);
            }
            cursor += advance;
        });
        self.rng = rng;
    }

    #[allow(clippy::too_many_arguments)]
    fn glyph(
        &mut self,
        o: [f32; 2],
        kind: GlyphKind,
        x: f32,
        y: f32,
        italic: bool,
        color: [u8; 4],
        assets: &mut Assets,
    ) {
        // Italic shears by 1 - 0.25 * (vertex y relative to the line top).
        let skew = |vy: f32| if italic { 1.0 - 0.25 * (vy - y) } else { 0.0 };
        let (rect, src, size, top) = match kind {
            GlyphKind::Empty => return,
            GlyphKind::Missing => {
                // A 5x8 outlined box.
                for r in [
                    [x, y, x + 5.0, y + 1.0],
                    [x, y + 7.0, x + 5.0, y + 8.0],
                    [x, y + 1.0, x + 1.0, y + 7.0],
                    [x + 4.0, y + 1.0, x + 5.0, y + 7.0],
                ] {
                    self.solid(o, r, color, color);
                }
                return;
            }
            GlyphKind::Bitmap {
                page,
                x: gx,
                y: gy,
                w,
                h,
                scale,
                top,
            } => {
                if w == 0 {
                    return;
                }
                let (rect, _) = self.lookup(&TexKey::FontPage(page), assets);
                (
                    rect,
                    [gx as f32, gy as f32, w as f32, h as f32],
                    [w as f32 * scale, h as f32 * scale],
                    top,
                )
            }
            GlyphKind::Hex { index } => {
                let Some(g) = self.font.hex_glyph(index) else {
                    return;
                };
                if g.trimmed == 0 {
                    return;
                }
                let (rect, _) = self.lookup(&TexKey::Hex(index), assets);
                (
                    rect,
                    [0.0, 0.0, rect[2] as f32, rect[3] as f32],
                    [rect[2] as f32 / 2.0, rect[3] as f32 / 2.0],
                    0.0,
                )
            }
        };
        let (x0, y0) = (x + o[0], y + top + o[1]);
        let (x1, y1) = (x0 + size[0], y0 + size[1]);
        let (st, sb) = (skew(y + top), skew(y + top + size[1]));
        let u0 = rect[0] as f32 + src[0];
        let v0 = rect[1] as f32 + src[1];
        let (u1, v1) = (u0 + src[2], v0 + src[3]);
        self.quad(
            [[x0 + st, y0], [x0 + sb, y1], [x1 + sb, y1], [x1 + st, y0]],
            [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
            [color; 4],
            rect,
        );
    }

    /// A living entity in a GUI box facing the viewer and turning toward the mouse (the
    /// inventory player). `size` is GUI pixels per block. Approximate: the body turns by
    /// atan(dx / 40) * 20 degrees and the head by twice that and pitches by atan(dy / 40) * 20,
    /// the shape of vanilla's inventory model; the feet stand near the box bottom.
    fn entity_in_box(
        &mut self,
        o: [f32; 2],
        e: &haste_core::model::EntityInstance,
        b: [f32; 4],
        size: f32,
        mouse: [f32; 2],
        assets: &mut Assets,
    ) {
        let (texture, model_scale) = crate::entity::texture_and_scale(e);
        let (rect, _) = self.lookup(&TexKey::Texture(texture), assets);
        let cx = (b[0] + b[2]) / 2.0;
        let height = crate::entity::model_top(&e.model.root, 0.0) * model_scale / 16.0;
        let feet = (b[1] + b[3]) / 2.0 + height * size / 2.0;
        let dx = cx - mouse[0];
        let dy = feet - height * size * 0.85 - mouse[1];
        let f = (dx / 40.0).atan();
        let g = (dy / 40.0).atan();
        let mut pose = e.pose.clone();
        pose.rotations.retain(|(n, _)| n != "head");
        pose.rotations.push((
            "head".into(),
            Vec3::new(-g * 20f32.to_radians(), f * 20f32.to_radians(), 0.0),
        ));
        // GUI space: x right, y down, z toward the viewer. The model's front (-z) turns to +z.
        let m = Mat4::from_translation(Vec3::new(cx + o[0], feet + o[1], 0.0))
            * Mat4::from_scale(Vec3::new(size, -size, size))
            * Mat4::from_rotation_y(std::f32::consts::PI + f * 20f32.to_radians())
            * Mat4::from_scale(Vec3::new(-model_scale, -model_scale, model_scale) / 16.0);
        let mut tris = std::mem::take(&mut self.entity_scratch);
        tris.clear();
        crate::entity::emit_part(&mut tris, &e.model, &e.model.root, &pose, m, rect, 1.0, 0.0);
        // Back faces off, then back to front.
        let mut order: Vec<(f32, usize)> = tris
            .chunks(3)
            .enumerate()
            .filter_map(|(i, t)| {
                let p = [
                    Vec3::from(t[0].pos),
                    Vec3::from(t[1].pos),
                    Vec3::from(t[2].pos),
                ];
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                (n.z > 0.0).then_some(((p[0].z + p[1].z + p[2].z) / 3.0, i))
            })
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, i) in order {
            for v in &tris[i * 3..i * 3 + 3] {
                let c = (v.color[0] * 255.0).round() as u8;
                self.vertices.push(GuiVertex {
                    pos: [v.pos[0], v.pos[1]],
                    uv: v.uv,
                    color: [c, c, c, 255],
                    rect: v.rect,
                });
            }
        }
        self.entity_scratch = tris;
    }

    // ---- items ----

    fn item(&mut self, o: [f32; 2], item: u32, x: f32, y: f32, assets: &mut Assets) {
        let prepared = match self.items.get(&item) {
            Some(p) => p.clone(),
            None => {
                let p = self
                    .item_names
                    .get(item as usize)
                    .cloned()
                    .and_then(|name| match assets.item_model(&name) {
                        Ok(model) => Some(Arc::new(prepare_item(&model))),
                        Err(e) => {
                            tracing::warn!("item model {name}: {e:#}");
                            None
                        }
                    });
                self.items.insert(item, p.clone());
                p
            }
        };
        let Some(prepared) = prepared else {
            // Unknown item: the missing texture, flat.
            let (rect, _) = self.lookup(&TexKey::Missing, assets);
            let r = rect.map(|v| v as f32);
            self.textured(
                o,
                [x, y, x + 16.0, y + 16.0],
                [r[0], r[1], r[0] + r[2], r[1] + r[3]],
                rect,
                [255; 4],
            );
            return;
        };
        for q in &prepared.quads {
            let (rect, _) = self.lookup(&q.tex, assets);
            let r = rect.map(|v| v as f32);
            let pos = q.pos.map(|p| [p[0] + x + o[0], p[1] + y + o[1]]);
            let uv = q.uv.map(|t| [r[0] + t[0] * r[2], r[1] + t[1] * r[3]]);
            self.quad(pos, uv, [q.color; 4], rect);
        }
    }
}

/// Scissor rectangle (GUI pixels, x1 y1 x2 y2) to framebuffer x, y, w, h, clamped.
fn to_framebuffer(r: [f32; 4], scale: f32, target: (u32, u32)) -> [u32; 4] {
    let x1 = (r[0] * scale).round().clamp(0.0, target.0 as f32) as u32;
    let y1 = (r[1] * scale).round().clamp(0.0, target.1 as f32) as u32;
    let x2 = (r[2] * scale).round().clamp(0.0, target.0 as f32) as u32;
    let y2 = (r[3] * scale).round().clamp(0.0, target.1 as f32) as u32;
    [x1, y1, x2.saturating_sub(x1), y2.saturating_sub(y1)]
}

/// Vanilla's missing texture: 16x16 magenta/black checker of 8x8 squares.
fn missing_image() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 16, |x, y| {
        if (x < 8) ^ (y < 8) {
            image::Rgba([248, 0, 248, 255])
        } else {
            image::Rgba([0, 0, 0, 255])
        }
    })
}

/// A unihex glyph's trimmed columns as a white-on-transparent image (16 rows).
fn hex_image(g: &haste_assets::HexGlyph) -> image::RgbaImage {
    let w = (g.trimmed as u32).max(1);
    image::RgbaImage::from_fn(w, 16, |x, y| {
        if g.pixel(g.left as u32 + x, y) {
            image::Rgba([255, 255, 255, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    })
}

// ---- GUI item geometry and lighting ----

/// Pomme `items_3d_lights`: the two GUI light directions for side-lit (3D) items, in the
/// space of `transform_gui_normal`.
fn items_3d_lights() -> (Vec3, Vec3) {
    let pose = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
        * Mat4::from_rotation_y(1.082_104_1)
        * Mat4::from_rotation_x(3.237_585_8)
        * Mat4::from_rotation_y(-std::f32::consts::PI / 8.0)
        * Mat4::from_rotation_x(2.356_194_5);
    let t = |x: f32, y: f32, z: f32| pose.transform_vector3(Vec3::new(x, y, z).normalize());
    (t(0.2, 1.0, -0.7), t(-0.2, 1.0, 0.7))
}

/// Pomme `items_flat_lights`: the GUI light directions for front-lit (flat) items.
fn items_flat_lights() -> (Vec3, Vec3) {
    let pose =
        Mat4::from_rotation_y(-std::f32::consts::PI / 8.0) * Mat4::from_rotation_x(2.356_194_5);
    let t = |x: f32, y: f32, z: f32| pose.transform_vector3(Vec3::new(x, y, z).normalize());
    (t(0.2, 1.0, -0.7), t(-0.2, 1.0, 0.7))
}

/// Pomme `lambert_shade`: two lights at 0.6 plus 0.4 ambient, capped at 1.
fn lambert_shade(n: Vec3, l0: Vec3, l1: Vec3) -> f32 {
    ((n.dot(l0).max(0.0) + n.dot(l1).max(0.0)) * 0.6 + 0.4).min(1.0)
}

/// Display transform matrix: translate, rotate X then Y then Z, scale.
pub(crate) fn display_matrix(d: &haste_assets::DisplayTransform) -> Mat4 {
    Mat4::from_translation(Vec3::from(d.translation))
        * Mat4::from_rotation_x(d.rotation[0].to_radians())
        * Mat4::from_rotation_y(d.rotation[1].to_radians())
        * Mat4::from_rotation_z(d.rotation[2].to_radians())
        * Mat4::from_scale(Vec3::from(d.scale))
}

pub(crate) fn part_matrix(t: &Option<haste_assets::PartTransformation>) -> Mat4 {
    match t {
        None => Mat4::IDENTITY,
        Some(t) => {
            Mat4::from_translation(Vec3::from(t.translation))
                * Mat4::from_quat(Quat::from_array(t.left_rotation).normalize())
                * Mat4::from_scale(Vec3::from(t.scale))
                * Mat4::from_quat(Quat::from_array(t.right_rotation).normalize())
        }
    }
}

/// Builds the slot geometry of an item: model space (0..1 blocks) through the part
/// transformation, the `display.gui` transform, and the slot mapping (16 px, y down, centred at
/// 8, 8). Back faces are dropped and the rest sorted back to front.
fn prepare_item(model: &ItemModel) -> PreparedItem {
    let mut quads: Vec<(f32, PreparedQuad)> = Vec::new();
    let slot = Mat4::from_translation(Vec3::new(8.0, 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    for part in &model.parts {
        let local = display_matrix(&part.gui) * part_matrix(&part.transformation);
        let full = slot * local * Mat4::from_translation(Vec3::splat(-0.5));
        let normal_matrix = Mat3::from_mat4(Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * local)
            .inverse()
            .transpose();
        let (l0, l1) = if part.gui_light_front {
            items_flat_lights()
        } else {
            items_3d_lights()
        };
        let tint = |index: Option<u32>| -> [u8; 4] {
            index
                .and_then(|i| part.tints.get(i as usize))
                .map(|c| rgba(*c | 0xFF00_0000))
                .unwrap_or([255; 4])
        };
        let mut push = |positions: [[f32; 3]; 4],
                        uv: [[f32; 2]; 4],
                        normal: Vec3,
                        tex: String,
                        tint: [u8; 4],
                        bias: f32| {
            let n = (normal_matrix * normal).normalize_or_zero();
            if n.z <= 1e-4 {
                return;
            }
            let shade = lambert_shade(n, l0, l1);
            let s = (shade * 255.0).round() as u8;
            let color = mul(tint, [s, s, s, 255]);
            let p = positions.map(|p| full.transform_point3(Vec3::from(p)));
            let depth = (p[0].z + p[1].z + p[2].z + p[3].z) / 4.0 + bias;
            quads.push((
                depth,
                PreparedQuad {
                    pos: p.map(|v| [v.x, v.y]),
                    uv,
                    tex: TexKey::ItemTexture(tex),
                    color,
                },
            ));
        };
        match &part.geometry {
            ItemGeometry::None => {}
            ItemGeometry::Generated { layers } => {
                // The front face of the extruded sprite (the only one visible in a slot).
                let z = 8.5 / 16.0;
                for (i, layer) in layers.iter().enumerate() {
                    push(
                        [[0.0, 1.0, z], [0.0, 0.0, z], [1.0, 0.0, z], [1.0, 1.0, z]],
                        [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
                        Vec3::Z,
                        layer.clone(),
                        tint(Some(i as u32)),
                        i as f32 * 1e-3,
                    );
                }
            }
            ItemGeometry::Elements(list) => {
                for q in list {
                    let o = q.direction.offset();
                    push(
                        q.positions,
                        q.uvs,
                        Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                        q.texture.clone(),
                        tint(q.tint_index),
                        0.0,
                    );
                }
            }
        }
    }
    quads.sort_by(|a, b| a.0.total_cmp(&b.0));
    PreparedItem {
        quads: quads.into_iter().map(|(_, q)| q).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vanilla's hotbar panel interior over a mid blue: 0.729 * 39/255 + 0.271 * 48/255 = 41.4,
    /// and the same in the other two channels (48.6, 55.6) -> 41, 49, 56.
    #[test]
    fn blend_gui_pixel_matches_vanilla_srgb_math() {
        assert_eq!(
            blend_gui_pixel([39, 37, 5, 186], [48, 80, 192]),
            [41, 49, 56, 255]
        );
        // A fully opaque sprite pixel is copied through.
        assert_eq!(
            blend_gui_pixel([10, 20, 30, 255], [48, 80, 192]),
            [10, 20, 30, 255]
        );
        // A fully transparent one leaves the background alone.
        assert_eq!(
            blend_gui_pixel([255, 255, 255, 0], [48, 80, 192]),
            [48, 80, 192, 255]
        );
        // Alpha: dst_a' = a + (1 - a) * dst_a, so a semi-transparent pixel over transparency
        // only reaches its own alpha.
        assert_eq!(blend_gui_alpha(128, 0), 128);
        assert_eq!(blend_gui_alpha(128, 51), 153);
        assert_eq!(blend_gui_alpha(0, 200), 200);
        assert_eq!(blend_gui_alpha(255, 200), 255);
    }

    /// The review's case: a 55 % alpha grey over a coloured background. Vanilla mixes the stored
    /// bytes: 0.549 * 128/255 + 0.451 * 200/255 = 0.629 -> 160, 0.417 -> 106, 0.345 -> 88.
    #[test]
    fn blend_gui_pixel_55_percent_grey_over_colour() {
        assert_eq!(
            blend_gui_pixel([128, 128, 128, 140], [200, 80, 40]),
            [160, 106, 88, 255]
        );
        // Half-way grey over the same background.
        assert_eq!(
            blend_gui_pixel([128, 128, 128, 128], [200, 80, 40]),
            [164, 104, 84, 255]
        );
    }

    /// The sRGB-target fallback blends in linear space instead, which is not vanilla: the same
    /// 55 % grey lands 6/255 brighter in red and 12 brighter in blue (0.549 * lin(128/255) +
    /// 0.451 * lin(200/255) = 0.379 -> 166), so every translucent GUI pixel (the hotbar panel,
    /// the vignette) takes on the background's encoding. On the GPU the drift is far larger: the
    /// hotbar panel texel over a blue sky measured 106 in blue against vanilla's 55.
    #[test]
    fn blend_gui_pixel_linear_is_not_vanilla() {
        assert_eq!(
            blend_gui_pixel_linear([128, 128, 128, 140], [200, 80, 40]),
            [166, 110, 100, 255]
        );
        assert_ne!(
            blend_gui_pixel([128, 128, 128, 140], [200, 80, 40]),
            blend_gui_pixel_linear([128, 128, 128, 140], [200, 80, 40])
        );
        // Opaque pixels survive the fallback exactly: encode(decode(x)) == x.
        for c in [0u8, 1, 17, 64, 128, 200, 254, 255] {
            assert_eq!(
                blend_gui_pixel_linear([c, c, c, 255], [c, c, c]),
                [c, c, c, 255]
            );
        }
    }

    /// The GUI only converts colour when its target is sRGB-encoded; with a non-sRGB view it
    /// writes vanilla's stored values directly.
    #[test]
    fn gui_writes_linear_only_for_srgb_targets() {
        assert!(gui_writes_linear(wgpu::TextureFormat::Bgra8UnormSrgb));
        assert!(gui_writes_linear(wgpu::TextureFormat::Rgba8UnormSrgb));
        assert!(!gui_writes_linear(wgpu::TextureFormat::Bgra8Unorm));
        assert!(!gui_writes_linear(wgpu::TextureFormat::Rgba8Unorm));
    }

    #[test]
    fn shadow_colour_is_a_quarter() {
        assert_eq!(shadow_color(0xFFFF_FFFF), 0xFF3F_3F3F);
        assert_eq!(shadow_color(0x80FF_8040), 0x803F_2010);
    }

    #[test]
    fn additive_blend_adds_source_to_destination() {
        // Vanilla's BlendFunction.ADDITIVE: ONE, ONE, both the colour and the alpha equation.
        let a = blend_state(GuiBlend::Additive);
        assert_eq!(a.color.src_factor, wgpu::BlendFactor::One);
        assert_eq!(a.color.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(a.color.operation, wgpu::BlendOperation::Add);
        assert_eq!(a.alpha.src_factor, wgpu::BlendFactor::One);
        assert_eq!(a.alpha.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(a.alpha.operation, wgpu::BlendOperation::Add);
        // The other modes are unchanged.
        assert_eq!(
            blend_state(GuiBlend::Normal).color.src_factor,
            wgpu::BlendFactor::SrcAlpha
        );
        assert_eq!(
            blend_state(GuiBlend::Multiply).color.dst_factor,
            wgpu::BlendFactor::OneMinusSrc
        );
        assert_eq!(
            blend_state(GuiBlend::Invert).color.src_factor,
            wgpu::BlendFactor::OneMinusDst
        );
    }

    #[test]
    fn scissor_maps_to_framebuffer_and_clamps() {
        assert_eq!(
            to_framebuffer([10.0, 20.0, 30.0, 25.0], 2.0, (100, 100)),
            [20, 40, 40, 10]
        );
        assert_eq!(
            to_framebuffer([-5.0, 0.0, 500.0, 10.0], 1.0, (100, 50)),
            [0, 0, 100, 10]
        );
    }

    #[test]
    fn shelf_packs_rows_and_reports_full() {
        let mut s = Shelf::default();
        assert_eq!(s.alloc(4000, 10), Some([0, 0]));
        assert_eq!(s.alloc(200, 20), Some([0, 10]));
        assert_eq!(s.alloc(GUI_ATLAS_SIZE, GUI_ATLAS_SIZE), None);
    }

    #[test]
    fn flat_items_are_full_bright_and_block_faces_shade_like_vanilla() {
        let (l0, l1) = items_flat_lights();
        let front = lambert_shade(Vec3::Z, l0, l1);
        assert!(front > 0.95, "flat item front shade {front}");
        // Block in the GUI: top brightest, the two visible sides darker and different.
        let model = ItemModel {
            parts: vec![haste_assets::ItemModelPart {
                geometry: ItemGeometry::Elements(
                    [
                        haste_assets::Direction::Up,
                        haste_assets::Direction::South,
                        haste_assets::Direction::East,
                        haste_assets::Direction::North,
                        haste_assets::Direction::West,
                        haste_assets::Direction::Down,
                    ]
                    .into_iter()
                    .map(|d| {
                        haste_assets::BakedQuad::from_element(d, [0.0; 3], [1.0; 3], None, None)
                    })
                    .map(|mut q| {
                        q.texture = "minecraft:block/stone".into();
                        q
                    })
                    .collect(),
                ),
                tints: vec![],
                gui: haste_assets::DisplayTransform {
                    rotation: [30.0, 225.0, 0.0],
                    translation: [0.0; 3],
                    scale: [0.625; 3],
                },
                firstperson: haste_assets::DisplayTransform::IDENTITY,
                gui_light_front: false,
                transformation: None,
            }],
        };
        let p = prepare_item(&model);
        assert_eq!(p.quads.len(), 3, "three faces visible");
        let shades: Vec<u8> = p.quads.iter().map(|q| q.color[0]).collect();
        let top = p
            .quads
            .iter()
            .find(|q| q.pos.iter().all(|v| v[1] < 8.5))
            .expect("top face above the centre")
            .color[0];
        assert_eq!(top, *shades.iter().max().unwrap(), "{shades:?}");
        // The cube fits the slot: 0.625 * 16 = 10 px wide at most (sqrt 2 diagonal ~14.1).
        for q in &p.quads {
            for v in q.pos {
                assert!(
                    (0.0..=16.0).contains(&v[0]) && (0.0..=16.0).contains(&v[1]),
                    "{v:?}"
                );
            }
        }
    }
}
