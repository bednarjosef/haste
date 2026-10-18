// Entity drawing: every `EntityInstance` posed on the CPU into one world-space vertex buffer over
// one lazily filled entity atlas, drawn in one call inside the main pass (depth-tested, before
// translucent terrain). Cube UVs follow vanilla's public box-UV skin layout (minecraft.wiki
// "Skin": head top at (8,0), front at (8,8), right side at (0,8), ...).

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use haste_assets::Assets;
use haste_core::model::{
    BodyOp, BodyTransform, CubeModel, Decoration, DroppedItem, EntityInstance, ItemFrameDecoration,
    ModelPart, PaintingDecoration, PoseOverrides,
};
use haste_core::parts::{self, PartId};
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{BlockPos, EntityId, Identifier};

use crate::gui::Shelf;

mod cape;
mod equipment;
mod item;
mod item_frame;
mod painting;
mod piston;
mod sign;
#[cfg(test)]
mod tests;

mod hand;
mod humanoid;
mod item_display;
mod model_parts;
mod vertex;

#[cfg(test)]
use hand::{arm_matrix, arm_model, hand_root_matrix, item_arm_matrix};
pub(crate) use hand::{custom_head_layer_matrix, held_item_layer_matrix};
pub(crate) use humanoid::{elytra_model, humanoid_armor};
use humanoid::{missing_item_model, missing_texture, placeholder_texture};
use item_display::item_model_bbox;
pub(crate) use item_display::{
    argb_linear, item_display_matrix, item_frame_item_transform, item_frame_transform,
    painting_transform,
};
pub(crate) use model_parts::{
    aabb_in_frustum, emit_part, emit_part_tinted, model_extents, model_top, part_matrix,
};
#[cfg(test)]
use vertex::HURT_RED_STRENGTH;
pub(crate) use vertex::{entity_shade, hurt_color, max_local_raw_brightness, push_world_quad};
pub use vertex::{lightmap_brightness, name_tag_visible, shadow_piece_alpha};
use vertex::{make_vertex_buffer, parse_tex, push_shadow_quad};

const ATLAS_SIZE: u32 = 2048;

/// Vanilla renders the player model at 15/16 scale.
const PLAYER_SCALE: f32 = 0.9375;

/// Dropped items spin about the vertical axis: `ItemEntity.getSpin`, the age in ticks
/// (plus the partial tick) over 20, plus the entity's random phase, in radians.
pub(crate) fn item_entity_spin(age: f32, bob_offs: f32) -> f32 {
    age / 20.0 + bob_offs
}

/// ... and bob up and down: `ItemEntityRenderer.submit`'s
/// `sin(age / 10 + phase) * 0.1 + 0.1` blocks, on top of the lift that puts the
/// model's bounding box above the ground (see `item_entity_lift`).
pub(crate) fn item_entity_bob(age: f32, bob_offs: f32) -> f32 {
    ((age / 10.0 + bob_offs).sin()) * 0.1 + 0.1
}

/// The lift `ItemEntityRenderer.submit` applies so the model's bounding box sits above
/// the ground: `-bbox.minY + 1/16`. The bounding box is the model drawn through its
/// `display.ground` transform (which carries vanilla's 0.5/0.25 ground scales), so no
/// extra scale is applied here.
pub(crate) fn item_entity_lift(bbox_min_y: f32) -> f32 {
    -bbox_min_y + 1.0 / 16.0
}

/// Which of vanilla's `ItemDisplayContext`s an item is drawn through: the `display`
/// section of the model it names (firstperson_righthand for the held item, fixed for
/// item frames, ground for dropped items, thirdperson for held items on mobs and players).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemDisplay {
    FirstPerson,
    /// `thirdperson_righthand` (`ItemInHandLayer`, the right hand).
    ThirdPersonRight,
    /// `thirdperson_lefthand`: the same transform with vanilla's left-hand fix (translation.x
    /// and rotation.y/z negated), `ItemTransform.apply(true, ...)`.
    ThirdPersonLeft,
    /// `head` (`CustomHeadLayer`: heads and hats worn on the head).
    Head,
    Fixed,
    Ground,
    /// `on_shelf` (`ShelfRenderer`: items on a shelf block, small and sunk into the board).
    OnShelf,
}

/// How many copies of the item a stack draws: `ItemClusterRenderState
/// .getRenderedAmount` — 1 for a single item, then one more per full 16 of the count,
/// up to 5.
pub(crate) fn item_entity_copies(count: i32) -> u32 {
    match count {
        ..=1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    }
}

/// The per-entity phase `ItemEntity.bobOffs` stands in for: vanilla draws it from the
/// entity's random at spawn, in [0, 2pi); ours hashes the entity id (indistinguishable
/// to a player).
pub(crate) fn entity_phase(id: EntityId) -> f32 {
    let mut h = (id.0 as u32).wrapping_mul(0x9E37_79B9);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    (h >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU
}

/// A deterministic [0,1) stream for the stack-copy offsets (vanilla reseeds Java's
/// `Random` with `getSeedForItemStack` — the item — each frame, so the offsets are
/// stable per item type; matching Java's sequence exactly is not required, only the
/// stability and the range).
pub(crate) fn seeded_float(seed: u32, i: u32) -> f32 {
    let mut h = seed
        .wrapping_mul(0x85EB_CA6B)
        .wrapping_add(i.wrapping_mul(0xC2B2_AE35));
    h ^= h >> 13;
    h = h.wrapping_mul(0x27D4_EB2F);
    (h >> 8) as f32 / (1u32 << 24) as f32
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EntityVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub rect: [u16; 4],
    /// Light levels for the vanilla light texture: block, sky, unused, unused.
    pub light: [u16; 4],
}

impl EntityVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Uint16x4, 4 => Uint16x4
    ];
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<EntityVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &Self::ATTRIBUTES,
    };
}

/// The texture an instance is drawn with and its model scale: players (haste-entity's
/// `haste:entity/humanoid` model) use their skin, Steve without one, and vanilla's 15/16 player
/// scale.
pub(crate) fn texture_and_scale(e: &EntityInstance) -> (Identifier, f32) {
    let humanoid = e.model.texture == Identifier::new("haste", "entity/humanoid");
    let texture = match (&e.skin, humanoid) {
        (Some(s), _) => s.clone(),
        (None, true) => Identifier::minecraft("entity/player/wide/steve"),
        (None, false) => e.model.texture.clone(),
    };
    (texture, e.scale * if humanoid { PLAYER_SCALE } else { 1.0 })
}

/// The whole-model transform `LivingEntityRenderer.setupRotations` builds: the ops compose left
/// to right (`poseStack` calls), so the first op is outermost, exactly like vanilla.
fn body_matrix(b: &BodyTransform) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    for op in b.iter() {
        m *= match op {
            BodyOp::RotateX(a) => Mat4::from_rotation_x(*a),
            BodyOp::RotateY(a) => Mat4::from_rotation_y(*a),
            BodyOp::RotateZ(a) => Mat4::from_rotation_z(*a),
            BodyOp::Translate(t) => Mat4::from_translation(*t),
        };
    }
    m
}

/// A per-tick value the renderer lerps by partial tick, the way a vanilla render state
/// extracts one (`Mth.lerp(partialTicks, xO, x)`): the value at the start and at the end of the
/// last game tick. The client owns the tick, the renderer only interpolates it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickPair {
    /// The value at the end of the previous tick (vanilla's `oX` field).
    pub start: f32,
    /// The value at the end of the last tick.
    pub end: f32,
}

impl TickPair {
    /// A value that never changes.
    pub const fn fixed(v: f32) -> TickPair {
        TickPair { start: v, end: v }
    }

    /// `Mth.lerp(partial, start, end)` at `partial` (0..1) through the tick.
    pub fn at(self, partial: f32) -> f32 {
        self.start + (self.end - self.start) * partial
    }
}

/// What the first-person view holds: the local player's skin and arm model, the item in the
/// main hand (the arm is drawn only when the hand is empty, like vanilla), the light at the
/// camera, and this tick's hand animation (swing, equip progress and bob).
///
/// Everything animated lives here as the two ends of the last tick; the renderer lerps them by
/// partial tick and applies vanilla's transforms, so all per-tick state stays in the client.
#[derive(Clone, Debug, PartialEq)]
pub struct FirstPerson {
    pub skin: Option<Identifier>,
    pub slim: bool,
    /// `AvatarRenderState.showRightSleeve`: the arm draws its sleeve layer.
    pub sleeve: bool,
    pub item: Option<haste_core::RegistryIndex>,
    pub light: (u8, u8),
    /// Swing progress 0..1 while the main hand swings, 0 when it does not
    /// (`FirstPersonHandsAndItemsRenderState`'s `attackValue`).
    pub attack: TickPair,
    /// Equip progress: 1 when the item is fully held, 0 while it switches
    /// (`FirstPersonHandsAndItems.mainHandHeight`, the inverse of vanilla's `inverseArmHeight`).
    pub equip: TickPair,
    /// Hand lag behind the camera turn, in degrees (`LocalPlayer.xBob`/`xBobO`).
    pub x_bob: TickPair,
    /// Hand lag behind the camera turn, in degrees (`LocalPlayer.yBob`/`yBobO`).
    pub y_bob: TickPair,
}

/// A name tag to draw: world anchor (above the head) and the text.
#[derive(Clone, Debug)]
pub struct Nametag {
    pub anchor: glam::DVec3,
    pub text: String,
    /// `!state.isDiscrete`: draw the see-through pass as well as the solid one
    /// (`SubmitNodeCollection.submitNameTag`).
    pub see_through: bool,
    /// The scoreboard below-name line (`Entity.belowNameDisplay`), drawn under the name.
    pub below_name: Option<String>,
}

struct EntityTexture {
    rect: Option<[u32; 4]>,
    image: Arc<image::RgbaImage>,
}

pub struct EntityRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    atlas: wgpu::Texture,
    shelf: Shelf,
    textures: HashMap<Identifier, EntityTexture>,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    vertices: Vec<EntityVertex>,
    /// The entity-shadow pipeline: same atlas/vertex format as the entity pipeline, but
    /// translucent with no depth write and no culling (vanilla `ENTITY_SHADOW`).
    shadow_pipeline: wgpu::RenderPipeline,
    shadow_buffer: wgpu::Buffer,
    shadow_capacity: usize,
    shadow_vertices: Vec<EntityVertex>,
    lights: HashMap<EntityId, (u8, u8)>,
    pub nametags: Vec<Nametag>,
    hand_buffer: wgpu::Buffer,
    hand_capacity: usize,
    hand: Vec<EntityVertex>,
    item_names: Vec<Identifier>,
    item_models: HashMap<u32, Option<Arc<haste_assets::ItemModel>>>,
    /// The first-person arm model, by [`FirstPerson::slim`] and [`FirstPerson::sleeve`]
    /// (built once on first use).
    arm_models: [Option<Arc<CubeModel>>; 4],
    /// The item frame block model quads, by `(glow, map)`, built once on first use.
    frame_models: HashMap<(bool, bool), Arc<Vec<haste_assets::BakedQuad>>>,
    /// The moving pistons' moved-block quads, by the state string (resolved once per state a
    /// piston ever moves).
    piston_quads: ahash::AHashMap<String, Option<Arc<Vec<crate::blockentity::PistonQuad>>>>,
    /// The beacon/gateway beam vertices (own pipelines: the beam ignores the lightmap), split
    /// into the opaque inner beam and the translucent glow (two draws, one buffer each), and
    /// the end portal/gateway starfield cube (position-only pipeline).
    beam_vertices: Vec<EntityVertex>,
    beam_buffer: wgpu::Buffer,
    beam_capacity: usize,
    glow_vertices: Vec<EntityVertex>,
    glow_buffer: wgpu::Buffer,
    glow_capacity: usize,
    portal_vertices: Vec<[f32; 4]>,
    portal_buffer: wgpu::Buffer,
    portal_capacity: usize,
    /// This frame's beam radius scale (vanilla `BeaconRenderer.extract`: 1 within 96 blocks
    /// or while scoping, else distance / 96), set per frame by the renderer.
    beam_radius_scale: f32,
    /// The beam and portal pipelines (see [`crate::blockentity::BeamPipelines`]).
    pub beams: crate::blockentity::BeamPipelines,
    /// The default font, for world-space sign text (glyph pages land in the entity atlas).
    font: Arc<haste_assets::BitmapFont>,
    /// Equipment asset info (`equipment/<asset>.json`), by asset id, loaded once (`None` for an
    /// asset that failed to load, so it is not retried every frame).
    equipment_infos: HashMap<Identifier, Option<Arc<haste_assets::EquipmentInfo>>>,
    pub drawn: u32,
}

impl EntityRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        lightmap_layout: &wgpu::BindGroupLayout,
        font: Arc<haste_assets::BitmapFont>,
        assets: &mut Assets,
    ) -> EntityRenderer {
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("entity-atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("entity-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("entity-bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &atlas.create_view(&Default::default()),
                ),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("entity"),
            source: wgpu::ShaderSource::Wgsl(include_str!("entity.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("entity-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout), Some(lightmap_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(EntityVertex::LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                // Vanilla's entity pipelines (ENTITY_SNIPPET and friends) use
                // `DepthStencilState.DEFAULT` (GREATER_THAN_OR_EQUAL, reverse-Z);
                // with Haste's depth cleared to 1 that is LessEqual.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // `ENTITY_SHADOW` (RenderPipelines.ENTITY_SHADOW): same vertex format and atlas as the
        // entity pipeline, but translucent with the depth test kept and no depth write
        // (`DepthStencilState(GREATER_THAN_OR_EQUAL, false)`) and no back-face culling.
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity-shadow"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(EntityVertex::LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                // Vanilla's `DepthStencilState(GREATER_THAN_OR_EQUAL, false)` (reverse-Z) is
                // LessEqual against Haste's depth cleared to 1. The shadow quad is coplanar
                // with the block top, so a small bias towards the camera replaces vanilla's
                // `VIEW_OFFSET_Z_LAYERING` polygon offset.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: -1,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = 8192;
        let vertex_buffer = make_vertex_buffer(&device, vertex_capacity);
        let shadow_capacity = 2048;
        let shadow_buffer = make_vertex_buffer(&device, shadow_capacity);
        let hand_buffer = make_vertex_buffer(&device, 1024);
        let beam_buffer = make_vertex_buffer(&device, 1024);
        let glow_buffer = make_vertex_buffer(&device, 1024);
        let portal_buffer = make_vertex_buffer(&device, 256);
        // The beam pipelines share the entity vertex format and atlas bind group (group 1)
        // but skip the lightmap (vanilla's beam shader has no lighting); the portal pipeline
        // is position-only over its own two-texture bind group.
        let beam_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("beam"),
            source: wgpu::ShaderSource::Wgsl(include_str!("beam.wgsl").into()),
        });
        let beam_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("beam-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let beam_state = |blend: Option<wgpu::BlendState>, depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if blend.is_some() {
                    "beam-glow"
                } else {
                    "beam-opaque"
                }),
                layout: Some(&beam_layout),
                vertex: wgpu::VertexState {
                    module: &beam_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(EntityVertex::LAYOUT)],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &beam_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24Plus,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let beam_opaque = beam_state(None, true);
        let beam_glow = beam_state(Some(wgpu::BlendState::ALPHA_BLENDING), false);
        // The end portal: position-only vertices, its own two samplers (the End sky and the
        // portal starfield from the jar), opaque with depth write (vanilla END_PORTAL).
        let portal_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("endportal"),
            source: wgpu::ShaderSource::Wgsl(include_str!("endportal.wgsl").into()),
        });
        let mut put_texture = |id: &str| -> wgpu::TextureView {
            let img = assets
                .texture(&haste_core::Identifier::minecraft(id))
                .unwrap_or_else(|e| {
                    tracing::warn!("end portal texture {id}: {e:#}");
                    image::RgbaImage::from_pixel(16, 16, image::Rgba([64, 0, 64, 255]))
                });
            let (w, h) = img.dimensions();
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("portal-texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            texture.create_view(&Default::default())
        };
        let end_sky = put_texture("environment/end_sky");
        let end_portal = put_texture("entity/end_portal/end_portal");
        let portal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("portal-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
        let portal_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("portal-bind"),
            layout: &portal_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&end_sky),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&end_portal),
                },
            ],
        });
        let portal_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("portal-pipeline-layout"),
                bind_group_layouts: &[Some(camera_layout), Some(&portal_layout)],
                immediate_size: 0,
            });
        let portal_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("endportal"),
            layout: Some(&portal_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &portal_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    // Position plus the layer count (15 portal, 16 gateway): the count rides
                    // per vertex so one pipeline serves both (vanilla uses two shader
                    // defines; wgpu here has no pipeline constants).
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32,
                        },
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &portal_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        EntityRenderer {
            device,
            queue,
            pipeline,
            bind_group,
            atlas,
            shelf: Shelf::default(),
            textures: HashMap::new(),
            vertex_buffer,
            vertex_capacity,
            vertices: Vec::new(),
            shadow_pipeline,
            shadow_buffer,
            shadow_capacity,
            shadow_vertices: Vec::new(),
            lights: HashMap::new(),
            nametags: Vec::new(),
            hand_buffer,
            hand_capacity: 1024,
            hand: Vec::new(),
            item_names: Vec::new(),
            item_models: HashMap::new(),
            arm_models: [None, None, None, None],
            frame_models: HashMap::new(),
            piston_quads: ahash::AHashMap::new(),
            beam_vertices: Vec::new(),
            beam_buffer,
            beam_capacity: 1024,
            glow_vertices: Vec::new(),
            glow_buffer,
            glow_capacity: 1024,
            portal_vertices: Vec::new(),
            portal_buffer,
            portal_capacity: 256,
            beam_radius_scale: 1.0,
            beams: crate::blockentity::BeamPipelines {
                opaque: beam_opaque,
                glow: beam_glow,
                portal: portal_pipeline,
                portal_bind_group,
            },
            font,
            equipment_infos: HashMap::new(),
            drawn: 0,
        }
    }

    /// Sky/block light at each entity's position (sampled by the caller with the world at hand).
    /// Entities without an entry are drawn at full sky light.
    pub fn set_lights(&mut self, lights: impl IntoIterator<Item = (EntityId, (u8, u8))>) {
        self.lights.clear();
        self.lights.extend(lights);
    }

    /// The beams' radius scale for this frame (vanilla `BeaconRenderer.extract`: 1 within 96
    /// blocks or while scoping, else the horizontal distance / 96).
    pub fn set_beam_radius_scale(&mut self, scale: f32) {
        self.beam_radius_scale = scale;
    }

    /// A font page's rectangle in the entity atlas (page images are 256x256, uploaded on first
    /// use like entity textures). Unihex glyphs are rasterised to white bitmaps and uploaded
    /// the same way, keyed by their index.
    fn glyph_page_rect(&mut self, kind: haste_assets::GlyphKind) -> Option<[u32; 4]> {
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

    fn texture_rect(&mut self, id: &Identifier, assets: &mut Assets) -> [u32; 4] {
        if !self.textures.contains_key(id) {
            let image = if id.namespace() == "haste" {
                if id.path() == "missing" {
                    missing_texture()
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

    /// Uploads `id`'s stored image into the atlas if it is not resident yet and returns its
    /// rectangle. The entry must exist (insert by `texture_rect` / `glyph_page_rect` first).
    fn resident_texture_rect(&mut self, id: &Identifier) -> [u32; 4] {
        let t = self.textures.get_mut(id).unwrap();
        if let Some(r) = t.rect {
            return r;
        }
        let (w, h) = t.image.dimensions();
        let at = match self.shelf.alloc_in(w, h, ATLAS_SIZE) {
            Some(at) => at,
            None => {
                // Full: start over (textures re-upload on demand).
                tracing::warn!("entity atlas full; repacking");
                self.shelf = Shelf::default();
                for t in self.textures.values_mut() {
                    t.rect = None;
                }
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

    /// Poses every entity into the vertex buffer. `skip` is the entity the camera is (not drawn
    /// in first person). Positions are camera-relative (`camera` subtracted in f64) like the
    /// terrain.
    pub fn prepare(
        &mut self,
        entities: &[EntityInstance],
        skip: Option<EntityId>,
        camera: glam::DVec3,
        frustum: &[glam::Vec4; 6],
        assets: &mut Assets,
    ) {
        self.vertices.clear();
        self.nametags.clear();
        self.drawn = 0;
        for e in entities {
            if Some(e.id) == skip {
                continue;
            }
            // Vanilla `EntityRenderer.shouldRender`: skip entities beyond
            // `64 * boundingBox.size` blocks (`Entity.shouldRenderAtSqrDistance`) or
            // outside the camera frustum (the AABB inflated by 0.5, `Frustum.isVisible`).
            // Same look: those entities would draw nothing visible anyway.
            let (size, half) = model_extents(&e.model.root);
            let rel = e.pos - camera;
            let dist2 = rel.length_squared();
            let limit = 64.0 * size.max(1.0) * e.scale;
            if dist2 > f64::from(limit * limit) {
                continue;
            }
            if !aabb_in_frustum(frustum, rel, half + 0.5) {
                continue;
            }
            let (sky, block) = self.lights.get(&e.id).copied().unwrap_or((15, 0));
            // The map is (sky, block); the vertex wants (block, sky).
            let light = (block, sky);
            if let Some(item) = &e.item {
                let start = self.vertices.len();
                self.emit_dropped_item(e, item, camera, light, assets);
                if self.vertices.len() > start {
                    self.drawn += 1;
                }
                continue;
            }
            if let Some(d) = &e.decoration {
                let start = self.vertices.len();
                match d {
                    Decoration::ItemFrame(f) => {
                        self.emit_item_frame(e, f, camera, sky, block, assets)
                    }
                    Decoration::Painting(p) => self.emit_painting(e, p, camera, sky, block, assets),
                    // The lightning bolt draws through the weather pass's own additive
                    // pipeline (`RenderPipelines.LIGHTNING`); nothing here.
                    Decoration::Lightning(_) => {}
                }
                if self.vertices.len() > start {
                    self.drawn += 1;
                }
                continue;
            }
            let (texture, scale) = texture_and_scale(e);
            let rect = self.texture_rect(&texture, assets);
            // `LivingEntityRenderer.setupRotations`: the body yaw turns the whole model, except a
            // sleeping entity, whose pose replaces that yaw with its own rotation sequence. The
            // body transform (death tilt, sleeping, upside-down, player flying/swimming pitch) is
            // applied between the yaw and the model scale.
            let yaw = if e.body.suppress_yaw {
                Mat4::IDENTITY
            } else {
                Mat4::from_rotation_y((180.0 - e.yaw).to_radians())
            };
            let origin = e.pos - camera + e.body.pre_translate.as_dvec3();
            // The subclass scale (`EntityRenderer.scale`) folds into the model scale: it is
            // diagonal, so it commutes with the flip and matches vanilla's order.
            let s = Vec3::new(
                -scale * e.body_scale.x,
                -scale * e.body_scale.y,
                scale * e.body_scale.z,
            );
            let root = Mat4::from_translation(origin.as_vec3())
                * yaw
                * body_matrix(&e.body)
                * Mat4::from_scale(s / 16.0);
            let start = self.vertices.len();
            emit_part_tinted(
                &mut self.vertices,
                &e.model,
                &e.model.root,
                &e.pose,
                root,
                rect,
                light,
                e.hurt,
                [1.0; 3],
            );
            // Overlay layers: the same transform and pose, each with its own texture and
            // tint (vanilla's RenderLayers, e.g. SheepWoolLayer's dyed copy of the model).
            for overlay in e.overlays.iter().flatten() {
                let Some((oroot, hurt)) = self.overlay_root(e, &overlay.model, root, assets) else {
                    continue;
                };
                let orect = self.texture_rect(&overlay.model.texture, assets);
                emit_part_tinted(
                    &mut self.vertices,
                    &overlay.model,
                    &overlay.model.root,
                    &e.pose,
                    oroot,
                    orect,
                    light,
                    hurt,
                    overlay.tint,
                );
            }
            self.emit_equipment(e, root, light, assets);
            if self.vertices.len() > start {
                self.drawn += 1;
            }
            if let Some(name) = &e.nametag {
                // `EntityRenderer.extractNameTags`: within 64 blocks, and past 32 when
                // crouching (`LivingEntityRenderer.shouldShowName`). Invisible entities never
                // reach here (haste-entity drops their tag). The anchor is the NAME_TAG
                // attachment (bb height) plus 0.5.
                let dist_sq = (e.pos - camera).length_squared();
                if name_tag_visible(dist_sq, e.is_discrete) {
                    self.nametags.push(Nametag {
                        anchor: e.pos + glam::DVec3::new(0.0, e.name_tag_y as f64 + 0.5, 0.0),
                        text: name.clone(),
                        see_through: !e.is_discrete,
                        below_name: e.below_name.clone(),
                    });
                }
            }
        }
        if self.vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = make_vertex_buffer(&self.device, self.vertex_capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }

    /// Vanilla `EntityRenderer.extractShadow`: the round `misc/shadow.png` quads on the full
    /// top faces below each entity. For every entity the shadow is a `shadowRadius` (capped at
    /// 32) disc, its power fading with camera distance (`1 - distSq/256`) times the renderer's
    /// shadow strength, and with height above the block (`extractShadowPiece`); the alpha also
    /// scales with the block's raw brightness through `Lightmap.getBrightness`. Skipped for
    /// invisible entities and when the entity-shadows option is off. `sky_darken` is
    /// `Level.getSkyDarken` (0..15) and `ambient_light` the dimension's `ambientLight`.
    ///
    /// Batched into one reused vertex buffer, no per-frame allocation.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_shadows(
        &mut self,
        entities: &[EntityInstance],
        skip: Option<EntityId>,
        camera: glam::DVec3,
        world: &dyn BlockView,
        shapes: &dyn CollisionShapes,
        sky_darken: i32,
        ambient_light: f32,
        entity_shadows: bool,
        assets: &mut Assets,
    ) {
        self.shadow_vertices.clear();
        if !entity_shadows {
            return;
        }
        let rect = self.texture_rect(&Identifier::minecraft("misc/shadow"), assets);
        for e in entities {
            if Some(e.id) == skip || e.invisible || e.decoration.is_some() {
                continue;
            }
            let radius = e.shadow_radius.min(32.0);
            if radius <= 0.0 {
                continue;
            }
            let dist_sq = (e.pos - camera).length_squared();
            let pow = (1.0 - dist_sq / 256.0) as f32 * e.shadow_strength;
            if pow <= 0.0 {
                continue;
            }
            let x0 = (e.pos.x - radius as f64).floor() as i32;
            let x1 = (e.pos.x + radius as f64).floor() as i32;
            let z0 = (e.pos.z - radius as f64).floor() as i32;
            let z1 = (e.pos.z + radius as f64).floor() as i32;
            let depth = (pow / 0.5 - 1.0).min(radius);
            let y0 = (e.pos.y - depth as f64).floor() as i32;
            let y1 = e.pos.y.floor() as i32;
            for z in z0..=z1 {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        let pos = BlockPos::new(x, y, z);
                        let power_at_depth = pow - (e.pos.y - y as f64) as f32 * 0.5;
                        let below = BlockPos::new(x, y - 1, z);
                        let below_state = world.block_state(below);
                        // `isCollisionShapeFullBlock`: exactly one full 0..1 box.
                        let boxes = shapes.collision_boxes(below_state);
                        let full = boxes.len() == 1
                            && boxes[0].min == glam::DVec3::ZERO
                            && boxes[0].max == glam::DVec3::ONE;
                        if !full {
                            continue;
                        }
                        let brightness = max_local_raw_brightness(world, pos, sky_darken);
                        if brightness <= 3 {
                            continue;
                        }
                        let b = lightmap_brightness(ambient_light, brightness);
                        let alpha = shadow_piece_alpha(power_at_depth, b);
                        if alpha <= 0.0 {
                            continue;
                        }
                        let r = 2.0 * radius as f64;
                        let bx = x as f64;
                        let bz = z as f64;
                        let by = y as f64;
                        let x01 = bx - e.pos.x;
                        let x11 = bx + 1.0 - e.pos.x;
                        let z01 = bz - e.pos.z;
                        let z11 = bz + 1.0 - e.pos.z;
                        let u0 = (-x01 / r) as f32 + 0.5;
                        let u1 = (-x11 / r) as f32 + 0.5;
                        let v0 = (-z01 / r) as f32 + 0.5;
                        let v1 = (-z11 / r) as f32 + 0.5;
                        let rel = |wx: f64, wy: f64, wz: f64| {
                            Vec3::new(
                                (wx - camera.x) as f32,
                                (wy - camera.y) as f32,
                                (wz - camera.z) as f32,
                            )
                        };
                        // `ShadowFeatureRenderer.prepare`: (x01,z01), (x01,z11), (x11,z11),
                        // (x11,z01), all at y01 = the block top.
                        let p = [
                            rel(bx, by, bz),
                            rel(bx, by, bz + 1.0),
                            rel(bx + 1.0, by, bz + 1.0),
                            rel(bx + 1.0, by, bz),
                        ];
                        let uv = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
                        push_shadow_quad(&mut self.shadow_vertices, p, uv, alpha, rect);
                    }
                }
            }
        }
        if self.shadow_vertices.len() > self.shadow_capacity {
            self.shadow_capacity = self.shadow_vertices.len().next_power_of_two();
            self.shadow_buffer = make_vertex_buffer(&self.device, self.shadow_capacity);
        }
        if !self.shadow_vertices.is_empty() {
            self.queue.write_buffer(
                &self.shadow_buffer,
                0,
                bytemuck::cast_slice(&self.shadow_vertices),
            );
        }
    }

    /// Draws the batched entity shadows; the camera and lightmap bind groups must be set.
    pub fn draw_shadows<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.shadow_vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.shadow_buffer.slice(..));
        pass.draw(0..self.shadow_vertices.len() as u32, 0..1);
        1
    }

    /// Number of shadow quads batched for the last frame (6 vertices each); tests/reporting.
    pub fn shadow_quad_count(&self) -> usize {
        self.shadow_vertices.len() / 6
    }

    /// Item registry names by protocol id (for the held item).
    pub fn set_item_names(&mut self, names: Vec<Identifier>) {
        self.item_names = names;
        self.item_models.clear();
    }

    /// Poses the frame's block entities (chests, shulker boxes) into the same vertex stream as
    /// the entities, after `prepare`: one draw call covers both, and each texture lands in the
    /// shared atlas on first use (the per-vertex `rect` picks it). `visible` decides per
    /// instance whether it is drawn (frustum and section culling are the caller's, like the
    /// terrain's); returns how many were posed. `orientation` is the camera's orientation
    /// quaternion, for the conduit's billboarded eye.
    pub fn append_block_entities(
        &mut self,
        instances: &[crate::blockentity::BlockEntityInstance],
        camera: glam::DVec3,
        visible: impl Fn(&crate::blockentity::BlockEntityInstance) -> bool,
        partial: f32,
        assets: &mut Assets,
        orientation: glam::Quat,
    ) -> u32 {
        let start = self.vertices.len();
        let mut drawn = 0;
        self.beam_vertices.clear();
        self.glow_vertices.clear();
        self.portal_vertices.clear();
        for inst in instances {
            if !visible(inst) {
                continue;
            }
            // Beacon beams and the end gateways' beams go into the beam stream (their own
            // pipelines, no lightmap); end portals and gateway cubes into the portal stream
            // (position-only starfield shader). Both are emitted here and drawn by the
            // renderer's `draw_beams`/`draw_portals` after the entities. A gateway draws BOTH
            // (its cube and, while spawning or cooling down, its beam), so the beam branch
            // must not `continue` past the portal one.
            if matches!(
                inst.kind,
                crate::blockentity::BlockEntityKind::Beacon
                    | crate::blockentity::BlockEntityKind::EndGateway
            ) {
                let mut inner = std::mem::take(&mut self.beam_vertices);
                let mut glow = std::mem::take(&mut self.glow_vertices);
                let radius_scale = self.beam_radius_scale;
                let me = &mut *self;
                match inst.kind {
                    crate::blockentity::BlockEntityKind::Beacon => {
                        crate::blockentity::emit_beacon_beam(
                            &mut inner,
                            &mut glow,
                            inst,
                            camera,
                            radius_scale,
                            &mut |id| me.texture_rect(id, assets),
                        );
                    }
                    _ => crate::blockentity::emit_gateway_beam(
                        &mut inner,
                        &mut glow,
                        inst,
                        camera,
                        &mut |id| me.texture_rect(id, assets),
                    ),
                }
                self.beam_vertices = inner;
                self.glow_vertices = glow;
                drawn += 1;
                if matches!(inst.kind, crate::blockentity::BlockEntityKind::Beacon) {
                    continue;
                }
            }
            if matches!(
                inst.kind,
                crate::blockentity::BlockEntityKind::EndPortal
                    | crate::blockentity::BlockEntityKind::EndGateway
            ) {
                let mut out = std::mem::take(&mut self.portal_vertices);
                // Vanilla `TheEndPortalRenderer.TRANSFORMATION`: translate y +0.375, scale
                // (1, 0.375, 1); the gateway draws the whole cube.
                let m = (inst.pos.as_dvec3() - camera).as_vec3();
                let (root, layers) = match inst.kind {
                    crate::blockentity::BlockEntityKind::EndPortal => (
                        Mat4::from_translation(m)
                            * Mat4::from_translation(Vec3::new(0.0, 0.375, 0.0))
                            * Mat4::from_scale(Vec3::new(1.0, 0.375, 1.0)),
                        15.0,
                    ),
                    _ => (Mat4::from_translation(m), 16.0),
                };
                crate::blockentity::emit_portal_faces(&mut out, inst.portal_faces, layers, root);
                self.portal_vertices = out;
                drawn += 1;
                continue;
            }
            if let crate::blockentity::BlockEntityKind::Sign { hanging } = inst.kind {
                self.emit_sign_text(inst, hanging, camera);
                drawn += 1;
                continue;
            }
            if let crate::blockentity::BlockEntityKind::Banner { .. } = inst.kind {
                // The banner's angle is the same rule as the sign's (rotation segment or
                // wall facing) and lives in `sign_angle`.
                let mut out = std::mem::take(&mut self.vertices);
                let me = &mut *self;
                crate::blockentity::emit_banner(
                    &mut out,
                    inst,
                    camera,
                    inst.sign_angle,
                    &mut |texture| me.texture_rect(texture, assets),
                );
                self.vertices = out;
                drawn += 1;
                continue;
            }
            // The item and mob block entities draw through the item and cube emitters: the
            // item models come from the same asset chain the held and dropped items use
            // (cached per registry id, so a frame walks the cache, never the jar).
            if let Some(kind) = crate::blockentity::item_kind(&inst.kind) {
                let light = (inst.light.1, inst.light.0);
                let mut out = std::mem::take(&mut self.vertices);
                match kind {
                    crate::blockentity::ItemBlockEntity::Campfire(campfire) => {
                        for slot in 0..campfire.items.len() {
                            let Some(item) = campfire.get(slot) else {
                                continue;
                            };
                            let Some(model) = self.item_model_for(item, assets) else {
                                continue;
                            };
                            let m = crate::blockentity::campfire_item_matrix(
                                inst.pos,
                                inst.facing,
                                slot,
                                camera,
                            );
                            self.emit_item(&mut out, &model, m, light, ItemDisplay::Fixed, assets);
                        }
                    }
                    crate::blockentity::ItemBlockEntity::Shelf(shelf) => {
                        for slot in 0..shelf.items.len() {
                            let Some(item) = shelf.get(slot) else {
                                continue;
                            };
                            let Some(model) = self.item_model_for(item, assets) else {
                                continue;
                            };
                            let bbox = item_model_bbox(&model, &ItemDisplay::OnShelf);
                            let m = crate::blockentity::shelf_item_matrix(
                                inst.pos,
                                inst.facing,
                                slot,
                                shelf.align_to_bottom,
                                crate::blockentity::shelf_item_offset_y(
                                    bbox,
                                    shelf.align_to_bottom,
                                ),
                                camera,
                            );
                            self.emit_item(
                                &mut out,
                                &model,
                                m,
                                light,
                                ItemDisplay::OnShelf,
                                assets,
                            );
                        }
                    }
                    crate::blockentity::ItemBlockEntity::Vault(vault) => {
                        let copies = vault.copies();
                        if copies > 0 {
                            if let Some(model) = self.item_model_for(vault.item, assets) {
                                let bbox = item_model_bbox(&model, &ItemDisplay::Ground);
                                let base = crate::blockentity::vault_root(
                                    inst.pos,
                                    vault.spin_at(partial),
                                    camera,
                                );
                                let rand = |i: u32| seeded_float(vault.item.0, i);
                                self.emit_item_copies(
                                    &mut out,
                                    &model,
                                    base,
                                    copies,
                                    rand,
                                    light,
                                    ItemDisplay::Ground,
                                    bbox,
                                    assets,
                                );
                            }
                        }
                    }
                    crate::blockentity::ItemBlockEntity::Brushable(b) => {
                        let Some(dir) = b.hit_direction else {
                            continue;
                        };
                        let Some(model) = self.item_model_for(b.item, assets) else {
                            continue;
                        };
                        let m = crate::blockentity::brushable_item_matrix(
                            inst.pos, dir, b.dusted, camera,
                        );
                        self.emit_item(&mut out, &model, m, light, ItemDisplay::Fixed, assets);
                    }
                }
                self.vertices = out;
                drawn += 1;
                continue;
            }
            if matches!(
                inst.kind,
                crate::blockentity::BlockEntityKind::Lectern
                    | crate::blockentity::BlockEntityKind::EnchantTable
                    | crate::blockentity::BlockEntityKind::Bell
                    | crate::blockentity::BlockEntityKind::DecoratedPot
                    | crate::blockentity::BlockEntityKind::Conduit
            ) {
                // The object-model block entities (books, bells, decorated pots, conduits):
                // one part list each, their own texture per part through the same atlas.
                let texture = match inst.kind {
                    crate::blockentity::BlockEntityKind::Lectern => {
                        crate::blockentity::book_texture()
                    }
                    crate::blockentity::BlockEntityKind::EnchantTable => {
                        crate::blockentity::book_texture()
                    }
                    crate::blockentity::BlockEntityKind::Bell => crate::blockentity::bell_texture(),
                    crate::blockentity::BlockEntityKind::DecoratedPot => {
                        crate::blockentity::pot_base_texture()
                    }
                    _ => crate::blockentity::conduit_shell_texture(),
                };
                let rect = self.texture_rect(&texture, assets);
                let mut out = std::mem::take(&mut self.vertices);
                let me = &mut *self;
                crate::blockentity::emit_model_block_entity(
                    &mut out,
                    inst,
                    camera,
                    orientation,
                    rect,
                    &mut |id| me.texture_rect(id, assets),
                );
                self.vertices = out;
                drawn += 1;
                continue;
            }
            // Moving pistons resolve their moved block's model through the same atlas (the
            // quads are cached per state string, so only the first frame pays the lookup).
            if matches!(inst.kind, crate::blockentity::BlockEntityKind::MovingPiston) {
                self.emit_piston(inst, camera, assets);
                drawn += 1;
                continue;
            }
            // A spawner's display entity is a cube model posed by the block entity's own
            // transform (`emit_spawner`), not by the item emitters.
            if let crate::blockentity::BlockEntityKind::Spawner(ref spawner) = inst.kind {
                if let Some(model) = spawner.model.as_ref() {
                    let rect = self.texture_rect(&model.texture, assets);
                    let mut out = std::mem::take(&mut self.vertices);
                    crate::blockentity::emit_spawner(&mut out, inst, model, rect, camera, partial);
                    self.vertices = out;
                    drawn += 1;
                }
                continue;
            }
            let texture = match inst.kind {
                crate::blockentity::BlockEntityKind::Chest(m) => {
                    crate::blockentity::chest_texture(m, inst.half)
                }
                crate::blockentity::BlockEntityKind::Shulker(c) => {
                    crate::blockentity::shulker_texture(c)
                }
                crate::blockentity::BlockEntityKind::Skull { kind, .. } => {
                    crate::blockentity::skull_texture(kind)
                }
                crate::blockentity::BlockEntityKind::Sign { .. } => unreachable!(),
                crate::blockentity::BlockEntityKind::Banner { .. } => unreachable!(),
                // The item and mob block entities left already.
                crate::blockentity::BlockEntityKind::Campfire(_)
                | crate::blockentity::BlockEntityKind::Spawner(_)
                | crate::blockentity::BlockEntityKind::Vault(_)
                | crate::blockentity::BlockEntityKind::Shelf(_)
                | crate::blockentity::BlockEntityKind::Brushable(_) => unreachable!(),
                crate::blockentity::BlockEntityKind::Lectern
                | crate::blockentity::BlockEntityKind::EnchantTable
                | crate::blockentity::BlockEntityKind::Bell
                | crate::blockentity::BlockEntityKind::DecoratedPot
                | crate::blockentity::BlockEntityKind::Conduit => unreachable!(),
                crate::blockentity::BlockEntityKind::Beacon
                | crate::blockentity::BlockEntityKind::EndPortal
                | crate::blockentity::BlockEntityKind::EndGateway
                | crate::blockentity::BlockEntityKind::MovingPiston => unreachable!(),
            };
            let rect = self.texture_rect(&texture, assets);
            let out = &mut self.vertices;
            let emit = |out: &mut Vec<EntityVertex>,
                        model: &CubeModel,
                        part: &ModelPart,
                        pose: &PoseOverrides,
                        m: Mat4,
                        rect: [u32; 4],
                        light: (u8, u8)| {
                emit_part(out, model, part, pose, m, rect, light, 0.0);
            };
            crate::blockentity::emit_block_entity(out, inst, camera, rect, &emit);
            drawn += 1;
        }
        if self.vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = make_vertex_buffer(&self.device, self.vertex_capacity);
        }
        if self.vertices.len() > start && !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
        if self.beam_vertices.len() > self.beam_capacity {
            self.beam_capacity = self.beam_vertices.len().next_power_of_two();
            self.beam_buffer = make_vertex_buffer(&self.device, self.beam_capacity);
        }
        if !self.beam_vertices.is_empty() {
            self.queue.write_buffer(
                &self.beam_buffer,
                0,
                bytemuck::cast_slice(&self.beam_vertices),
            );
        }
        if self.glow_vertices.len() > self.glow_capacity {
            self.glow_capacity = self.glow_vertices.len().next_power_of_two();
            self.glow_buffer = make_vertex_buffer(&self.device, self.glow_capacity);
        }
        if !self.glow_vertices.is_empty() {
            self.queue.write_buffer(
                &self.glow_buffer,
                0,
                bytemuck::cast_slice(&self.glow_vertices),
            );
        }
        if self.portal_vertices.len() > self.portal_capacity {
            self.portal_capacity = self.portal_vertices.len().next_power_of_two();
            self.portal_buffer = make_vertex_buffer(&self.device, self.portal_capacity);
        }
        if !self.portal_vertices.is_empty() {
            self.queue.write_buffer(
                &self.portal_buffer,
                0,
                bytemuck::cast_slice(&self.portal_vertices),
            );
        }
        self.drawn += drawn;
        drawn
    }

    /// Whether any beam or portal vertices were posed this frame (the draw calls are skipped).
    pub fn has_beams_or_portals(&self) -> bool {
        !self.beam_vertices.is_empty()
            || !self.glow_vertices.is_empty()
            || !self.portal_vertices.is_empty()
    }

    /// Draws the beacon/gateway beams: the opaque inner beams first (depth write, vanilla
    /// `RenderPipelines.BEACON_BEAM_OPAQUE`), then the translucent glow without depth write
    /// (`BEACON_BEAM_TRANSLUCENT`). Into the main pass; the camera bind group at group 0 and
    /// the entity atlas at group 1 must already be set.
    pub fn draw_beams<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        let mut calls = 0;
        if !self.beam_vertices.is_empty() {
            pass.set_pipeline(&self.beams.opaque);
            pass.set_bind_group(1, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.beam_buffer.slice(..));
            pass.draw(0..self.beam_vertices.len() as u32, 0..1);
            calls += 1;
        }
        if !self.glow_vertices.is_empty() {
            pass.set_pipeline(&self.beams.glow);
            pass.set_bind_group(1, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.glow_buffer.slice(..));
            pass.draw(0..self.glow_vertices.len() as u32, 0..1);
            calls += 1;
        }
        calls
    }

    /// Draws the end portal/gateway starfield cubes (vanilla `RenderPipelines.END_PORTAL` /
    /// `END_GATEWAY`): position-only vertices. The camera bind group must be set at group 0.
    pub fn draw_portals<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.portal_vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.beams.portal);
        pass.set_bind_group(1, &self.beams.portal_bind_group, &[]);
        pass.set_vertex_buffer(0, self.portal_buffer.slice(..));
        pass.draw(0..self.portal_vertices.len() as u32, 0..1);
        1
    }

    /// Draws into the main pass; the camera bind group must be set at group 0.
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertices.len() as u32, 0..1);
        1
    }
}
