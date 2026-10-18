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

    /// Draws an entity's equipment on top of its model (already emitted at `root`): the held
    /// items (`ItemInHandLayer`) and armour (`HumanoidArmorLayer`). Unknown items and equipment
    /// draw nothing. The vertex buffer is reused (taken out and put back) so nothing allocates
    /// per frame.
    fn emit_equipment(
        &mut self,
        e: &EntityInstance,
        root: Mat4,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        // Held items: vanilla draws the right hand first, then the left.
        for (i, item) in e.hands.iter().enumerate() {
            let Some(idx) = *item else { continue };
            let Some(model) = self.item_model_for(idx, assets) else {
                continue;
            };
            let left = i == 1;
            let arm = if left {
                parts::LEFT_ARM
            } else {
                parts::RIGHT_ARM
            };
            let Some(arm_m) = part_matrix(&e.model.root, &e.pose, Mat4::IDENTITY, arm) else {
                continue;
            };
            let item_root = root * arm_m * held_item_layer_matrix(left);
            let display = if left {
                ItemDisplay::ThirdPersonLeft
            } else {
                ItemDisplay::ThirdPersonRight
            };
            let mut out = std::mem::take(&mut self.vertices);
            self.emit_item(&mut out, &model, item_root, light, display, assets);
            self.vertices = out;
        }
        // Armour, in vanilla's order (`HumanoidArmorLayer.submit`: chest, legs, feet, head).
        for slot in [
            haste_core::model::EquipmentSlot::Chest,
            haste_core::model::EquipmentSlot::Legs,
            haste_core::model::EquipmentSlot::Feet,
            haste_core::model::EquipmentSlot::Head,
        ] {
            let Some(piece) = e.equipment.iter().find(|p| p.slot == slot) else {
                continue;
            };
            let Some(asset) = &piece.asset else { continue };
            let Some(info) = self.equipment_info(asset, assets) else {
                continue;
            };
            // `HumanoidArmorLayer`: leggings use the inner (leggings) layer, the rest the
            // humanoid one (babies are not modelled).
            let layer_type = if slot == haste_core::model::EquipmentSlot::Legs {
                "humanoid_leggings"
            } else {
                "humanoid"
            };
            let model = humanoid_armor(slot);
            let mut out = std::mem::take(&mut self.vertices);
            for layer in info.layers_of(layer_type) {
                // `EquipmentLayerRenderer.getColorForLayer`: the stack's dye, else the layer's
                // undyed colour, else no tint.
                let tint = piece
                    .dye
                    .or(layer.dyeable)
                    .map(argb_linear)
                    .unwrap_or([1.0; 3]);
                let rect = self.texture_rect(&layer.texture, assets);
                emit_part_tinted(
                    &mut out,
                    &model,
                    &model.root,
                    &e.pose,
                    root,
                    rect,
                    light,
                    e.hurt,
                    tint,
                );
            }
            // Wings (`WingsLayer`): an elytra on the chest draws the WINGS layers on the elytra
            // model, pushed 0.125 blocks back (the wings' flap animation is not modelled yet).
            if slot == haste_core::model::EquipmentSlot::Chest {
                let elytra = elytra_model();
                let wings_root = root * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.125));
                for layer in info.layers_of("wings") {
                    let tint = piece
                        .dye
                        .or(layer.dyeable)
                        .map(argb_linear)
                        .unwrap_or([1.0; 3]);
                    let texture = cape::player_elytra_texture(e)
                        .filter(|_| layer.use_player_texture)
                        .unwrap_or(&layer.texture);
                    let rect = self.texture_rect(texture, assets);
                    emit_part_tinted(
                        &mut out,
                        &elytra,
                        &elytra.root,
                        &e.pose,
                        wings_root,
                        rect,
                        light,
                        e.hurt,
                        tint,
                    );
                }
            }
            self.vertices = out;
        }
        // Custom head (vanilla `CustomHeadLayer`): the head-slot item when it is not armour
        // (pumpkins, heads), through the head part's animated transform.
        if let Some(idx) = e.head_item {
            if let Some(model) = self.item_model_for(idx, assets) {
                if let Some(head_m) =
                    part_matrix(&e.model.root, &e.pose, Mat4::IDENTITY, parts::HEAD)
                {
                    let head_root = root * head_m * custom_head_layer_matrix();
                    let mut out = std::mem::take(&mut self.vertices);
                    self.emit_item(
                        &mut out,
                        &model,
                        head_root,
                        light,
                        ItemDisplay::Head,
                        assets,
                    );
                    self.vertices = out;
                }
            }
        }
    }

    /// Poses a dropped item entity (`minecraft:item`) into the vertex buffer, following
    /// `ItemEntityRenderer.submit` and `ItemClusterRenderState.submitMultipleFromCount`: the
    /// item's own model (the one the GUI and held items use) through its `display.ground`
    /// transform — which carries vanilla's ground scales, a flat item 0.5 and a block 0.25 —
    /// lifted so its bounding box sits 1/16 above the ground, bobbing and spinning with the
    /// entity's random phase, and a stack's copies spread like vanilla's. An unknown item
    /// draws the missing model (vanilla's missingno cube); nothing panics.
    fn emit_dropped_item(
        &mut self,
        e: &EntityInstance,
        dropped: &DroppedItem,
        camera: glam::DVec3,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let model = self
            .item_model_for(dropped.item, assets)
            .unwrap_or_else(missing_item_model);
        // `ItemEntity.bobOffs`: a per-entity random phase in [0, 2pi); ours is a hash of
        // the entity id (the player cannot tell the difference).
        let bob_offs = entity_phase(e.id);
        // The model's bounding box through the ground transform: it decides the lift and
        // whether the copies of a stack spread in 3D or line up flat
        // (`submitMultipleFromCount` branches on the z size).
        let (bbox_min, bbox_max) = item_model_bbox(&model, &ItemDisplay::Ground);
        let lift = item_entity_lift(bbox_min.y);
        let base = Mat4::from_translation(
            (e.pos - camera).as_vec3()
                + Vec3::new(0.0, lift + item_entity_bob(dropped.age, bob_offs), 0.0),
        ) * Mat4::from_rotation_y(item_entity_spin(dropped.age, bob_offs));
        // `ItemClusterRenderState.getSeedForItemStack`: the renderer reseeds its random
        // with the item each frame, so the copy offsets are stable per item type (ours
        // hashes the registry id and stream index; matching Java's Random is not needed).
        let rand = |i: u32| seeded_float(dropped.item.0, i);
        let mut out = std::mem::take(&mut self.vertices);
        self.emit_item_copies(
            &mut out,
            &model,
            base,
            item_entity_copies(dropped.count),
            rand,
            light,
            ItemDisplay::Ground,
            (bbox_min, bbox_max),
            assets,
        );
        self.vertices = out;
    }

    /// The copies of an item stack, laid out like vanilla's
    /// `ItemEntityRenderer.submitMultipleFromCount`: the first copy at `base`, then one per
    /// extra copy of the stack, offset by `+-0.15` in all three axes when the model is thick,
    /// or lined up along z (by 1.5x the model's depth, jittered `+-0.075` in x and y) when
    /// it is flat. `rand` supplies the jitter in [0,1) by stream index; the block entities
    /// that draw a stack (the vault) and the dropped items share this exact layout.
    #[allow(clippy::too_many_arguments)]
    fn emit_item_copies(
        &mut self,
        out: &mut Vec<EntityVertex>,
        model: &haste_assets::ItemModel,
        base: Mat4,
        copies: u32,
        rand: impl Fn(u32) -> f32,
        light: (u8, u8),
        display: ItemDisplay,
        bbox: (Vec3, Vec3),
        assets: &mut Assets,
    ) {
        let (bbox_min, bbox_max) = bbox;
        let depth = bbox_max.z - bbox_min.z;
        let flat = depth <= 1.0 / 16.0;
        let step = depth * 1.5;
        for copy in 0..copies {
            let mut off = Vec3::ZERO;
            if copy > 0 {
                if flat {
                    off = Vec3::new(
                        (rand(2 * copy - 2) * 2.0 - 1.0) * 0.075,
                        (rand(2 * copy - 1) * 2.0 - 1.0) * 0.075,
                        step * copy as f32 - step * (copies.saturating_sub(1)) as f32 / 2.0,
                    );
                } else {
                    off = Vec3::new(
                        (rand(3 * copy - 3) * 2.0 - 1.0) * 0.15,
                        (rand(3 * copy - 2) * 2.0 - 1.0) * 0.15,
                        (rand(3 * copy - 1) * 2.0 - 1.0) * 0.15,
                    );
                }
            }
            let m = base * Mat4::from_translation(off);
            self.emit_item(out, model, m, light, display, assets);
        }
    }

    /// One item frame, following vanilla's `ItemFrameRenderer.submit`: the frame block model at
    /// `direction * 0.46875` turned to face `direction`, then the framed item rotated by
    /// `rotation * 45` degrees at scale 0.5. Glow frames light the frame at least block-light 5
    /// and draw the item full-bright.
    fn emit_item_frame(
        &mut self,
        e: &EntityInstance,
        f: &ItemFrameDecoration,
        camera: glam::DVec3,
        sky: u8,
        block: u8,
        assets: &mut Assets,
    ) {
        let base =
            Mat4::from_translation((e.pos - camera).as_vec3()) * item_frame_transform(f.facing);

        let map = f
            .item
            .and_then(|i| self.item_names.get(i.0 as usize))
            .is_some_and(|n| n.path() == "filled_map");
        let quads = self.frame_quads(f.glow, map, assets);
        // `ItemFrameRenderer.getBlockLightLevel`: glow frames are at least block-light 5.
        let frame_light = if f.glow {
            (block.max(5), sky)
        } else {
            (block, sky)
        };
        let model = base * Mat4::from_translation(Vec3::splat(-0.5));
        let nm = Mat3::from_mat4(model).inverse().transpose();
        let mut out = std::mem::take(&mut self.vertices);
        for q in quads.iter() {
            let rect = self.texture_rect(&parse_tex(&q.texture), assets);
            let o = q.direction.offset();
            push_world_quad(
                &mut out,
                model,
                nm,
                q.positions.map(Vec3::from),
                q.uvs,
                Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                [1.0; 3],
                frame_light,
                rect,
            );
        }
        if let Some(idx) = f.item {
            if let Some(item) = self.item_model_for(idx, assets) {
                // Glow frames draw the item full-bright (`getLightCoords(isGlowFrame, 15728880)`,
                // block 15 / sky 15); otherwise the frame's own (block, sky) light.
                let item_light = if f.glow { (15, 15) } else { (block, sky) };
                let root = base * item_frame_item_transform(f.rotation);
                self.emit_item(
                    &mut out,
                    &item,
                    root,
                    item_light,
                    ItemDisplay::Fixed,
                    assets,
                );
            }
        }
        self.vertices = out;
    }

    /// One moving piston, following vanilla `PistonHeadRenderer`: the moved block's model
    /// (from the block entity's NBT state string, `short`/`extended` adjusted like
    /// `extractRenderState` does) at the interpolated progress offset, plus the retracting
    /// source piston's base. The quads are resolved once per state string and cached; an
    /// unreadable model draws nothing, never a panic.
    fn emit_piston(
        &mut self,
        inst: &crate::blockentity::BlockEntityInstance,
        camera: glam::DVec3,
        assets: &mut Assets,
    ) {
        use crate::blockentity::{
            piston_drawn_states, piston_extended_progress, piston_quads, PistonQuad,
        };
        let Some(p) = inst.piston.clone() else {
            return;
        };
        // The progress pair already folds the partial tick (the client's anim ends).
        let progress = p.progress.end;
        let (block_state, head_state) = piston_drawn_states(&p, progress);
        let off = piston_extended_progress(&p, progress);
        // Vanilla `getMovementDirection`: the facing while extending, its opposite
        // retracting.
        let movement = if p.extending {
            p.facing
        } else {
            p.facing.opposite()
        };
        let step = p.facing.offset();
        let offset = Vec3::new(
            step.x as f32 * off,
            step.y as f32 * off,
            step.z as f32 * off,
        );
        let (sky, block) = inst.light;
        let light = (block, sky);
        let base = (inst.pos.as_dvec3() - camera).as_vec3();
        let mut out = std::mem::take(&mut self.vertices);
        let mut emit_state = |me: &mut Self, out: &mut Vec<EntityVertex>, state: &str, at: Vec3| {
            let quads: Option<Arc<Vec<PistonQuad>>> = match me.piston_quads.get(state) {
                Some(q) => q.clone(),
                None => {
                    let q = piston_quads(assets, state);
                    me.piston_quads.insert(state.to_string(), q.clone());
                    q
                }
            };
            let Some(quads) = quads else {
                return;
            };
            let m = Mat4::from_translation(base + at);
            let nm = Mat3::from_mat4(m).inverse().transpose();
            for (q, rx, ry) in quads.iter() {
                // The variant's x/y rotation around the block centre, baked into the matrix.
                let mut rot = Mat4::IDENTITY;
                if *rx != 0 {
                    rot *= Mat4::from_rotation_x((*rx as f32).to_radians());
                }
                if *ry != 0 {
                    rot *= Mat4::from_rotation_y((*ry as f32).to_radians());
                }
                let rot = Mat4::from_translation(Vec3::splat(0.5))
                    * rot
                    * Mat4::from_translation(-Vec3::splat(0.5));
                let m = m * rot;
                let rect = me.texture_rect(&parse_tex(&q.texture), assets);
                let o = q.direction.offset();
                push_world_quad(
                    out,
                    m,
                    nm,
                    q.positions.map(Vec3::from),
                    q.uvs,
                    Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                    [1.0; 3],
                    light,
                    rect,
                );
            }
        };
        // Vanilla: the moved block is posed at `pos.relative(movement.getOpposite())`, then
        // translated by the progress offset.
        let back = Vec3::new(
            -movement.offset().x as f32,
            -movement.offset().y as f32,
            -movement.offset().z as f32,
        );
        emit_state(self, &mut out, &block_state, back + offset);
        if let Some(head) = head_state {
            // The base sits at the block entity's own position: vanilla's
            // `basePos = pos.relative(movement)` walks back to where `pos` came from
            // (`pos = bePos.relative(movement.getOpposite())`), so `basePos == bePos`,
            // unoffset.
            emit_state(self, &mut out, &head, Vec3::ZERO);
        }
        self.vertices = out;
    }

    /// One painting, following vanilla's `PaintingRenderer`: a `width` x `height` grid of front
    /// quads at z = -1/32 with the variant's sprite, a mirrored back at z = +1/32 and the edge
    /// strips, all turned by `180 - direction.get2DDataValue() * 90`. An unknown variant draws
    /// the back sprite on the front too (never a panic).
    fn emit_painting(
        &mut self,
        e: &EntityInstance,
        p: &PaintingDecoration,
        camera: glam::DVec3,
        sky: u8,
        block: u8,
        assets: &mut Assets,
    ) {
        let base =
            Mat4::from_translation((e.pos - camera).as_vec3()) * painting_transform(p.facing);
        // Paintings use the entity's own light (`PaintingRenderer` draws with `state.lightCoords`).
        let light = (block, sky);
        let width = p.width.max(1) as i32;
        let height = p.height.max(1) as i32;
        let back = self.texture_rect(&Identifier::minecraft("painting/back"), assets);
        let front = p
            .front
            .as_ref()
            .map(|id| self.texture_rect(id, assets))
            .unwrap_or(back);
        let nm = Mat3::from_mat4(base).inverse().transpose();
        let mut out = std::mem::take(&mut self.vertices);
        let (ox, oy) = (-(width as f32) / 2.0, -(height as f32) / 2.0);
        let (du, dv) = (1.0 / width as f32, 1.0 / height as f32);
        const Z: f32 = 0.03125;
        // The edge strips sample the back sprite's top/bottom 1/16 and left/right 1/16.
        let (edge_v, edge_u) = (0.0625f32, 0.0625f32);
        let quad = |out: &mut Vec<EntityVertex>, p: [Vec3; 4], uv: [[f32; 2]; 4], n: Vec3, rect| {
            push_world_quad(out, base, nm, p, uv, n, [1.0; 3], light, rect);
        };
        for sx in 0..width {
            for sy in 0..height {
                let x0 = ox + (sx + 1) as f32;
                let x1 = ox + sx as f32;
                let y0 = oy + (sy + 1) as f32;
                let y1 = oy + sy as f32;
                let fu0 = du * (width - sx) as f32;
                let fu1 = du * (width - (sx + 1)) as f32;
                let fv0 = dv * (height - sy) as f32;
                let fv1 = dv * (height - (sy + 1)) as f32;
                quad(
                    &mut out,
                    [
                        Vec3::new(x0, y1, -Z),
                        Vec3::new(x1, y1, -Z),
                        Vec3::new(x1, y0, -Z),
                        Vec3::new(x0, y0, -Z),
                    ],
                    [[fu1, fv0], [fu0, fv0], [fu0, fv1], [fu1, fv1]],
                    Vec3::new(0.0, 0.0, -1.0),
                    front,
                );
                quad(
                    &mut out,
                    [
                        Vec3::new(x0, y0, Z),
                        Vec3::new(x1, y0, Z),
                        Vec3::new(x1, y1, Z),
                        Vec3::new(x0, y1, Z),
                    ],
                    [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                    Vec3::new(0.0, 0.0, 1.0),
                    back,
                );
                if sy == height - 1 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x0, y0, -Z),
                            Vec3::new(x1, y0, -Z),
                            Vec3::new(x1, y0, Z),
                            Vec3::new(x0, y0, Z),
                        ],
                        [[0.0, 0.0], [1.0, 0.0], [1.0, edge_v], [0.0, edge_v]],
                        Vec3::new(0.0, 1.0, 0.0),
                        back,
                    );
                }
                if sy == 0 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x0, y1, Z),
                            Vec3::new(x1, y1, Z),
                            Vec3::new(x1, y1, -Z),
                            Vec3::new(x0, y1, -Z),
                        ],
                        [[0.0, 0.0], [1.0, 0.0], [1.0, edge_v], [0.0, edge_v]],
                        Vec3::new(0.0, -1.0, 0.0),
                        back,
                    );
                }
                if sx == width - 1 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x0, y0, Z),
                            Vec3::new(x0, y1, Z),
                            Vec3::new(x0, y1, -Z),
                            Vec3::new(x0, y0, -Z),
                        ],
                        [[edge_u, 0.0], [edge_u, 1.0], [0.0, 1.0], [0.0, 0.0]],
                        Vec3::new(-1.0, 0.0, 0.0),
                        back,
                    );
                }
                if sx == 0 {
                    quad(
                        &mut out,
                        [
                            Vec3::new(x1, y0, -Z),
                            Vec3::new(x1, y1, -Z),
                            Vec3::new(x1, y1, Z),
                            Vec3::new(x1, y0, Z),
                        ],
                        [[edge_u, 0.0], [edge_u, 1.0], [0.0, 1.0], [0.0, 0.0]],
                        Vec3::new(1.0, 0.0, 0.0),
                        back,
                    );
                }
            }
        }
        self.vertices = out;
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

    /// One sign's text, following vanilla `AbstractSignRenderer.submitSignText`: both sides,
    /// up to four lines each, centred per line, glowing text full-bright in its dye colour and
    /// non-glowing text in the dark colour at the sign's light. The glyphs walk the default
    /// font like the GUI's text pass and land in the entity atlas (one draw with everything).
    fn emit_sign_text(
        &mut self,
        inst: &crate::blockentity::BlockEntityInstance,
        hanging: bool,
        camera: glam::DVec3,
    ) {
        use crate::blockentity::{
            sign_dark_color, sign_glow_color, sign_line_metrics, sign_text_transform,
        };
        let Some(text) = inst.text.clone() else {
            return;
        };
        let (sky, block) = inst.light;
        let (line_height, _max_width) = sign_line_metrics(hanging);
        // `signMidpoint = 4 * lineHeight / 2`: the four lines are centred on the sign.
        let midpoint = 4.0 * line_height as f32 / 2.0;
        for (front, side) in [(true, &text.front), (false, &text.back)] {
            let m = sign_text_transform(hanging, inst.sign_angle, front, camera, inst.pos);
            let nm = Mat3::from_mat4(m).inverse().transpose();
            let glowing = side.glowing;
            let color = if glowing {
                sign_glow_color(side)
            } else {
                sign_dark_color(side)
            };
            // Vanilla `getLightCoords`: glowing text is full-bright (15728880: block 15,
            // sky 15), the rest use the sign's own light.
            let light = if glowing { (15, 15) } else { (block, sky) };
            self.emit_sign_side(&m, nm, side, color, light, line_height, midpoint);
        }
    }

    /// One side's four lines, centred each (`-width / 2`), at `i * lineHeight - midpoint`.
    #[allow(clippy::too_many_arguments)]
    fn emit_sign_side(
        &mut self,
        m: &Mat4,
        nm: Mat3,
        side: &crate::blockentity::SignSideText,
        color: u32,
        light: (u8, u8),
        line_height: i32,
        midpoint: f32,
    ) {
        let tint = |fmt_color: Option<u32>| {
            let c = fmt_color.unwrap_or(color);
            [
                ((c >> 16) & 0xFF) as f32 / 255.0,
                ((c >> 8) & 0xFF) as f32 / 255.0,
                (c & 0xFF) as f32 / 255.0,
            ]
        };
        let font = self.font.clone();
        let mut out = std::mem::take(&mut self.vertices);
        for (i, line) in side.lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let w = font.width_f32(line);
            let y = i as f32 * line_height as f32 - midpoint;
            let mut cursor = -w / 2.0;
            haste_assets::for_each_formatted(line, Default::default(), |ch, fmt| {
                let glyph = font.glyph(ch);
                let t = tint(fmt.color);
                self.sign_glyph_quad(&mut out, *m, nm, glyph, cursor, y, t, light);
                cursor += glyph.advance + if fmt.bold { glyph.bold_offset } else { 0.0 };
            });
        }
        self.vertices = out;
    }

    /// One glyph quad in text space (x right, y down, z toward the viewer), following the GUI
    /// text pass's geometry (`GuiRenderer::glyph`): bitmap glyphs draw their page rectangle at
    /// `y + top` scaled, unihex glyphs at half scale from their trimmed range. `uv` is
    /// normalized 0..1 within `rect` (the page image), like `push_world_quad` maps it.
    #[allow(clippy::too_many_arguments)]
    fn sign_glyph_quad(
        &mut self,
        out: &mut Vec<EntityVertex>,
        m: Mat4,
        nm: Mat3,
        glyph: haste_assets::Glyph,
        x: f32,
        y: f32,
        tint: [f32; 3],
        light: (u8, u8),
    ) {
        // The drawn size and the source rectangle, normalized within the page image.
        let (rect, draw, src, top) = match glyph.kind {
            haste_assets::GlyphKind::Empty => return,
            // Vanilla's missing-glyph box; drawn only in the GUI pass today.
            haste_assets::GlyphKind::Missing => return,
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
        let (x0, y0) = (x, y + top);
        let (x1, y1) = (x0 + draw[0], y0 + draw[1]);
        let corners = [
            Vec3::new(x0, y0, 0.0),
            Vec3::new(x0, y1, 0.0),
            Vec3::new(x1, y1, 0.0),
            Vec3::new(x1, y0, 0.0),
        ];
        let uv = [
            [src[0], src[1]],
            [src[0], src[1] + src[3]],
            [src[0] + src[2], src[1] + src[3]],
            [src[0] + src[2], src[1]],
        ];
        push_world_quad(
            out,
            m,
            nm,
            corners,
            uv,
            Vec3::new(0.0, 0.0, 1.0),
            tint,
            light,
            rect,
        );
    }
    fn item_model_for(
        &mut self,
        i: haste_core::RegistryIndex,
        assets: &mut Assets,
    ) -> Option<Arc<haste_assets::ItemModel>> {
        if let Some(m) = self.item_models.get(&i.0) {
            return m.clone();
        }
        let m = self
            .item_names
            .get(i.0 as usize)
            .filter(|n| n.path() != "air")
            .and_then(|n| assets.item_model(n).ok())
            .map(Arc::new);
        self.item_models.insert(i.0, m.clone());
        m
    }

    /// The item frame block model quads for a `(glow, map)` combination, resolved from the jar's
    /// `item_frame` / `glow_item_frame` blockstate (`BlockModelResolver.updateForItemFrame`) once
    /// and cached. An unreadable model yields no quads, never a panic.
    fn frame_quads(
        &mut self,
        glow: bool,
        map: bool,
        assets: &mut Assets,
    ) -> Arc<Vec<haste_assets::BakedQuad>> {
        if let Some(q) = self.frame_models.get(&(glow, map)) {
            return q.clone();
        }
        let block = if glow {
            "glow_item_frame"
        } else {
            "item_frame"
        };
        let mut quads = Vec::new();
        if let Ok(def) = assets.blockstate(&Identifier::minecraft(block)) {
            let props = vec![("map".to_string(), map.to_string())];
            for r in def.model_refs_for(&props) {
                if let Ok(id) = Identifier::parse(&r.model) {
                    if let Ok(mut q) = assets.model_quads(&id) {
                        quads.append(&mut q);
                    }
                }
            }
        }
        let arc = Arc::new(quads);
        self.frame_models.insert((glow, map), arc.clone());
        arc
    }

    /// The equipment asset info, loaded once and cached (`None` for an unknown asset, never a
    /// panic and never a re-read).
    fn equipment_info(
        &mut self,
        asset: &Identifier,
        assets: &mut Assets,
    ) -> Option<Arc<haste_assets::EquipmentInfo>> {
        if let Some(info) = self.equipment_infos.get(asset) {
            return info.clone();
        }
        let info = assets.equipment_info(asset).ok().map(Arc::new);
        if info.is_none() {
            tracing::warn!("equipment asset {asset} not found");
        }
        self.equipment_infos.insert(asset.clone(), info.clone());
        info
    }

    /// The empty hand's arm model for the skin's arm width, built once (no per-frame work).
    fn arm_model(&mut self, slim: bool, sleeve: bool) -> Arc<CubeModel> {
        let slot = &mut self.arm_models[usize::from(slim) * 2 + usize::from(sleeve)];
        slot.get_or_insert_with(|| Arc::new(arm_model(slim, sleeve)))
            .clone()
    }

    /// Builds the first-person arm or held item, following vanilla's
    /// `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`: the hurt tilt and walk bob
    /// (`GameRenderer.renderItemInHand`'s pose stack) and the hand's lag behind the camera
    /// turn are applied to the whole hand in view space, then per hand vanilla's own
    /// transform, then the model.
    ///
    /// `prefix` is the pose stack the hand is submitted under (view space stays view space:
    /// x right, y up, -z forward), `view` is the camera's look (yaw, pitch) in degrees and
    /// `partial` the partial tick; the per-tick animation values live in `fp` and are only
    /// lerped here.
    ///
    /// Nothing is allocated per frame: the arm model is cached by `slim` and the vertices go
    /// into the reused hand buffer.
    pub fn prepare_hand(
        &mut self,
        fp: Option<&FirstPerson>,
        prefix: Mat4,
        view: (f32, f32),
        partial: f32,
        assets: &mut Assets,
    ) {
        self.hand.clear();
        let Some(fp) = fp else {
            return;
        };
        // `FirstPerson::light` is (sky, block); the vertex wants (block, sky).
        let light = (fp.light.1, fp.light.0);
        let item = fp.item.and_then(|i| self.item_model_for(i, assets));
        let mut out = std::mem::take(&mut self.hand);
        let root = prefix * hand_root_matrix(fp, view, partial);
        match item {
            Some(model) => {
                let root = root * item_arm_matrix(fp, partial);
                self.emit_item(
                    &mut out,
                    &model,
                    root,
                    light,
                    ItemDisplay::FirstPerson,
                    assets,
                );
            }
            None => {
                let steve = Identifier::minecraft("entity/player/wide/steve");
                let skin = fp.skin.as_ref().unwrap_or(&steve);
                let rect = self.texture_rect(skin, assets);
                let model = self.arm_model(fp.slim, fp.sleeve);
                // Vanilla's `ModelPart` scales its pivot by 1/16 (the model is in pixels), and
                // `AvatarRenderer.renderHand` gives the right arm its 0.1 rad z tilt.
                let m = root * arm_matrix(fp, partial) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
                emit_part_tinted(
                    &mut out,
                    &model,
                    &model.root,
                    &PoseOverrides::default(),
                    m,
                    rect,
                    light,
                    0.0,
                    [1.0; 3],
                );
            }
        }
        self.hand = out;
        if self.hand.len() > self.hand_capacity {
            self.hand_capacity = self.hand.len().next_power_of_two();
            self.hand_buffer = make_vertex_buffer(&self.device, self.hand_capacity);
        }
        if !self.hand.is_empty() {
            self.queue
                .write_buffer(&self.hand_buffer, 0, bytemuck::cast_slice(&self.hand));
        }
    }

    /// Emits an item model's geometry through `root` (model space, 0..1 blocks, through the
    /// part's display transform) into `out`. `display` picks which of vanilla's
    /// `ItemDisplayContext`s the model is drawn through (`firstperson_righthand` for the hand,
    /// `fixed` for item frames, `ground` for dropped items).
    fn emit_item(
        &mut self,
        out: &mut Vec<EntityVertex>,
        model: &haste_assets::ItemModel,
        root: Mat4,
        light: (u8, u8),
        display: ItemDisplay,
        assets: &mut Assets,
    ) {
        for part in &model.parts {
            let (display, left) = match display {
                ItemDisplay::FirstPerson => (&part.firstperson, false),
                ItemDisplay::ThirdPersonRight => (&part.thirdperson, false),
                ItemDisplay::ThirdPersonLeft => (&part.thirdperson, true),
                ItemDisplay::Head => (&part.head, false),
                ItemDisplay::Fixed => (&part.fixed, false),
                ItemDisplay::Ground => (&part.ground, false),
                ItemDisplay::OnShelf => (&part.on_shelf, false),
            };
            let m = root * item_display_matrix(part, display, left);
            let nm = Mat3::from_mat4(m).inverse().transpose();
            let tint = |i: Option<u32>| -> [f32; 3] {
                i.and_then(|i| part.tints.get(i as usize))
                    .map(|c| {
                        let f =
                            |s: u32| crate::sky::linear([((c >> s) & 0xFF) as f32 / 255.0; 3])[0];
                        [f(16), f(8), f(0)]
                    })
                    .unwrap_or([1.0; 3])
            };
            match &part.geometry {
                haste_assets::ItemGeometry::None => {}
                haste_assets::ItemGeometry::Generated { layers } => {
                    // The extruded sprite: front face, mirrored back face and the 1/16-deep
                    // sides at its opaque edges (vanilla `ItemModelGenerator`).
                    for (i, layer) in layers.iter().enumerate() {
                        let rect = self.texture_rect(&parse_tex(&layer.texture), assets);
                        let t = tint(Some(i as u32));
                        for q in &layer.quads {
                            let o = q.direction.offset();
                            push_world_quad(
                                out,
                                m,
                                nm,
                                q.positions.map(Vec3::from),
                                q.uvs,
                                Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                                t,
                                light,
                                rect,
                            );
                        }
                    }
                }
                haste_assets::ItemGeometry::Elements(quads) => {
                    for q in quads {
                        let rect = self.texture_rect(&parse_tex(&q.texture), assets);
                        let o = q.direction.offset();
                        push_world_quad(
                            out,
                            m,
                            nm,
                            q.positions.map(Vec3::from),
                            q.uvs,
                            Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                            tint(q.tint_index),
                            light,
                            rect,
                        );
                    }
                }
            }
        }
    }

    /// Draws the first-person arm or item (its own pass, depth cleared).
    pub fn draw_hand<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.hand.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.hand_buffer.slice(..));
        pass.draw(0..self.hand.len() as u32, 0..1);
        1
    }

    pub fn has_hand(&self) -> bool {
        !self.hand.is_empty()
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

fn make_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("entity-vertices"),
        size: (capacity * std::mem::size_of::<EntityVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The arm model of a first-person hand: the player model's `right_arm` with its
/// `right_sleeve` child (vanilla `HumanoidModel.createMesh` + `PlayerModel.createMesh`), in
/// model pixels. `slim` picks the 3 px arm. Built once and cached, so the hand pass allocates
/// nothing per frame.
pub(crate) fn arm_model(slim: bool, sleeve: bool) -> CubeModel {
    // Wide: (-3, -2, -2) + (4, 12, 4); slim: (-2, -2, -2) + (3, 12, 4).
    let origin_x = if slim { -2.0 } else { -3.0 };
    let w = if slim { 3.0 } else { 4.0 };
    let cube = |uv: [u32; 2], inflate: f32| haste_core::model::Cube {
        origin: Vec3::new(origin_x, -2.0, -2.0),
        size: Vec3::new(w, 12.0, 4.0),
        inflate,
        uv,
        mirror: false,
    };
    CubeModel {
        texture: Identifier::minecraft("entity/player/wide/steve"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            // `PartPose.offset(-5, 2, 0)` of the model's right_arm.
            name: "right_arm".into(),
            id: parts::RIGHT_ARM,
            pivot: Vec3::new(-5.0, 2.0, 0.0),
            // `AvatarRenderer.renderHand`: `rightArm.zRot = 0.1`.
            rotation: Vec3::new(0.0, 0.0, 0.1),
            visible: true,
            cubes: vec![cube([40, 16], 0.0)],
            // `AvatarRenderer.renderHand`: the sleeve shows with the right-sleeve part.
            children: vec![ModelPart {
                name: "right_sleeve".into(),
                id: parts::RIGHT_SLEEVE,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: sleeve,
                cubes: vec![cube([40, 32], 0.25)],
                children: vec![],
            }],
        },
    }
}

/// `ItemInHandLayer.submitArmWithItem`'s transform after `ArmedModel.translateToHand`: rotate X
/// -90 degrees, rotate Y 180 degrees, then the hand offset (`(±1, 2, -10)/16` for adults; a
/// left-hand item is offset to -1). The item then draws through `thirdperson_righthand`.
pub(crate) fn held_item_layer_matrix(left: bool) -> Mat4 {
    Mat4::from_rotation_x((-90.0f32).to_radians())
        * Mat4::from_rotation_y(180.0f32.to_radians())
        * Mat4::from_translation(Vec3::new(
            if left { -1.0 } else { 1.0 } / 16.0,
            2.0 / 16.0,
            -10.0 / 16.0,
        ))
}

/// `CustomHeadLayer.translateToHead` (the static one, after the head part's own transform):
/// translate `(0, -0.25, 0)`, rotate Y 180 degrees, scale `(0.625, -0.625, -0.625)`. The item
/// then draws through `display.head`.
pub(crate) fn custom_head_layer_matrix() -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
        * Mat4::from_rotation_y(180.0f32.to_radians())
        * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
}

/// An ARGB colour (armour dye / undyed fallback) as a linear-RGB vertex multiplier, the same
/// conversion the GUI item tints use (`sky::linear`).
pub(crate) fn argb_linear(c: u32) -> [f32; 3] {
    let f = |s: u32| crate::sky::linear([((c >> s) & 0xFF) as f32 / 255.0; 3])[0];
    [f(16), f(8), f(0)]
}

/// A held item's model matrix: its `display.<context>` transform and composite
/// `transformation`, then the recentring vanilla's `ItemTransform.apply` ends with
/// (the model's unit cube is drawn around the origin). `left` applies vanilla's left-hand fix
/// (`ItemTransform.apply(true, ...)`: translation.x and rotation.y/z negated).
pub(crate) fn item_display_matrix(
    part: &haste_assets::ItemModelPart,
    display: &haste_assets::DisplayTransform,
    left: bool,
) -> Mat4 {
    let display = if left {
        haste_assets::DisplayTransform {
            rotation: [
                display.rotation[0],
                -display.rotation[1],
                -display.rotation[2],
            ],
            translation: [
                -display.translation[0],
                display.translation[1],
                display.translation[2],
            ],
            scale: display.scale,
        }
    } else {
        *display
    };
    crate::gui::display_matrix(&display)
        * crate::gui::part_matrix(&part.transformation)
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// `ItemFrameRenderer.submit`'s frame transform: translate to `direction * 0.46875`, then the
/// rotations that turn the block model to face `direction` (horizontal: yaw `180 - toYRot`;
/// vertical: pitch `-90 * axisStep` then yaw 180).
pub(crate) fn item_frame_transform(facing: haste_core::Direction) -> Mat4 {
    let (fx, fy, fz) = (
        facing.step_x() as f32,
        facing.step_y() as f32,
        facing.step_z() as f32,
    );
    let (x_rot, y_rot) = if facing.is_horizontal() {
        (0.0, 180.0 - facing.to_y_rot())
    } else {
        (-90.0 * facing.axis_direction_step() as f32, 180.0)
    };
    Mat4::from_translation(Vec3::new(fx, fy, fz) * 0.46875)
        * Mat4::from_rotation_x(x_rot.to_radians())
        * Mat4::from_rotation_y(y_rot.to_radians())
}

/// `ItemFrameRenderer.submit`'s item transform: forward to the frame's face, `rotation * 45`
/// degrees around Z, then scale 0.5.
pub(crate) fn item_frame_item_transform(rotation: u8) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, 0.0, 0.4375))
        * Mat4::from_rotation_z((rotation as f32 * 45.0).to_radians())
        * Mat4::from_scale(Vec3::splat(0.5))
}

/// `PaintingRenderer.submit`'s rotation: yaw `180 - direction.get2DDataValue() * 90`.
pub(crate) fn painting_transform(facing: haste_core::Direction) -> Mat4 {
    Mat4::from_rotation_y((180.0 - facing.to_2d_data_value() as f32 * 90.0).to_radians())
}

/// `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`'s outer transforms, in view
/// space: the hand's lag behind the camera turn (`(viewXRot - xBob) * 0.1`,
/// `(viewYRot - yBob) * 0.1`). The walk bob is NOT here: vanilla applies `bobHurt` and
/// `bobView` to the whole hand pose stack in `GameRenderer.renderItemInHand` (the renderer
/// passes them as the prefix matrix), once, like the world's projection carries them.
pub(crate) fn hand_root_matrix(fp: &FirstPerson, view: (f32, f32), partial: f32) -> Mat4 {
    Mat4::from_rotation_x(((view.1 - fp.x_bob.at(partial)) * 0.1).to_radians())
        * Mat4::from_rotation_y(((view.0 - fp.y_bob.at(partial)) * 0.1).to_radians())
}

/// `FirstPersonHandsAndItemsRenderer.renderPlayerArm` for the empty hand: the arm's swing
/// translation and rotations, ending at the arm model's own transform. `inverse_arm_height` is
/// 0 when the item is fully held and 1 while it switches.
pub(crate) fn arm_matrix(fp: &FirstPerson, partial: f32) -> Mat4 {
    const ARM_SWING_X_POS_SCALE: f32 = -0.3;
    const ARM_SWING_Y_POS_SCALE: f32 = 0.4;
    const ARM_SWING_Z_POS_SCALE: f32 = -0.4;
    const ARM_SWING_Y_ROT_AMOUNT: f32 = 70.0;
    const ARM_SWING_Z_ROT_AMOUNT: f32 = -20.0;
    let attack = fp.attack.at(partial);
    let inverse_arm_height = 1.0 - fp.equip.at(partial);
    // Only the right hand is rendered (a left main hand is not modelled yet), so the arm's
    // `invert` factor is 1 throughout.
    let sqrt_attack = attack.max(0.0).sqrt();
    let x = ARM_SWING_X_POS_SCALE * (sqrt_attack * std::f32::consts::PI).sin();
    let y = ARM_SWING_Y_POS_SCALE * (sqrt_attack * std::f32::consts::PI * 2.0).sin();
    let z = ARM_SWING_Z_POS_SCALE * (attack * std::f32::consts::PI).sin();
    let mut m = Mat4::from_translation(Vec3::new(
        // `0.64000005` and `-0.71999997` are vanilla's own literals.
        x + 0.64000005,
        y - 0.6 + inverse_arm_height * -0.6,
        z - 0.71999997,
    ));
    m *= Mat4::from_rotation_y(45.0f32.to_radians());
    let z_swing_rotation = (attack * attack * std::f32::consts::PI).sin();
    let y_swing_rotation = (sqrt_attack * std::f32::consts::PI).sin();
    m *= Mat4::from_rotation_y((y_swing_rotation * ARM_SWING_Y_ROT_AMOUNT).to_radians());
    m *= Mat4::from_rotation_z((z_swing_rotation * ARM_SWING_Z_ROT_AMOUNT).to_radians());
    m *= Mat4::from_translation(Vec3::new(-1.0, 3.6, 3.5));
    m *= Mat4::from_rotation_z(120.0f32.to_radians());
    m *= Mat4::from_rotation_x(200.0f32.to_radians());
    m *= Mat4::from_rotation_y((-135.0f32).to_radians());
    m * Mat4::from_translation(Vec3::new(5.6, 0.0, 0.0))
}

/// `FirstPersonHandsAndItemsRenderer.applyItemArmTransform` + `swingArm` for a held item: the
/// item's place in view space (`0.56, -0.52 + inverseArmHeight * -0.6, -0.72`) with the swing's
/// translation and `applyItemArmAttackTransform`'s rotations.
pub(crate) fn item_arm_matrix(fp: &FirstPerson, partial: f32) -> Mat4 {
    const ITEM_SWING_X_POS_SCALE: f32 = -0.4;
    const ITEM_SWING_Y_POS_SCALE: f32 = 0.2;
    const ITEM_SWING_Z_POS_SCALE: f32 = -0.2;
    let attack = fp.attack.at(partial);
    let inverse_arm_height = 1.0 - fp.equip.at(partial);
    let mut m = Mat4::from_translation(Vec3::new(0.56, -0.52 + inverse_arm_height * -0.6, -0.72));
    // `swingArm`.
    let sqrt_attack = attack.max(0.0).sqrt();
    m *= Mat4::from_translation(Vec3::new(
        ITEM_SWING_X_POS_SCALE * (sqrt_attack * std::f32::consts::PI).sin(),
        ITEM_SWING_Y_POS_SCALE * (sqrt_attack * std::f32::consts::PI * 2.0).sin(),
        ITEM_SWING_Z_POS_SCALE * (attack * std::f32::consts::PI).sin(),
    ));
    // `applyItemArmAttackTransform`.
    let y_swing_rotation = (attack * attack * std::f32::consts::PI).sin();
    m *= Mat4::from_rotation_y((45.0 + y_swing_rotation * -20.0).to_radians());
    let xz_swing_rotation = (sqrt_attack * std::f32::consts::PI).sin();
    m *= Mat4::from_rotation_z((xz_swing_rotation * -20.0).to_radians());
    m *= Mat4::from_rotation_x((xz_swing_rotation * -80.0).to_radians());
    m * Mat4::from_rotation_y((-45.0f32).to_radians())
}

fn parse_tex(id: &str) -> Identifier {
    Identifier::parse(id).unwrap_or_else(|_| Identifier::minecraft("missingno"))
}

/// One textured quad (`uv` 0..1 over `rect`) through `m`, wound counter-clockwise seen from
/// the side its normal points to, shaded like entities.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_world_quad(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    normal_matrix: Mat3,
    p: [Vec3; 4],
    uv: [[f32; 2]; 4],
    n: Vec3,
    tint: [f32; 3],
    light: (u8, u8),
    rect: [u32; 4],
) {
    let p = p.map(|v| m.transform_point3(v));
    let nw = (normal_matrix * n).normalize_or_zero();
    let s = entity_shade(nw);
    let color = [tint[0] * s, tint[1] * s, tint[2] * s, 1.0];
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let ccw = (p[1] - p[0]).cross(p[2] - p[0]).dot(nw) >= 0.0;
    let order: [usize; 6] = if ccw {
        [0, 1, 2, 0, 2, 3]
    } else {
        [0, 2, 1, 0, 3, 2]
    };
    let r16 = rect.map(|v| v as u16);
    for i in order {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            light: [light.0 as u16, light.1 as u16, 0, 0],
        });
    }
}

/// World-space entity shading: two directional lights at 0.6 and 0.4 ambient (the same
/// lighting model as GUI items, with the light directions in world space).
pub(crate) fn entity_shade(n: Vec3) -> f32 {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    ((n.dot(l0).max(0.0) + n.dot(l1).max(0.0)) * 0.6 + 0.4).min(1.0)
}

/// One shadow quad (four vertices, two triangles) with a flat white colour at `alpha`. The
/// shadow texture is sampled at `uv` (0..1) inside `rect` in the entity atlas; the light is
/// full-bright (vanilla's `15728880`), so `entity.wgsl` leaves the texture colour unchanged.
fn push_shadow_quad(
    out: &mut Vec<EntityVertex>,
    p: [Vec3; 4],
    uv: [[f32; 2]; 4],
    alpha: f32,
    rect: [u32; 4],
) {
    let color = [1.0, 1.0, 1.0, alpha];
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let r16 = rect.map(|v| v as u16);
    for i in [0usize, 1, 2, 0, 2, 3] {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            light: [15, 15, 0, 0],
        });
    }
}

/// `Level.getMaxLocalRawBrightness(pos)`: `max(blockLight, skyLight - skyDarken)`
/// (`LevelLightEngine.getRawBrightness`). `sky_darken` is `Level.getSkyDarken`.
pub(crate) fn max_local_raw_brightness(
    world: &dyn BlockView,
    pos: BlockPos,
    sky_darken: i32,
) -> i32 {
    let block = world.block_light(pos) as i32;
    let sky = world.sky_light(pos) as i32 - sky_darken;
    block.max(sky)
}

/// `EntityRenderer.extractNameTags` + `LivingEntityRenderer.shouldShowName`: a name tag shows
/// within 64 blocks, and only within 32 while crouching (`isDiscrete`, distanceSq >= 1024).
pub fn name_tag_visible(dist_sq: f64, is_discrete: bool) -> bool {
    dist_sq < 64.0 * 64.0 && !(is_discrete && dist_sq >= 32.0 * 32.0)
}

/// `Lightmap.getBrightness(dimensionType, level)`: the curved light curve
/// `v / (4 - 3v)` with the dimension's ambient light floor, `lerp(ambient, curved, 1)`.
pub fn lightmap_brightness(ambient: f32, level: i32) -> f32 {
    let v = level as f32 / 15.0;
    let curved = v / (4.0 - 3.0 * v);
    curved + ambient * (1.0 - curved)
}

/// `EntityRenderer.extractShadowPiece`'s per-piece alpha:
/// `clamp(powerAtDepth * 0.5 * brightness, 0, 1)` with
/// `powerAtDepth = pow - (entity.y - pos.y) * 0.5`.
pub fn shadow_piece_alpha(power_at_depth: f32, brightness: f32) -> f32 {
    (power_at_depth * 0.5 * brightness).clamp(0.0, 1.0)
}

/// Hurt flash red overlay strength. Vanilla overlays red at about 30 % (`hurt` in
/// `EntityRenderState` mixes `0.3 * hurt` towards (1, 0, 0)); we use the same 0.3.
pub(crate) const HURT_RED_STRENGTH: f32 = 0.3;

/// The per-channel colour of a hurt entity: shade and light with red mixed in at vanilla's
/// about-30 % overlay strength. This is the CPU-side vertex colour the shader multiplies the
/// texture by (linear RGB), so the tint shows without shader changes.
pub(crate) fn hurt_color(s: f32, hurt: f32) -> [f32; 3] {
    let r = (s + HURT_RED_STRENGTH * hurt).min(1.0);
    let g = s * (1.0 - HURT_RED_STRENGTH * hurt);
    [r, g, g]
}

/// Highest point of a model above its feet, in model units (y is down in model space).
pub(crate) fn model_top(part: &ModelPart, parent_y: f32) -> f32 {
    let y = parent_y + part.pivot.y;
    let own = part
        .cubes
        .iter()
        .map(|c| -(y + c.origin.y - c.inflate))
        .fold(0.0f32, f32::max);
    part.children
        .iter()
        .map(|c| model_top(c, y))
        .fold(own, f32::max)
}

/// A model's render size and camera-relative half-extents in blocks, for vanilla's
/// `Entity.shouldRenderAtSqrDistance` (`boundingBox.getSize()`) and the frustum test
/// (`getBoundingBoxForCulling`, the model's static bounds: like vanilla's AABB it does
/// not track animation). Model space is y-down, 1/16 block per unit; `emit_part` scales
/// by 1/16 with x/y mirrored, so extents are symmetric and only the scale matters.
pub(crate) fn model_extents(part: &ModelPart) -> (f32, f32) {
    fn walk(part: &ModelPart, ox: f32, oy: f32, oz: f32, lo: &mut Vec3, hi: &mut Vec3) {
        let (px, py, pz) = (ox + part.pivot.x, oy + part.pivot.y, oz + part.pivot.z);
        for c in &part.cubes {
            let (x0, y0, z0) = (px + c.origin.x, py + c.origin.y, pz + c.origin.z);
            let (x1, y1, z1) = (x0 + c.size.x, y0 + c.size.y, z0 + c.size.z);
            let i = c.inflate;
            *lo = lo.min(Vec3::new(x0 - i, y0 - i, z0 - i));
            *hi = hi.max(Vec3::new(x1 + i, y1 + i, z1 + i));
        }
        for child in &part.children {
            walk(child, px, py, pz, lo, hi);
        }
    }
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    walk(part, 0.0, 0.0, 0.0, &mut lo, &mut hi);
    let (w, h, d) = (
        (hi.x - lo.x).abs(),
        (hi.y - lo.y).abs(),
        (hi.z - lo.z).abs(),
    );
    // `AABB.getSize()` is the diagonal's length; model space is 1/16 block, so / 16.
    let size = ((w * w + h * h + d * d).sqrt() / 16.0).max(1.0);
    let half = Vec3::new(w, h, d) / 16.0 / 2.0;
    (size, half.length())
}

/// Conservative AABB-in-frustum test: the sphere around the entity (centre `rel` in
/// camera-relative blocks, radius `r`) against the 6 planes (Gribb/Hartmann, the same
/// plane form the terrain cull uses). The sphere contains the model's extents.
pub(crate) fn aabb_in_frustum(planes: &[glam::Vec4; 6], rel: glam::DVec3, r: f32) -> bool {
    let c = rel.as_vec3();
    planes
        .iter()
        .all(|p| p.x * c.x + p.y * c.y + p.z * c.z + p.w >= -r)
}

pub(crate) fn pose_rotation(pose: &PoseOverrides, id: PartId) -> Option<Vec3> {
    pose.rotation_of(id)
}

/// The accumulated model matrix of a named part, the same transform `emit_part` gives it
/// (`parent * translate(pivot) * R(z) R(y) R(x)`, pose overrides added to the base rotation), so
/// a held item attaches to the animated arm (vanilla `ArmedModel.translateToHand`). `None` when
/// the model has no such part.
pub(crate) fn part_matrix(
    part: &ModelPart,
    pose: &PoseOverrides,
    parent: Mat4,
    id: PartId,
) -> Option<Mat4> {
    let rot = part.rotation + pose_rotation(pose, part.id).unwrap_or(Vec3::ZERO);
    let m = parent
        * Mat4::from_translation(part.pivot)
        * Mat4::from_rotation_z(rot.z)
        * Mat4::from_rotation_y(rot.y)
        * Mat4::from_rotation_x(rot.x);
    if part.id == id {
        return Some(m);
    }
    part.children
        .iter()
        .find_map(|c| part_matrix(c, pose, m, id))
}

/// Child pivots are relative to the parent (vanilla `PartPose`); rotation is Z * Y * X around
/// the pivot, pose overrides add to the base rotation. `hurt` (0..1) tints the part red;
/// `tint` multiplies the colour per channel (dyed overlay layers).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_part(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    part: &ModelPart,
    pose: &PoseOverrides,
    parent: Mat4,
    rect: [u32; 4],
    light: (u8, u8),
    hurt: f32,
) {
    emit_part_tinted(out, model, part, pose, parent, rect, light, hurt, [1.0; 3]);
}

/// [`emit_part`] with a colour multiplier (vanilla's `submitModel` `color` parameter): armour
/// dye. The tint multiplies the shade/hurt colour.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_part_tinted(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    part: &ModelPart,
    pose: &PoseOverrides,
    parent: Mat4,
    rect: [u32; 4],
    light: (u8, u8),
    hurt: f32,
    tint: [f32; 3],
) {
    if !part.visible || pose.is_hidden(part.id) {
        return;
    }
    let rot = part.rotation + pose_rotation(pose, part.id).unwrap_or(Vec3::ZERO);
    let m = parent
        * Mat4::from_translation(part.pivot)
        * Mat4::from_rotation_z(rot.z)
        * Mat4::from_rotation_y(rot.y)
        * Mat4::from_rotation_x(rot.x);
    let normal_matrix = Mat3::from_mat4(m).inverse().transpose();
    let kx = rect[2] as f32 / model.tex_width.max(1) as f32;
    let ky = rect[3] as f32 / model.tex_height.max(1) as f32;
    let r16 = rect.map(|v| v as u16);
    for c in &part.cubes {
        let lo = c.origin - Vec3::splat(c.inflate);
        let hi = c.origin + c.size + Vec3::splat(c.inflate);
        let (u, v) = (c.uv[0] as f32, c.uv[1] as f32);
        let (w, h, d) = (c.size.x.floor(), c.size.y.floor(), c.size.z.floor());
        // Faces: corners (model space) in order top-left, bottom-left, bottom-right, top-right
        // as seen on the texture, the texture rectangle (u0, v0, u1, v1) and outward normal.
        // Model space: y down, the model's front is -z, its right side is -x.
        let (x0, y0, z0) = (lo.x, lo.y, lo.z);
        let (x1, y1, z1) = (hi.x, hi.y, hi.z);
        let mut faces: [([Vec3; 4], [f32; 4], Vec3); 6] = [
            // north (front): u grows with x, v with y
            (
                [
                    Vec3::new(x0, y0, z0),
                    Vec3::new(x0, y1, z0),
                    Vec3::new(x1, y1, z0),
                    Vec3::new(x1, y0, z0),
                ],
                [u + d, v + d, u + d + w, v + d + h],
                Vec3::NEG_Z,
            ),
            // south (back): u grows toward -x
            (
                [
                    Vec3::new(x1, y0, z1),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x0, y0, z1),
                ],
                [u + 2.0 * d + w, v + d, u + 2.0 * (d + w), v + d + h],
                Vec3::Z,
            ),
            // west (-x, the model's right side): u grows toward the front
            (
                [
                    Vec3::new(x0, y0, z1),
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x0, y1, z0),
                    Vec3::new(x0, y0, z0),
                ],
                [u, v + d, u + d, v + d + h],
                Vec3::NEG_X,
            ),
            // east (+x, the left side): u grows toward the back
            (
                [
                    Vec3::new(x1, y0, z0),
                    Vec3::new(x1, y1, z0),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x1, y0, z1),
                ],
                [u + d + w, v + d, u + 2.0 * d + w, v + d + h],
                Vec3::X,
            ),
            // up (y0 is the top): v grows toward the front
            (
                [
                    Vec3::new(x0, y0, z1),
                    Vec3::new(x0, y0, z0),
                    Vec3::new(x1, y0, z0),
                    Vec3::new(x1, y0, z1),
                ],
                [u + d, v, u + d + w, v + d],
                Vec3::NEG_Y,
            ),
            // down
            (
                [
                    Vec3::new(x0, y1, z0),
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x1, y1, z0),
                ],
                [u + d + w, v, u + d + 2.0 * w, v + d],
                Vec3::Y,
            ),
        ];
        if c.mirror {
            // Mirrored cubes swap the side textures and flip every face horizontally.
            let west = faces[2].1;
            faces[2].1 = faces[3].1;
            faces[3].1 = west;
            for f in &mut faces {
                f.1.swap(0, 2);
            }
        }
        for (corners, uvr, n) in faces {
            let p = corners.map(|c| m.transform_point3(c));
            let nw = (normal_matrix * n).normalize_or_zero();
            let s = entity_shade(nw);
            let [r, g, b] = hurt_color(s, hurt);
            let color = [r * tint[0], g * tint[1], b * tint[2], 1.0];
            let uv = [
                [uvr[0], uvr[1]],
                [uvr[0], uvr[3]],
                [uvr[2], uvr[3]],
                [uvr[2], uvr[1]],
            ]
            .map(|t| [rect[0] as f32 + t[0] * kx, rect[1] as f32 + t[1] * ky]);
            // Wind counter-clockwise seen from outside, whatever the transform did.
            let ccw = (p[1] - p[0]).cross(p[2] - p[0]).dot(nw) >= 0.0;
            let order: [usize; 6] = if ccw {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            for i in order {
                out.push(EntityVertex {
                    pos: p[i].to_array(),
                    uv: uv[i],
                    color,
                    rect: r16,
                    light: [light.0 as u16, light.1 as u16, 0, 0],
                });
            }
        }
    }
    for child in &part.children {
        emit_part_tinted(out, model, child, pose, m, rect, light, hurt, tint);
    }
}

/// The model's bounding box in the space `emit_item` draws it through the given display
/// transform (block units, after the -0.5 centring `ItemTransform.apply` ends with): the
/// transformed element or extruded-sprite positions. Vanilla's
/// `ItemStackRenderState.getModelBoundingBox` (which drives `ItemEntityRenderer.submit`'s
/// lift, the stack-copy layout and `ShelfRenderer`'s vertical offset) computes exactly this;
/// a model with no geometry gets vanilla's empty box (a point at the origin).
fn item_model_bbox(model: &haste_assets::ItemModel, display: &ItemDisplay) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    let mut see = |p: Vec3| {
        min = min.min(p);
        max = max.max(p);
        any = true;
    };
    for part in &model.parts {
        // The composition `item_display_matrix` applies: the display transform, the model's
        // composite transformation, then the -0.5 recentring (no left-hand fix here: block
        // entities never draw an item in the left hand).
        let display = match display {
            ItemDisplay::FirstPerson => &part.firstperson,
            ItemDisplay::ThirdPersonRight | ItemDisplay::ThirdPersonLeft => &part.thirdperson,
            ItemDisplay::Head => &part.head,
            ItemDisplay::Fixed => &part.fixed,
            ItemDisplay::Ground => &part.ground,
            ItemDisplay::OnShelf => &part.on_shelf,
        };
        let m = crate::gui::display_matrix(display)
            * crate::gui::part_matrix(&part.transformation)
            * Mat4::from_translation(Vec3::splat(-0.5));
        match &part.geometry {
            haste_assets::ItemGeometry::None => {}
            haste_assets::ItemGeometry::Generated { layers } => {
                for layer in layers {
                    for q in &layer.quads {
                        for p in q.positions {
                            see(m.transform_point3(Vec3::from(p)));
                        }
                    }
                }
            }
            haste_assets::ItemGeometry::Elements(quads) => {
                for q in quads {
                    for p in q.positions {
                        see(m.transform_point3(Vec3::from(p)));
                    }
                }
            }
        }
    }
    if any {
        (min, max)
    } else {
        (Vec3::ZERO, Vec3::ZERO)
    }
}

/// The humanoid armour model for one slot (vanilla `HumanoidModel.createArmorMeshSet` +
/// `ArmorModelSet`, the 64x32 armour texture layout). Built once per slot; the renderer draws it
/// with the equipment asset's layer texture and the entity's own pose, so armour animates with
/// the body. The outer deformation is 1.0, the inner (leggings) 0.5, and leg cubes 0.1 less.
pub(crate) fn humanoid_armor(slot: haste_core::model::EquipmentSlot) -> Arc<CubeModel> {
    use haste_core::model::EquipmentSlot;
    static MODELS: std::sync::OnceLock<[Arc<CubeModel>; 4]> = std::sync::OnceLock::new();
    let models = MODELS.get_or_init(|| {
        [
            EquipmentSlot::Head,
            EquipmentSlot::Chest,
            EquipmentSlot::Legs,
            EquipmentSlot::Feet,
        ]
        .map(build_armor_model)
    });
    let i = match slot {
        EquipmentSlot::Head => 0,
        EquipmentSlot::Chest => 1,
        EquipmentSlot::Legs => 2,
        EquipmentSlot::Feet => 3,
        _ => 1,
    };
    models[i].clone()
}

/// The armour model for one slot. `head`/`hat` (outer), `body`/arms (outer), `body`+legs
/// (inner) and the legs (outer), per `HumanoidModel.ADULT_ARMOR_PARTS_PER_SLOT`.
fn build_armor_model(slot: haste_core::model::EquipmentSlot) -> Arc<CubeModel> {
    use haste_core::model::EquipmentSlot;
    use haste_core::model::{Cube, ModelPart};
    let outer = 1.0f32;
    let inner = 0.5f32;
    let cube = |origin: [f32; 3], size: [f32; 3], uv: [u32; 2], inflate: f32, mirror: bool| Cube {
        origin: origin.into(),
        size: size.into(),
        inflate,
        uv,
        mirror,
    };
    let part =
        |name: &str, pivot: [f32; 3], cubes: Vec<Cube>, children: Vec<ModelPart>| ModelPart {
            name: name.to_owned(),
            id: parts::intern(name),
            pivot: pivot.into(),
            rotation: Vec3::ZERO,
            visible: true,
            cubes,
            children,
        };
    let head = || {
        part(
            "head",
            [0.0, -24.0, 0.0],
            vec![cube(
                [-4.0, -8.0, -4.0],
                [8.0, 8.0, 8.0],
                [0, 0],
                outer,
                false,
            )],
            vec![part(
                "hat",
                [0.0, 0.0, 0.0],
                vec![cube(
                    [-4.0, -8.0, -4.0],
                    [8.0, 8.0, 8.0],
                    [32, 0],
                    outer + 0.5,
                    false,
                )],
                Vec::new(),
            )],
        )
    };
    let body = |inflate: f32| {
        part(
            "body",
            [0.0, -24.0, 0.0],
            vec![cube(
                [-4.0, 0.0, -2.0],
                [8.0, 12.0, 4.0],
                [16, 16],
                inflate,
                false,
            )],
            Vec::new(),
        )
    };
    let arms = || {
        vec![
            part(
                "right_arm",
                [-5.0, -22.0, 0.0],
                vec![cube(
                    [-3.0, -2.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [40, 16],
                    outer,
                    false,
                )],
                Vec::new(),
            ),
            part(
                "left_arm",
                [5.0, -22.0, 0.0],
                vec![cube(
                    [-1.0, -2.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [40, 16],
                    outer,
                    true,
                )],
                Vec::new(),
            ),
        ]
    };
    let legs = |inflate: f32| {
        vec![
            part(
                "right_leg",
                [-1.9, -12.0, 0.0],
                vec![cube(
                    [-2.0, 0.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [0, 16],
                    inflate,
                    false,
                )],
                Vec::new(),
            ),
            part(
                "left_leg",
                [1.9, -12.0, 0.0],
                vec![cube(
                    [-2.0, 0.0, -2.0],
                    [4.0, 12.0, 4.0],
                    [0, 16],
                    inflate,
                    true,
                )],
                Vec::new(),
            ),
        ]
    };
    let mut children = Vec::new();
    match slot {
        EquipmentSlot::Head => children.push(head()),
        EquipmentSlot::Chest => {
            children.push(body(outer));
            children.extend(arms());
        }
        EquipmentSlot::Legs => {
            children.push(body(inner));
            children.extend(legs(inner - 0.1));
        }
        EquipmentSlot::Feet => children.extend(legs(outer - 0.1)),
        _ => {}
    }
    Arc::new(CubeModel {
        texture: Identifier::new("haste", "entity/armor"),
        tex_width: 64,
        tex_height: 32,
        root: part("root", [0.0, 0.0, 0.0], Vec::new(), children),
    })
}

/// The elytra model (vanilla `ElytraModel.createLayer`): a left and a right wing, 64x32 texture.
/// In Haste's model space (y down, the head base at y = -24) the wings hang from the shoulders.
/// Built once.
pub(crate) fn elytra_model() -> Arc<CubeModel> {
    static MODEL: std::sync::OnceLock<Arc<CubeModel>> = std::sync::OnceLock::new();
    MODEL
        .get_or_init(|| {
            use haste_core::model::{Cube, ModelPart};
            let cube = |origin: [f32; 3], mirror: bool| Cube {
                origin: origin.into(),
                size: Vec3::new(10.0, 20.0, 2.0),
                inflate: 1.0,
                uv: [22, 0],
                mirror,
            };
            let wing = |name: &str, pivot: [f32; 3], rotation: [f32; 3], mirror: bool| ModelPart {
                name: name.to_owned(),
                id: parts::intern(name),
                pivot: pivot.into(),
                rotation: rotation.into(),
                visible: true,
                cubes: vec![cube(
                    if mirror {
                        [0.0, 0.0, 0.0]
                    } else {
                        [-10.0, 0.0, 0.0]
                    },
                    mirror,
                )],
                children: vec![],
            };
            let pi = std::f32::consts::PI;
            Arc::new(CubeModel {
                texture: Identifier::minecraft("entity/elytra"),
                tex_width: 64,
                tex_height: 32,
                root: ModelPart {
                    name: "root".into(),
                    id: parts::ROOT,
                    pivot: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![],
                    children: vec![
                        wing(
                            "left_wing",
                            [5.0, -24.0, 0.0],
                            [pi / 12.0, 0.0, -pi / 12.0],
                            false,
                        ),
                        wing(
                            "right_wing",
                            [-5.0, -24.0, 0.0],
                            [pi / 12.0, 0.0, pi / 12.0],
                            true,
                        ),
                    ],
                },
            })
        })
        .clone()
}

/// The missing item model vanilla's `ModelBakery.MissingModels` bakes (`MissingCuboidModel`):/// a unit cube with the missingno texture and no display transforms, so it draws centred on
/// the origin like any `NO_TRANSFORM` item. Built once; unknown items are rare (modded ids).
fn missing_item_model() -> std::sync::Arc<haste_assets::ItemModel> {
    static MISSING: std::sync::OnceLock<std::sync::Arc<haste_assets::ItemModel>> =
        std::sync::OnceLock::new();
    MISSING
        .get_or_init(|| {
            let quads = [
                haste_assets::Direction::Down,
                haste_assets::Direction::Up,
                haste_assets::Direction::North,
                haste_assets::Direction::South,
                haste_assets::Direction::West,
                haste_assets::Direction::East,
            ]
            .into_iter()
            .map(|d| {
                let mut q =
                    haste_assets::BakedQuad::from_element(d, [0.0; 3], [1.0; 3], None, None);
                q.texture = "haste:missing".into();
                q
            })
            .collect();
            std::sync::Arc::new(haste_assets::ItemModel {
                parts: vec![haste_assets::ItemModelPart {
                    geometry: haste_assets::ItemGeometry::Elements(quads),
                    tints: vec![],
                    gui: haste_assets::DisplayTransform::IDENTITY,
                    firstperson: haste_assets::DisplayTransform::IDENTITY,
                    thirdperson: haste_assets::DisplayTransform::IDENTITY,
                    head: haste_assets::DisplayTransform::IDENTITY,
                    ground: haste_assets::DisplayTransform::IDENTITY,
                    fixed: haste_assets::DisplayTransform::IDENTITY,
                    on_shelf: haste_assets::DisplayTransform::IDENTITY,
                    gui_light_front: false,
                    transformation: None,
                }],
            })
        })
        .clone()
}

/// Vanilla's missing texture: 16x16 magenta/black checker of 8x8 squares.
fn missing_texture() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 16, |x, y| {
        if (x < 8) ^ (y < 8) {
            image::Rgba([248, 0, 248, 255])
        } else {
            image::Rgba([0, 0, 0, 255])
        }
    })
}

/// `haste:` placeholder texture: light grey with a darker 1 px border per 8 px cell.
fn placeholder_texture() -> image::RgbaImage {
    image::RgbaImage::from_fn(64, 64, |x, y| {
        if x % 8 == 0 || y % 8 == 0 {
            image::Rgba([120, 120, 130, 255])
        } else {
            image::Rgba([200, 200, 210, 255])
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frustum cull keeps entities inside the view and drops those behind the camera:
    /// vanilla `EntityRenderer.shouldRender` (`Frustum.isVisible` on the inflated AABB).
    /// A sphere around the model stands in for the AABB, so an entity half-in view stays.
    /// The planes come from the renderer's own `frustum_planes` on a straight, level view.
    #[test]
    fn frustum_cull_keeps_visible_and_drops_behind() {
        let camera = crate::Camera {
            pos: glam::DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_deg: 70.0,
            hand_fov_deg: 70.0,
            roll_deg: 0.0,
            bob: crate::ViewBob::default(),
        };
        let view_proj = crate::Renderer::view_projection(&camera, 1.0, None);
        let planes = crate::Renderer::frustum_planes(&view_proj);
        let r = 1.0;
        // Straight ahead at 10 blocks (yaw 0 looks along +z): kept.
        assert!(aabb_in_frustum(
            &planes,
            glam::DVec3::new(0.0, 0.0, 10.0),
            r
        ));
        // Behind the camera: dropped.
        assert!(!aabb_in_frustum(
            &planes,
            glam::DVec3::new(0.0, 0.0, -10.0),
            r
        ));
        // Ahead but far outside the 70-degree fov: dropped.
        assert!(!aabb_in_frustum(
            &planes,
            glam::DVec3::new(100.0, 0.0, 10.0),
            r
        ));
    }

    /// A model's render size (vanilla `Entity.shouldRenderAtSqrDistance` compares against
    /// `64 * boundingBox.getSize()`): an 8x8x8-unit cube spans half a block each way, so
    /// its diagonal is ~0.87 blocks (clamped to 1) and the sphere radius is the
    /// half-diagonal, ~0.43 blocks.
    #[test]
    fn model_extents_from_a_cube() {
        let part = ModelPart {
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: vec![haste_core::model::Cube {
                origin: Vec3::new(-4.0, 0.0, -4.0),
                size: Vec3::new(8.0, 8.0, 8.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            children: Vec::new(),
        };
        let (size, half) = model_extents(&part);
        let diagonal = (3.0_f32 * 8.0 * 8.0).sqrt();
        assert!(
            (size - (diagonal / 16.0).max(1.0)).abs() < 1e-5,
            "size {size}"
        );
        assert!((half - diagonal / 16.0 / 2.0).abs() < 1e-5, "half {half}");
    }

    /// `EntityRenderer.extractShadowPiece`: the shadow alpha fades with height above the
    /// block (`powerAtDepth = pow - (entity.y - pos.y) * 0.5`, then `* 0.5 * brightness`).
    #[test]
    fn shadow_alpha_fades_with_height_above_the_block() {
        // pow = 1: no camera distance falloff and shadow strength 1; the entity's feet at y=1.5.
        let pow = 1.0f32;
        let feet = 1.5f32;
        let alpha_at = |block_y: i32| {
            let power_at_depth = pow - (feet - block_y as f32) * 0.5;
            shadow_piece_alpha(power_at_depth, lightmap_brightness(0.0, 15))
        };
        // The block under the feet is strongest, one lower is a quarter as strong, two lower
        // is negative and clamps to zero.
        assert!((alpha_at(1) - 0.375).abs() < 1e-6, "{}", alpha_at(1));
        assert!((alpha_at(0) - 0.125).abs() < 1e-6, "{}", alpha_at(0));
        assert_eq!(alpha_at(-1), 0.0);
        // The alpha is clamped to 1.
        assert_eq!(shadow_piece_alpha(4.0, 1.0), 1.0);
    }

    /// `Lightmap.getBrightness`: the curved `v / (4 - 3v)` curve with the dimension's ambient
    /// light floor.
    #[test]
    fn lightmap_brightness_is_the_vanilla_curve() {
        assert_eq!(lightmap_brightness(0.0, 15), 1.0);
        assert_eq!(lightmap_brightness(0.0, 0), 0.0);
        // 7 / (15 * (4 - 3 * 7/15)) = 7/39.
        assert!((lightmap_brightness(0.0, 7) - 7.0 / 39.0).abs() < 1e-6);
        // Nether ambient 0.1 lifts every level towards 1.
        assert!((lightmap_brightness(0.1, 0) - 0.1).abs() < 1e-6);
    }

    /// `EntityRenderer.extractNameTags`/`LivingEntityRenderer.shouldShowName`: 64 blocks, 32
    /// while crouching (distanceSq >= 1024 hides it).
    #[test]
    fn name_tag_culls_at_64_and_32_when_crouching() {
        assert!(name_tag_visible(63.9 * 63.9, false));
        assert!(!name_tag_visible(64.1 * 64.1, false));
        assert!(name_tag_visible(31.9 * 31.9, true));
        assert!(!name_tag_visible(32.0 * 32.0, true));
        assert!(!name_tag_visible(40.0 * 40.0, true));
        // Standing is not affected by the 32 limit.
        assert!(name_tag_visible(40.0 * 40.0, false));
    }

    /// `body_matrix` composes the ops left to right like `LivingEntityRenderer.setupRotations`
    /// (`poseStack` calls): `rotateZ(90); translate(0, 2, 0)` maps the origin to (-2, 0, 0).
    #[test]
    fn body_matrix_applies_ops_in_vanilla_order() {
        let mut b = BodyTransform::default();
        b.push(BodyOp::RotateZ(std::f32::consts::FRAC_PI_2));
        b.push(BodyOp::Translate(Vec3::new(0.0, 2.0, 0.0)));
        let m = body_matrix(&b);
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.x - -2.0).abs() < 1e-5, "x {}", p.x);
        assert!(p.y.abs() < 1e-5, "y {}", p.y);
        // A single X rotation: (0, 1, 0) -> (0, 0, 1).
        let mut b = BodyTransform::default();
        b.push(BodyOp::RotateX(std::f32::consts::FRAC_PI_2));
        let p = body_matrix(&b).transform_point3(Vec3::Y);
        assert!((p.z - 1.0).abs() < 1e-5, "z {}", p.z);
    }

    /// Every texture haste-entity's model table names must exist in the client jar (the
    /// renderer loads them on demand by identifier; a typo would draw the placeholder at
    /// runtime with only a log line). Cross-crate on purpose: haste-render owns the jar.
    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn entity_table_textures_exist_in_the_jar() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        for texture in haste_entity::registry::table_textures() {
            let id = Identifier::minecraft(texture);
            assets
                .texture_any(&id)
                .unwrap_or_else(|e| panic!("table texture {texture}: {e:#}"));
        }
    }

    /// The item frame block models and the painting sprites this renderer loads must exist in
    /// the client jar (a typo would only show as placeholder quads at runtime).
    #[test]
    #[ignore = "needs the client jar in the cache"]
    fn frame_and_painting_assets_exist_in_the_jar() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        // `BlockModelResolver.updateForItemFrame` -> `item_frame` / `glow_item_frame`.
        for (block, map) in [
            ("item_frame", false),
            ("item_frame", true),
            ("glow_item_frame", false),
            ("glow_item_frame", true),
        ] {
            let def = assets
                .blockstate(&Identifier::minecraft(block))
                .unwrap_or_else(|e| panic!("blockstate {block}: {e:#}"));
            let refs = def.model_refs_for(&[("map".to_string(), map.to_string())]);
            assert!(!refs.is_empty(), "{block} map={map} has no model");
            for r in refs {
                let id = Identifier::parse(&r.model).unwrap();
                let quads = assets
                    .model_quads(&id)
                    .unwrap_or_else(|e| panic!("model {}: {e:#}", r.model));
                assert!(!quads.is_empty(), "{} has no quads", r.model);
                for q in &quads {
                    assets
                        .texture_any(&parse_tex(&q.texture))
                        .unwrap_or_else(|e| panic!("frame texture {}: {e:#}", q.texture));
                }
            }
        }
        // The painting front sprite and the back sprite (the `paintings` atlas directory).
        for sprite in ["painting/back", "painting/match", "painting/kebab"] {
            assets
                .texture_any(&Identifier::minecraft(sprite))
                .unwrap_or_else(|e| panic!("painting texture {sprite}: {e:#}"));
        }
    }

    #[test]
    fn a_cube_has_six_outward_ccw_faces() {
        let model = CubeModel {
            texture: Identifier::minecraft("x"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
                id: parts::ROOT,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: vec![haste_core::model::Cube {
                    origin: Vec3::new(-4.0, -8.0, -4.0),
                    size: Vec3::splat(8.0),
                    inflate: 0.0,
                    uv: [0, 0],
                    mirror: false,
                }],
                children: vec![],
            },
        };
        let mut out = Vec::new();
        let root = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0);
        emit_part_tinted(
            &mut out,
            &model,
            &model.root,
            &PoseOverrides::default(),
            root,
            [0, 0, 64, 64],
            (15, 15),
            0.0,
            [1.0; 3],
        );
        assert_eq!(out.len(), 36);
        // Every triangle faces away from the cube centre (0, 0.25, 0).
        let centre = Vec3::new(0.0, 0.25, 0.0);
        for t in out.chunks(3) {
            let p: Vec<Vec3> = t.iter().map(|v| Vec3::from(v.pos)).collect();
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            let mid = (p[0] + p[1] + p[2]) / 3.0;
            assert!(n.dot(mid - centre) > 0.0, "inward triangle {p:?}");
        }
        // The front face (texture 8..16 x 8..16) stays at model -z (the root flips x and y).
        let front: Vec<&[EntityVertex]> = out
            .chunks(3)
            .filter(|t| {
                t.iter()
                    .all(|v| (8.0..=16.0).contains(&v.uv[0]) && (8.0..=16.0).contains(&v.uv[1]))
            })
            .collect();
        assert_eq!(front.len(), 2);
        assert!(front
            .iter()
            .flat_map(|t| t.iter())
            .all(|v| (v.pos[2] + 0.25).abs() < 1e-5));
    }

    #[test]
    fn hurt_mixes_red_into_the_vertex_colour() {
        // Untinted: the colour is the shade alone.
        assert_eq!(hurt_color(0.8, 0.0), [0.8, 0.8, 0.8]);
        // Fully hurt at vanilla's ~30 % overlay: red boosted (clamped at 1.0 here),
        // green/blue pulled down.
        let [r, g, b] = hurt_color(0.8, 1.0);
        assert_eq!(r, 1.0);
        assert!((g - 0.8 * (1.0 - HURT_RED_STRENGTH)).abs() < 1e-6);
        assert_eq!(g, b);
        // Monotone in hurt: the red channel grows, green shrinks.
        assert!(hurt_color(0.8, 0.25)[0] < hurt_color(0.8, 0.75)[0]);
        assert!(hurt_color(0.8, 0.25)[1] > hurt_color(0.8, 0.75)[1]);
    }

    #[test]
    fn dropped_item_spin_is_age_over_20_plus_the_phase() {
        // `ItemEntity.getSpin`: radians, the age over 20 plus the entity's phase, so
        // one turn takes 40*pi ticks (about 125.7) — a slow spin.
        assert_eq!(item_entity_spin(0.0, 0.0), 0.0);
        assert_eq!(item_entity_spin(20.0, 0.0), 1.0);
        assert!((item_entity_spin(10.0, 0.0) - 0.5).abs() < 1e-6);
        assert!((item_entity_spin(0.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((item_entity_spin(10.0, 1.0) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn dropped_item_bobs_a_tenth_of_a_block_every_20_pi_ticks() {
        // `ItemEntityRenderer.submit`: sin(age / 10 + phase) * 0.1 + 0.1, so the bob
        // spans 0..0.2 blocks with a 20*pi-tick period (about 62.8).
        assert!((item_entity_bob(0.0, 0.0) - 0.1).abs() < 1e-6);
        // sin tops out a quarter period in: age = 5*pi.
        assert!((item_entity_bob(5.0 * std::f32::consts::PI, 0.0) - 0.2).abs() < 1e-5);
        assert!((item_entity_bob(15.0 * std::f32::consts::PI, 0.0) - 0.0).abs() < 1e-5);
        // The phase shifts the whole curve.
        assert!((item_entity_bob(0.0, std::f32::consts::FRAC_PI_2) - 0.2).abs() < 1e-5);
        for age in [0.0f32, 7.5, 19.3, 33.7, 60.1] {
            let b = item_entity_bob(age, entity_phase(EntityId(7)));
            assert!((0.0..=0.2).contains(&b), "bob {b} at age {age}");
        }
    }

    #[test]
    fn dropped_item_copies_follow_the_stack_size() {
        // `ItemClusterRenderState.getRenderedAmount`: 1 for a single item, then one
        // more per full 16 of the count, up to 5.
        assert_eq!(item_entity_copies(0), 1);
        assert_eq!(item_entity_copies(1), 1);
        assert_eq!(item_entity_copies(2), 2);
        assert_eq!(item_entity_copies(16), 2);
        assert_eq!(item_entity_copies(17), 3);
        assert_eq!(item_entity_copies(32), 3);
        assert_eq!(item_entity_copies(33), 4);
        assert_eq!(item_entity_copies(48), 4);
        assert_eq!(item_entity_copies(49), 5);
        assert_eq!(item_entity_copies(64), 5);
        assert_eq!(item_entity_copies(99), 5);
    }

    #[test]
    fn dropped_item_lift_puts_the_bbox_bottom_a_sixteenth_up() {
        // `ItemEntityRenderer.submit` lifts by -bbox.minY + 1/16 (the bounding box is the
        // model through its ground display, so a block item reaching minY -0.125 needs
        // 0.1875 and a centred model spanning -0.5..0.5 needs 0.5625).
        assert!((item_entity_lift(-0.5) - 0.5625).abs() < 1e-6);
        assert!((item_entity_lift(-0.125) - 0.1875).abs() < 1e-6);
        assert!((item_entity_lift(0.0) - 0.0625).abs() < 1e-6);
        // A model that dips below 0 needs more lift; one that starts above 0, less.
        assert!(item_entity_lift(-0.25) > item_entity_lift(0.0));
        assert!(item_entity_lift(0.25) < item_entity_lift(0.0));
    }

    #[test]
    fn entity_phase_and_seed_streams_cover_their_ranges() {
        // The phase covers [0, 2pi) across ids and is stable for one id.
        let phases: Vec<f32> = (0..200).map(|i| entity_phase(EntityId(i))).collect();
        assert!(phases
            .iter()
            .all(|p| (0.0..std::f32::consts::TAU).contains(p)));
        assert!(phases.iter().any(|p| *p > std::f32::consts::PI));
        assert!(phases.iter().any(|p| *p < std::f32::consts::PI));
        assert_eq!(entity_phase(EntityId(42)), entity_phase(EntityId(42)));
        // The seeded stream is in [0,1) and stable per (seed, index).
        for (seed, i) in [(0u32, 0u32), (5, 1), (1234, 7), (187, 3)] {
            let r = seeded_float(seed, i);
            assert!((0.0..1.0).contains(&r));
            assert_eq!(r, seeded_float(seed, i));
        }
        let values: Vec<f32> = (0..50).map(|i| seeded_float(9, i)).collect();
        assert!(values.iter().any(|r| *r < 0.33));
        assert!(values.iter().any(|r| *r > 0.66));
    }

    #[test]
    fn head_top_of_the_humanoid_layout() {
        let part = ModelPart {
            name: "head".into(),
            id: parts::HEAD,
            pivot: Vec3::new(0.0, -24.0, 0.0),
            rotation: Vec3::ZERO,
            visible: true,
            cubes: vec![haste_core::model::Cube {
                origin: Vec3::new(-4.0, -8.0, -4.0),
                size: Vec3::splat(8.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            children: vec![],
        };
        assert_eq!(model_top(&part, 0.0), 32.0);
    }

    /// A hand with no animation: no swing, the equip progress up, no bob, no lag.
    fn still_hand() -> FirstPerson {
        FirstPerson {
            skin: None,
            slim: false,
            sleeve: true,
            item: None,
            light: (15, 0),
            attack: TickPair::fixed(0.0),
            equip: TickPair::fixed(1.0),
            x_bob: TickPair::fixed(0.0),
            y_bob: TickPair::fixed(0.0),
        }
    }

    #[test]
    fn tick_pair_lerps_the_two_ends_of_a_tick() {
        // `Mth.lerp(partial, start, end)`.
        let p = TickPair {
            start: 0.25,
            end: 0.75,
        };
        assert_eq!(p.at(0.0), 0.25);
        assert_eq!(p.at(0.5), 0.5);
        assert_eq!(p.at(1.0), 0.75);
        assert_eq!(TickPair::fixed(0.5).at(0.37), 0.5);
    }

    #[test]
    fn no_bob_and_no_lag_is_the_identity() {
        // With nothing animated, `submitHandsWithItems`'s outer transforms change nothing.
        assert_eq!(
            hand_root_matrix(&still_hand(), (0.0, 0.0), 0.0),
            Mat4::IDENTITY
        );
    }

    #[test]
    fn the_arm_sits_at_vanilla_rest_position() {
        // The arm's vertices through `renderPlayerArm`'s transform, the right arm's own
        // `PartPose` (-5, 2, 0) and its 0.1 rad z tilt: the hand ends up right of the view
        // centre, below it and in front of the camera, reaching down and forward.
        let m = arm_matrix(&still_hand(), 0.0) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
        let model = arm_model(false, true);
        let mut out = Vec::new();
        emit_part_tinted(
            &mut out,
            &model,
            &model.root,
            &PoseOverrides::default(),
            m,
            [0, 0, 64, 64],
            (15, 15),
            0.0,
            [1.0; 3],
        );
        // The arm and its sleeve, six faces of six vertices each.
        assert_eq!(out.len(), 72);
        let lo = out
            .iter()
            .map(|v| Vec3::from(v.pos))
            .reduce(Vec3::min)
            .unwrap();
        let hi = out
            .iter()
            .map(|v| Vec3::from(v.pos))
            .reduce(Vec3::max)
            .unwrap();
        println!("the bare arm's box: {lo:?} .. {hi:?}");
        assert!(lo.x > 0.0 && hi.x < 1.2, "x {lo:?} .. {hi:?}");
        assert!(hi.y < 0.0 && lo.y > -1.6, "y {lo:?} .. {hi:?}");
        assert!(hi.z < -0.3 && lo.z > -1.6, "z {lo:?} .. {hi:?}");
        // A swing moves the whole hand (every vertex moves, and the arm's box widens upwards).
        let mut swung_hand = still_hand();
        swung_hand.attack = TickPair::fixed(0.25);
        let swung = arm_matrix(&swung_hand, 0.0) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
        let mut out2 = Vec::new();
        emit_part_tinted(
            &mut out2,
            &arm_model(false, true),
            &arm_model(false, true).root,
            &PoseOverrides::default(),
            swung,
            [0, 0, 64, 64],
            (15, 15),
            0.0,
            [1.0; 3],
        );
        let moved = out
            .iter()
            .zip(&out2)
            .filter(|(a, b)| (Vec3::from(a.pos) - Vec3::from(b.pos)).length() > 1e-4)
            .count();
        assert_eq!(moved, out.len(), "the swing must move every vertex");
    }

    /// Compares two matrices element by within `tol`.
    fn close(a: Mat4, b: Mat4, tol: f32) -> bool {
        a.to_cols_array_2d()
            .iter()
            .zip(b.to_cols_array_2d())
            .all(|(x, y)| x.iter().zip(y).all(|(x, y)| (x - y).abs() < tol))
    }

    /// `FirstPersonHandsAndItemsRenderer.renderPlayerArm`, transcribed pose-stack call by
    /// pose-stack call (translate in blocks, rotations in degrees): the renderer's chain must
    /// produce the same matrix.
    fn vanilla_render_player_arm(attack: f32, inverse_arm_height: f32) -> Mat4 {
        let pi = std::f32::consts::PI;
        let sqrt_attack = mth_sqrt(attack);
        let mut m = Mat4::IDENTITY;
        let x = -0.3 * (sqrt_attack * pi).sin();
        let y = 0.4 * (sqrt_attack * (pi * 2.0)).sin();
        let z = -0.4 * (attack * pi).sin();
        m *= Mat4::from_translation(Vec3::new(
            x + 0.64000005,
            y + -0.6 + inverse_arm_height * -0.6,
            z + -0.71999997,
        ));
        m *= rot_deg(1, 45.0);
        let z_swing = (attack * attack * pi).sin();
        let y_swing = (sqrt_attack * pi).sin();
        m *= rot_deg(1, y_swing * 70.0);
        m *= rot_deg(2, z_swing * -20.0);
        m *= Mat4::from_translation(Vec3::new(-1.0, 3.6, 3.5));
        m *= rot_deg(2, 120.0);
        m *= rot_deg(0, 200.0);
        m *= rot_deg(1, -135.0);
        m *= Mat4::from_translation(Vec3::new(5.6, 0.0, 0.0));
        m
    }

    /// `Mth.sqrt`: NaN for negative values, the root otherwise.
    fn mth_sqrt(v: f32) -> f32 {
        if v < 0.0 {
            f32::NAN
        } else {
            v.sqrt()
        }
    }

    /// `rotateDegrees(Axis.XP|YP|ZP, degrees)`: 0 = X, 1 = Y, 2 = Z.
    fn rot_deg(axis: u8, deg: f32) -> Mat4 {
        let r = deg.to_radians();
        match axis {
            0 => Mat4::from_rotation_x(r),
            1 => Mat4::from_rotation_y(r),
            _ => Mat4::from_rotation_z(r),
        }
    }

    #[test]
    fn the_empty_hands_transform_is_vanillas_render_player_arm() {
        for (attack, equip) in [(0.0, 1.0), (0.25, 1.0), (0.5, 0.5), (0.75, 1.0), (1.0, 0.0)] {
            let mut h = still_hand();
            h.attack = TickPair::fixed(attack);
            h.equip = TickPair::fixed(equip);
            let want = vanilla_render_player_arm(attack, 1.0 - equip);
            assert!(
                close(arm_matrix(&h, 0.0), want, 1e-6),
                "attack {attack} equip {equip}: {:?} vs {want:?}",
                arm_matrix(&h, 0.0).to_cols_array_2d(),
            );
        }
    }

    #[test]
    fn the_held_items_transform_is_vanillas_apply_item_arm_transform() {
        // `applyItemArmTransform` + `swingArm`'s translate + `applyItemArmAttackTransform`, at a
        // quarter of the swing: -0.4 * sin(PI / 2) = -0.4 left and -0.2 * sin(PI / 4) =
        // -0.14142136 forward, plus the swing's rotations.
        for (attack, equip) in [(0.0, 1.0), (0.25, 1.0), (0.5, 0.4), (0.75, 1.0), (1.0, 0.0)] {
            let mut h = still_hand();
            h.attack = TickPair::fixed(attack);
            h.equip = TickPair::fixed(equip);
            let pi = std::f32::consts::PI;
            let sqrt_attack = mth_sqrt(attack);
            let inverse_arm_height = 1.0 - equip;
            let mut want =
                Mat4::from_translation(Vec3::new(0.56, -0.52 + inverse_arm_height * -0.6, -0.72));
            want *= Mat4::from_translation(Vec3::new(
                -0.4 * (sqrt_attack * pi).sin(),
                0.2 * (sqrt_attack * (pi * 2.0)).sin(),
                -0.2 * (attack * pi).sin(),
            ));
            let y_swing = (attack * attack * pi).sin();
            want *= rot_deg(1, 45.0 + y_swing * -20.0);
            let xz_swing = (sqrt_attack * pi).sin();
            want *= rot_deg(2, xz_swing * -20.0);
            want *= rot_deg(0, xz_swing * -80.0);
            want *= rot_deg(1, -45.0);
            assert!(
                close(item_arm_matrix(&h, 0.0), want, 1e-6),
                "attack {attack} equip {equip}: {:?} vs {want:?}",
                item_arm_matrix(&h, 0.0).to_cols_array_2d(),
            );
        }
    }

    #[test]
    fn the_swing_is_the_interpolated_attack_value() {
        // A swing's tick pair lerps into the arm and item transforms at the partial tick
        // (vanilla interpolates `swingAnimation` the same way before both transforms read it):
        // mid-tick of a 0 -> 0.5 swing is the 0.25 matrix, not an average of matrices.
        let mut h = still_hand();
        h.attack = TickPair {
            start: 0.0,
            end: 0.5,
        };
        for partial in [0.25f32, 0.5, 0.75] {
            let mut lerped = still_hand();
            lerped.attack = TickPair::fixed(h.attack.at(partial));
            assert!(
                close(arm_matrix(&h, partial), arm_matrix(&lerped, 0.0), 1e-6),
                "arm at partial {partial}"
            );
            assert!(
                close(
                    item_arm_matrix(&h, partial),
                    item_arm_matrix(&lerped, 0.0),
                    1e-6
                ),
                "item at partial {partial}"
            );
        }
        // The mid-tick matrix is none of the ends' matrices.
        assert!(!close(
            arm_matrix(&h, 0.5),
            arm_matrix(&still_hand(), 0.0),
            1e-4
        ));
    }

    #[test]
    fn a_lowered_equip_progress_drops_the_hand() {
        // `inverseArmHeight = 1 - equip` is inside the outermost translate, so dropping the
        // equip progress from 1 to 0 moves the arm (and the item) down by exactly 0.6 blocks:
        // `renderPlayerArm`'s `inverseArmHeight * -0.6` and `applyItemArmTransform`'s.
        let mut down = still_hand();
        down.equip = TickPair::fixed(0.0);
        for (rest, moved) in [
            (arm_matrix(&still_hand(), 0.0), arm_matrix(&down, 0.0)),
            (
                item_arm_matrix(&still_hand(), 0.0),
                item_arm_matrix(&down, 0.0),
            ),
        ] {
            let d = moved.transform_point3(Vec3::ZERO) - rest.transform_point3(Vec3::ZERO);
            assert!(d.x.abs() < 1e-6 && d.z.abs() < 1e-6, "{d:?}");
            assert!((d.y - -0.6).abs() < 1e-6, "{d:?}");
        }
    }

    #[test]
    fn the_walk_bob_is_carried_by_the_prefix_not_the_hand_root() {
        // Vanilla applies `bobView` to the hand exactly once, on `renderItemInHand`'s pose
        // stack (`GameRenderer.bobView`, the same stack the world's projection carries); the
        // hand's own root (`submitHandsWithItems`) has only the view-lag rotations. The
        // prefix is the renderer's `bob_transform`, so with a bobbing prefix the hand root
        // must not add, remove or reorder anything: the prefix is all there is.
        let mut h = still_hand();
        h.x_bob = TickPair::fixed(20.0);
        let lag = hand_root_matrix(&h, (25.0, 35.0), 0.5);
        // A pure view-bob prefix (translation only): the composed matrix is the prefix
        // times the lag rotations, never the translation twice.
        let prefix = Mat4::from_translation(Vec3::new(0.25, -0.5, 0.0));
        let m = prefix * lag;
        assert!(close(
            m,
            prefix * hand_root_matrix(&h, (25.0, 35.0), 0.5),
            1e-7
        ));
        assert!((m.transform_point3(Vec3::ZERO) - Vec3::new(0.25, -0.5, 0.0)).length() < 1e-6);
        // `hand_root_matrix` with nothing animated is the identity: the bob lives outside.
        assert_eq!(
            hand_root_matrix(&still_hand(), (0.0, 0.0), 1.0),
            Mat4::IDENTITY
        );
    }

    #[test]
    fn the_hand_lags_behind_the_camera_turn() {
        // `submitHandsWithItems` rotates the hand by (viewXRot - xBob) * 0.1 on X and
        // (viewYRot - yBob) * 0.1 on Y, in degrees.
        let mut h = still_hand();
        h.x_bob = TickPair::fixed(20.0);
        h.y_bob = TickPair::fixed(30.0);
        let m = hand_root_matrix(&h, (25.0, 35.0), 0.0);
        // The yaw lags 30 degrees behind a look at 25: (25 - 30) * 0.1 = -0.5 degrees turns the
        // hand half a degree to the left; the pitch lags 20 behind 35: 1.5 degrees up.
        let turned = m.transform_vector3(Vec3::Z);
        assert!(
            (turned.x - -(0.5f32).to_radians().sin()).abs() < 1e-5,
            "{turned:?}"
        );
        assert!(
            (turned.y - -(1.5f32).to_radians().sin()).abs() < 1e-5,
            "{turned:?}"
        );
        // The hand's up axis pitches the same way.
        let pitched = m.transform_vector3(Vec3::Y);
        assert!(
            (pitched.z - (1.5f32).to_radians().sin()).abs() < 1e-5,
            "{pitched:?}"
        );
        // With no bob at all the hand still carries the whole view lag, the way a fresh client
        // does before its bob has caught up: (35 - 0) * 0.1 = 3.5 degrees of pitch and
        // (25 - 0) * 0.1 = 2.5 degrees of yaw.
        let no_lag = hand_root_matrix(&still_hand(), (25.0, 35.0), 0.0);
        let mut want = Mat4::IDENTITY;
        want *= Mat4::from_rotation_x(3.5f32.to_radians());
        want *= Mat4::from_rotation_y(2.5f32.to_radians());
        assert!(close(no_lag, want, 1e-6), "{no_lag:?}");
    }

    #[test]
    fn the_arm_model_is_the_player_models_right_arm() {
        // `HumanoidModel.createMesh` + `PlayerModel.createMesh` + `AvatarRenderer.renderHand`.
        let wide = arm_model(false, true);
        assert_eq!(wide.root.pivot, Vec3::new(-5.0, 2.0, 0.0));
        assert_eq!(wide.root.rotation, Vec3::new(0.0, 0.0, 0.1));
        let cube = &wide.root.cubes[0];
        assert_eq!(cube.origin, Vec3::new(-3.0, -2.0, -2.0));
        assert_eq!(cube.size, Vec3::new(4.0, 12.0, 4.0));
        assert_eq!(cube.uv, [40, 16]);
        let sleeve = &wide.root.children[0];
        assert_eq!(sleeve.name, "right_sleeve");
        assert_eq!(sleeve.cubes[0].uv, [40, 32]);
        assert_eq!(sleeve.cubes[0].inflate, 0.25);
        // Slim: a 3 px arm one pixel to the right.
        let slim = arm_model(true, true);
        assert_eq!(slim.root.cubes[0].origin, Vec3::new(-2.0, -2.0, -2.0));
        assert_eq!(slim.root.cubes[0].size, Vec3::new(3.0, 12.0, 4.0));
    }

    /// `ItemInHandLayer.submitArmWithItem`: rotate X -90, rotate Y 180, translate
    /// (±1, 2, -10)/16 (adult). The left hand is the right hand offset to -1 in x.
    #[test]
    fn the_held_item_layer_transform_is_vanillas() {
        let deg = |d: f32| d.to_radians();
        let right = held_item_layer_matrix(false);
        let mut want = Mat4::from_rotation_x(deg(-90.0));
        want *= Mat4::from_rotation_y(deg(180.0));
        want *= Mat4::from_translation(Vec3::new(1.0 / 16.0, 2.0 / 16.0, -10.0 / 16.0));
        assert!(close(right, want, 1e-6));
        let left = held_item_layer_matrix(true);
        let mut want_left = Mat4::from_rotation_x(deg(-90.0));
        want_left *= Mat4::from_rotation_y(deg(180.0));
        want_left *= Mat4::from_translation(Vec3::new(-1.0 / 16.0, 2.0 / 16.0, -10.0 / 16.0));
        assert!(close(left, want_left, 1e-6));
    }

    /// `ItemTransform.apply(true, ...)`: the left-hand item negates translation.x and
    /// rotation.y/z of the third-person transform.
    #[test]
    fn the_left_hand_fixes_the_third_person_transform() {
        let part = haste_assets::ItemModelPart {
            geometry: haste_assets::ItemGeometry::None,
            tints: vec![],
            gui: haste_assets::DisplayTransform::IDENTITY,
            firstperson: haste_assets::DisplayTransform::IDENTITY,
            thirdperson: haste_assets::DisplayTransform::IDENTITY,
            head: haste_assets::DisplayTransform::IDENTITY,
            ground: haste_assets::DisplayTransform::IDENTITY,
            fixed: haste_assets::DisplayTransform::IDENTITY,
            on_shelf: haste_assets::DisplayTransform::IDENTITY,
            gui_light_front: false,
            transformation: None,
        };
        let d = haste_assets::DisplayTransform {
            rotation: [10.0, 20.0, 30.0],
            translation: [0.1, 0.2, 0.3],
            scale: [0.5, 0.5, 0.5],
        };
        let want = item_display_matrix(
            &part,
            &haste_assets::DisplayTransform {
                rotation: [10.0, -20.0, -30.0],
                translation: [-0.1, 0.2, 0.3],
                scale: [0.5, 0.5, 0.5],
            },
            false,
        );
        assert!(close(item_display_matrix(&part, &d, true), want, 1e-6));
    }

    /// `ArmedModel.translateToHand` -> `part_matrix` gives the arm's animated matrix: the arm's
    /// pivot and rotation plus the pose override, under the entity root.
    #[test]
    fn part_matrix_places_the_hand_at_the_animated_arm() {
        let model = haste_entity_test_humanoid();
        let mut pose = PoseOverrides::default();
        let rest = part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::RIGHT_ARM).unwrap();
        // The right arm's own pivot (no override): (-5, -22, 0).
        assert_eq!(
            rest.transform_point3(Vec3::ZERO),
            Vec3::new(-5.0, -22.0, 0.0)
        );
        // A pose override rotates the arm about its pivot.
        pose.set_rotation(
            parts::RIGHT_ARM,
            Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2),
        );
        let swung = part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::RIGHT_ARM).unwrap();
        let p = swung.transform_point3(Vec3::new(0.0, 10.0, 0.0));
        assert!(
            (p.x - -15.0).abs() < 1e-4 && (p.y - -22.0).abs() < 1e-4,
            "{p:?}"
        );
        // A part no model carries resolves to nothing.
        assert!(part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::intern("nope")).is_none());
    }

    /// `CustomHeadLayer.translateToHead`: translate (0, -0.25, 0), rotate Y 180, scale
    /// (0.625, -0.625, -0.625).
    #[test]
    fn the_custom_head_transform_is_vanillas() {
        let deg = |d: f32| d.to_radians();
        let want = Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_rotation_y(deg(180.0))
            * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625));
        assert!(close(custom_head_layer_matrix(), want, 1e-6));
    }

    /// `ElytraModel.createLayer`: a left and a right wing at the shoulders, 64x32 texture.
    #[test]
    fn the_elytra_model_has_two_wings() {
        let m = elytra_model();
        assert_eq!(m.tex_width, 64);
        assert_eq!(m.tex_height, 32);
        assert_eq!(m.root.children.len(), 2);
        let left = &m.root.children[0];
        assert_eq!(left.name, "left_wing");
        assert_eq!(left.pivot, Vec3::new(5.0, -24.0, 0.0));
        assert_eq!(left.cubes[0].size, Vec3::new(10.0, 20.0, 2.0));
        assert_eq!(left.cubes[0].inflate, 1.0);
        assert!(!left.cubes[0].mirror);
        assert!(m.root.children[1].cubes[0].mirror);
    }

    /// The armour model has the parts vanilla retains per slot, with the right deformations.
    #[test]
    fn armour_models_keep_vanillas_parts() {
        use haste_core::model::EquipmentSlot as S;
        let head = humanoid_armor(S::Head);
        assert!(head.root.children.iter().any(|c| c.name == "head"));
        assert_eq!(head.tex_width, 64);
        assert_eq!(head.tex_height, 32);
        let chest = humanoid_armor(S::Chest);
        assert_eq!(chest.root.children.len(), 3);
        assert!(chest.root.children.iter().any(|c| c.name == "left_arm"));
        let legs = humanoid_armor(S::Legs);
        let body = legs
            .root
            .children
            .iter()
            .find(|c| c.name == "body")
            .unwrap();
        assert_eq!(body.cubes[0].inflate, 0.5);
        let feet = humanoid_armor(S::Feet);
        assert_eq!(feet.root.children.len(), 2);
        assert_eq!(feet.root.children[0].cubes[0].inflate, 0.9);
        // The left arm/leg mirror their texture.
        assert!(
            chest
                .root
                .children
                .iter()
                .find(|c| c.name == "left_arm")
                .unwrap()
                .cubes[0]
                .mirror
        );
    }

    /// A humanoid model for the tests above (the renderer cannot depend on haste-entity).
    fn haste_entity_test_humanoid() -> CubeModel {
        CubeModel {
            texture: Identifier::minecraft("x"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
                id: parts::ROOT,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: vec![],
                children: vec![ModelPart {
                    name: "right_arm".into(),
                    id: parts::RIGHT_ARM,
                    pivot: Vec3::new(-5.0, -22.0, 0.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![],
                    children: vec![],
                }],
            },
        }
    }

    #[test]
    fn item_frame_facing_transform_matches_vanilla() {
        use haste_core::Direction as D;
        let deg = |d: f32| d.to_radians();
        // `ItemFrameRenderer.submit`: translate to `direction * 0.46875`, then the rotations.
        assert!(close(
            item_frame_transform(D::North),
            Mat4::from_translation(Vec3::new(0.0, 0.0, -0.46875)),
            1e-6
        ));
        assert!(close(
            item_frame_transform(D::South),
            Mat4::from_translation(Vec3::new(0.0, 0.0, 0.46875))
                * Mat4::from_rotation_y(deg(180.0)),
            1e-6
        ));
        assert!(close(
            item_frame_transform(D::West),
            Mat4::from_translation(Vec3::new(-0.46875, 0.0, 0.0))
                * Mat4::from_rotation_y(deg(90.0)),
            1e-6
        ));
        assert!(close(
            item_frame_transform(D::East),
            Mat4::from_translation(Vec3::new(0.46875, 0.0, 0.0))
                * Mat4::from_rotation_y(deg(-90.0)),
            1e-6
        ));
        assert!(close(
            item_frame_transform(D::Up),
            Mat4::from_translation(Vec3::new(0.0, 0.46875, 0.0))
                * Mat4::from_rotation_x(deg(-90.0))
                * Mat4::from_rotation_y(deg(180.0)),
            1e-6
        ));
        assert!(close(
            item_frame_transform(D::Down),
            Mat4::from_translation(Vec3::new(0.0, -0.46875, 0.0))
                * Mat4::from_rotation_x(deg(90.0))
                * Mat4::from_rotation_y(deg(180.0)),
            1e-6
        ));
    }

    #[test]
    fn item_frame_item_rotates_45_degrees_per_step_at_half_scale() {
        // `ItemFrameRenderer.submit`: `rotateDegrees(ZP, rotation * 45)`, then scale 0.5.
        let rest = item_frame_item_transform(0);
        assert!(close(
            rest,
            Mat4::from_translation(Vec3::new(0.0, 0.0, 0.4375))
                * Mat4::from_scale(Vec3::splat(0.5)),
            1e-6
        ));
        let one = item_frame_item_transform(1);
        assert!(close(
            one,
            Mat4::from_translation(Vec3::new(0.0, 0.0, 0.4375))
                * Mat4::from_rotation_z((45.0f32).to_radians())
                * Mat4::from_scale(Vec3::splat(0.5)),
            1e-6
        ));
        // The model's x axis turns 45 degrees and shrinks to half.
        let x = one.transform_vector3(Vec3::X);
        assert!((x.length() - 0.5).abs() < 1e-6, "{x:?}");
        assert!(
            (x.x - 0.5 * (45.0f32).to_radians().cos()).abs() < 1e-6
                && (x.y - 0.5 * (45.0f32).to_radians().sin()).abs() < 1e-6,
            "{x:?}"
        );
    }

    #[test]
    fn painting_facing_transform_matches_vanilla() {
        use haste_core::Direction as D;
        let deg = |d: f32| d.to_radians();
        assert!(close(painting_transform(D::North), Mat4::IDENTITY, 1e-6));
        assert!(close(
            painting_transform(D::South),
            Mat4::from_rotation_y(deg(180.0)),
            1e-6
        ));
        assert!(close(
            painting_transform(D::West),
            Mat4::from_rotation_y(deg(90.0)),
            1e-6
        ));
        assert!(close(
            painting_transform(D::East),
            Mat4::from_rotation_y(deg(-90.0)),
            1e-6
        ));
    }
}
