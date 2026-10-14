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
use haste_core::{EntityId, Identifier};

use crate::gui::Shelf;

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
/// item frames, ground for dropped items).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemDisplay {
    FirstPerson,
    Fixed,
    Ground,
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

/// The default skin vanilla gives a player without a downloaded skin: the UUID's Java
/// `hashCode` modulo the 18 default skins (nine names, slim then wide), per minecraft.wiki
/// "Skin" ("the default skin is chosen from the UUID"). Returns the texture and whether the
/// slim (3 px arm) model goes with it.
pub fn default_skin(uuid: u128) -> (Identifier, bool) {
    const NAMES: [&str; 9] = [
        "alex", "ari", "efe", "kai", "makena", "noor", "steve", "sunny", "zuri",
    ];
    let hi = (uuid >> 64) as u64;
    let lo = uuid as u64;
    let hilo = hi ^ lo;
    let hash = ((hilo >> 32) as i32) ^ (hilo as i32);
    let i = hash.rem_euclid(18) as usize;
    let slim = i < 9;
    let kind = if slim { "slim" } else { "wide" };
    (
        Identifier::minecraft(&format!("entity/player/{kind}/{}", NAMES[i % 9])),
        slim,
    )
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
    /// Walk bob amplitude (`ClientAvatarState.bob`).
    pub bob: TickPair,
    /// Distance walked in blocks, the bob's phase (`ClientAvatarState.walkDist`).
    pub walk_dist: TickPair,
}

/// A name tag to draw: world anchor (above the head) and the text.
#[derive(Clone, Debug)]
pub struct Nametag {
    pub anchor: glam::DVec3,
    pub text: String,
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
    lights: HashMap<EntityId, (u8, u8)>,
    pub nametags: Vec<Nametag>,
    hand_buffer: wgpu::Buffer,
    hand_capacity: usize,
    hand: Vec<EntityVertex>,
    item_names: Vec<Identifier>,
    item_models: HashMap<u32, Option<Arc<haste_assets::ItemModel>>>,
    /// The first-person arm model, by [`FirstPerson::slim`] (built once on first use).
    arm_models: [Option<Arc<CubeModel>>; 2],
    /// The item frame block model quads, by `(glow, map)`, built once on first use.
    frame_models: HashMap<(bool, bool), Arc<Vec<haste_assets::BakedQuad>>>,
    pub drawn: u32,
}

impl EntityRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        lightmap_layout: &wgpu::BindGroupLayout,
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
        let vertex_capacity = 8192;
        let vertex_buffer = make_vertex_buffer(&device, vertex_capacity);
        let hand_buffer = make_vertex_buffer(&device, 1024);
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
            lights: HashMap::new(),
            nametags: Vec::new(),
            hand_buffer,
            hand_capacity: 1024,
            hand: Vec::new(),
            item_names: Vec::new(),
            item_models: HashMap::new(),
            arm_models: [None, None],
            frame_models: HashMap::new(),
            drawn: 0,
        }
    }

    /// Sky/block light at each entity's position (sampled by the caller with the world at hand).
    /// Entities without an entry are drawn at full sky light.
    pub fn set_lights(&mut self, lights: impl IntoIterator<Item = (EntityId, (u8, u8))>) {
        self.lights.clear();
        self.lights.extend(lights);
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
        assets: &mut Assets,
    ) {
        self.vertices.clear();
        self.nametags.clear();
        self.drawn = 0;
        for e in entities {
            if Some(e.id) == skip {
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
            emit_part(
                &mut self.vertices,
                &e.model,
                &e.model.root,
                &e.pose,
                root,
                rect,
                light,
                e.hurt,
            );
            if self.vertices.len() > start {
                self.drawn += 1;
            }
            if let Some(name) = &e.nametag {
                let top = model_top(&e.model.root, 0.0) * scale / 16.0;
                self.nametags.push(Nametag {
                    anchor: e.pos + glam::DVec3::new(0.0, top as f64 + 0.5, 0.0),
                    text: name.clone(),
                });
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
        // The model's bounding box (through the ground transform) decides the lift and
        // whether the copies of a stack spread in 3D or line up flat
        // (`submitMultipleFromCount` branches on the z size).
        let (bbox_min, bbox_max) = item_model_bbox(&model);
        let lift = item_entity_lift(bbox_min.y);
        let base = Mat4::from_translation(
            (e.pos - camera).as_vec3()
                + Vec3::new(0.0, lift + item_entity_bob(dropped.age, bob_offs), 0.0),
        ) * Mat4::from_rotation_y(item_entity_spin(dropped.age, bob_offs));
        let copies = item_entity_copies(dropped.count) as usize;
        // `ItemClusterRenderState.getSeedForItemStack`: the renderer reseeds its random
        // with the item each frame, so the copy offsets are stable per item type (ours
        // hashes the registry id and stream index; matching Java's Random is not needed).
        let rand = |i: u32| seeded_float(dropped.item.0, i);
        // The copies: the first sits at the origin, each further one offset by
        // `submitMultipleFromCount` (3D: +-0.15 in x, y and z; flat: spread along z by
        // 1.5x the sprite depth, jittered +-0.075 in x and y).
        let depth = bbox_max.z - bbox_min.z;
        let flat = depth <= 1.0 / 16.0;
        let step = depth * 1.5;
        // Take the vertex buffer out so the texture lookup (`&mut self`) and the emission
        // (the buffer) do not fight over the borrow.
        let mut out = std::mem::take(&mut self.vertices);
        for copy in 0..copies {
            let mut off = Vec3::ZERO;
            if copy > 0 {
                if flat {
                    off = Vec3::new(
                        (rand(2 * copy as u32 - 2) * 2.0 - 1.0) * 0.075,
                        (rand(2 * copy as u32 - 1) * 2.0 - 1.0) * 0.075,
                        step * copy as f32 - step * (copies as f32 - 1.0) / 2.0,
                    );
                } else {
                    off = Vec3::new(
                        (rand(3 * copy as u32 - 3) * 2.0 - 1.0) * 0.15,
                        (rand(3 * copy as u32 - 2) * 2.0 - 1.0) * 0.15,
                        (rand(3 * copy as u32 - 1) * 2.0 - 1.0) * 0.15,
                    );
                }
            }
            let m = base * Mat4::from_translation(off);
            self.emit_item(&mut out, &model, m, light, ItemDisplay::Ground, assets);
        }
        self.vertices = out;
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

    /// The item model of a registry index, loaded once and cached (`air` and unknown ids map to
    /// `None`, never a panic).
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

    /// The empty hand's arm model for the skin's arm width, built once (no per-frame work).
    fn arm_model(&mut self, slim: bool) -> Arc<CubeModel> {
        let slot = &mut self.arm_models[usize::from(slim)];
        slot.get_or_insert_with(|| Arc::new(arm_model(slim)))
            .clone()
    }

    /// Builds the first-person arm or held item, following vanilla's
    /// `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`: the walk bob
    /// (`GameRenderer.bobView`) and the hand's lag behind the camera turn are applied to the
    /// whole hand in view space, then per hand vanilla's own transform, then the model.
    ///
    /// `view_to_world` rotates view space (x right, y up, -z forward) into the camera-relative
    /// world, `view` is the camera's look (yaw, pitch) in degrees and `partial` the partial
    /// tick; the per-tick animation values live in `fp` and are only lerped here.
    ///
    /// Nothing is allocated per frame: the arm model is cached by `slim` and the vertices go
    /// into the reused hand buffer.
    pub fn prepare_hand(
        &mut self,
        fp: Option<&FirstPerson>,
        view_to_world: Mat4,
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
        let root = view_to_world * hand_root_matrix(fp, view, partial);
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
                let model = self.arm_model(fp.slim);
                // Vanilla's `ModelPart` scales its pivot by 1/16 (the model is in pixels), and
                // `AvatarRenderer.renderHand` gives the right arm its 0.1 rad z tilt.
                let m = root * arm_matrix(fp, partial) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
                emit_part(
                    &mut out,
                    &model,
                    &model.root,
                    &PoseOverrides::default(),
                    m,
                    rect,
                    light,
                    0.0,
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
            let display = match display {
                ItemDisplay::FirstPerson => &part.firstperson,
                ItemDisplay::Fixed => &part.fixed,
                ItemDisplay::Ground => &part.ground,
            };
            let m = root * item_display_matrix(part, display);
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
pub(crate) fn arm_model(slim: bool) -> CubeModel {
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
            pivot: Vec3::new(-5.0, 2.0, 0.0),
            // `AvatarRenderer.renderHand`: `rightArm.zRot = 0.1`.
            rotation: Vec3::new(0.0, 0.0, 0.1),
            visible: true,
            cubes: vec![cube([40, 16], 0.0)],
            children: vec![ModelPart {
                name: "right_sleeve".into(),
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: vec![cube([40, 32], 0.25)],
                children: vec![],
            }],
        },
    }
}

/// A held item's model matrix: its `display.firstperson_righthand` transform and composite
/// `transformation`, then the recentring vanilla's `ItemTransform.apply` ends with
/// (the model's unit cube is drawn around the origin).
pub(crate) fn item_display_matrix(
    part: &haste_assets::ItemModelPart,
    display: &haste_assets::DisplayTransform,
) -> Mat4 {
    crate::gui::display_matrix(display)
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

/// `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`'s outer transforms, in view space:
/// `GameRenderer.bobView`'s walk bob, then the hand's lag behind the camera turn
/// (`(viewXRot - xBob) * 0.1`, `(viewYRot - yBob) * 0.1`).
pub(crate) fn hand_root_matrix(fp: &FirstPerson, view: (f32, f32), partial: f32) -> Mat4 {
    // `ClientAvatarState.getBackwardsInterpolatedWalkDistance`.
    let walk = fp.walk_dist;
    let backwards = -(walk.at(partial) + (walk.end - walk.start) * partial);
    let bob = fp.bob.at(partial);
    // `GameRenderer.bobView`: x right, y up, the camera at the origin.
    let s = (backwards * std::f32::consts::PI).sin() * bob;
    let c = (backwards * std::f32::consts::PI).cos() * bob;
    let roll_x = ((backwards * std::f32::consts::PI - 0.2).cos() * bob).abs();
    let bob_matrix = Mat4::from_translation(Vec3::new(s * 0.5, -c.abs(), 0.0))
        * Mat4::from_rotation_z((s * 3.0).to_radians())
        * Mat4::from_rotation_x((roll_x * 5.0).to_radians());
    bob_matrix
        * Mat4::from_rotation_x(((view.1 - fp.x_bob.at(partial)) * 0.1).to_radians())
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
fn push_world_quad(
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
fn entity_shade(n: Vec3) -> f32 {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    ((n.dot(l0).max(0.0) + n.dot(l1).max(0.0)) * 0.6 + 0.4).min(1.0)
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

fn pose_rotation(pose: &PoseOverrides, name: &str) -> Option<Vec3> {
    pose.rotations
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| *r)
}

/// Child pivots are relative to the parent (vanilla `PartPose`); rotation is Z * Y * X around
/// the pivot, pose overrides add to the base rotation. `hurt` (0..1) tints the part red.
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
    if !part.visible || pose.hidden.iter().any(|h| h == &part.name) {
        return;
    }
    let rot = part.rotation + pose_rotation(pose, &part.name).unwrap_or(Vec3::ZERO);
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
            let color = [r, g, b, 1.0];
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
        emit_part(out, model, child, pose, m, rect, light, hurt);
    }
}

/// The model's bounding box in the space `emit_item` draws it through the ground display
/// (block units, after the -0.5 centring `ItemTransform.apply` ends with): the transformed
/// element or extruded-sprite positions. Vanilla's `ItemStackRenderState.getModelBoundingBox`
/// (which drives `ItemEntityRenderer.submit`'s lift and the stack-copy layout) computes
/// exactly this; a model with no geometry gets vanilla's empty box (a point at the origin).
fn item_model_bbox(model: &haste_assets::ItemModel) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    let mut see = |p: Vec3| {
        min = min.min(p);
        max = max.max(p);
        any = true;
    };
    for part in &model.parts {
        let m = crate::gui::display_matrix(&part.ground)
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

/// The missing item model vanilla's `ModelBakery.MissingModels` bakes (`MissingCuboidModel`):
/// a unit cube with the missingno texture and no display transforms, so it draws centred on
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
                    ground: haste_assets::DisplayTransform::IDENTITY,
                    fixed: haste_assets::DisplayTransform::IDENTITY,
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
    fn default_skin_follows_the_uuid_hash() {
        // hashCode of the all-zero UUID is 0 -> first slim skin.
        assert_eq!(
            default_skin(0),
            (Identifier::minecraft("entity/player/slim/alex"), true)
        );
        // hashCode 9 -> first wide skin.
        assert_eq!(
            default_skin(9),
            (Identifier::minecraft("entity/player/wide/alex"), false)
        );
        // Negative hash codes wrap with floorMod: hash -1 -> 17 -> wide zuri.
        let uuid = 0xFFFF_FFFFu128; // lo = 0xFFFFFFFF: (0 ^ -1) = -1
        assert_eq!(default_skin(uuid).0.path(), "entity/player/wide/zuri");
    }

    #[test]
    fn a_cube_has_six_outward_ccw_faces() {
        let model = CubeModel {
            texture: Identifier::minecraft("x"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
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
        emit_part(
            &mut out,
            &model,
            &model.root,
            &PoseOverrides::default(),
            root,
            [0, 0, 64, 64],
            (15, 15),
            0.0,
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
            item: None,
            light: (15, 0),
            attack: TickPair::fixed(0.0),
            equip: TickPair::fixed(1.0),
            x_bob: TickPair::fixed(0.0),
            y_bob: TickPair::fixed(0.0),
            bob: TickPair::fixed(0.0),
            walk_dist: TickPair::fixed(0.0),
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
        let model = arm_model(false);
        let mut out = Vec::new();
        emit_part(
            &mut out,
            &model,
            &model.root,
            &PoseOverrides::default(),
            m,
            [0, 0, 64, 64],
            (15, 15),
            0.0,
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
        emit_part(
            &mut out2,
            &arm_model(false),
            &arm_model(false).root,
            &PoseOverrides::default(),
            swung,
            [0, 0, 64, 64],
            (15, 15),
            0.0,
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
        for (attack, equip) in [(0.0, 1.0), (0.25, 1.0), (0.5, 0.5), (1.0, 0.0)] {
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
        for (attack, equip) in [(0.0, 1.0), (0.25, 1.0), (0.5, 0.4), (1.0, 0.0)] {
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
    fn the_walk_bob_moves_and_rolls_the_hand() {
        // `GameRenderer.bobView` over a walk cycle that walked 0.75 blocks this tick (0.6 the
        // tick before) with the bob amplitude at 0.1. The phase is the backwards interpolated
        // walk distance: -(walkDist + (walkDist - walkDistO) * partial) = -0.9 at the end of the
        // tick, so the translation is (sin(-0.9PI) * 0.1 * 0.5, -|cos(-0.9PI) * 0.1|, 0) =
        // (-0.0154508, -0.0951057, 0), with a 3 deg roll and a 5 deg tilt from the same phase.
        let mut h = still_hand();
        h.walk_dist = TickPair {
            start: 0.6,
            end: 0.75,
        };
        h.bob = TickPair::fixed(0.1);
        let pi = std::f32::consts::PI;
        // The phase is the backwards interpolated walk distance, extrapolated by one tick.
        let phase = -(0.75 + (0.75 - 0.6) * 1.0);
        let m = hand_root_matrix(&h, (0.0, 0.0), 1.0);
        let t = m.transform_point3(Vec3::ZERO);
        assert!((t.x - -0.015450852).abs() < 1e-6, "{}", t.x);
        assert!((t.y - -0.09510567).abs() < 1e-6, "{}", t.y);
        assert_eq!(t.z, 0.0);
        // The roll (Z) and the tilt (X) are rotations from the same phase, so the hand's up axis
        // leans by them: sin(-0.9PI) * 0.1 * 3 = -0.0927 deg and |cos(-0.9PI - 0.2) * 0.1| * 5.
        let rolled = m.transform_vector3(Vec3::Y);
        let roll_deg = (phase * pi).sin() * 0.1 * 3.0;
        let tilt_deg = ((phase * pi - 0.2).cos() * 0.1).abs() * 5.0;
        assert!(
            (rolled.x + roll_deg.to_radians().sin()).abs() < 1e-6,
            "{rolled:?} (roll {roll_deg} deg)"
        );
        assert!(
            (rolled.z - tilt_deg.to_radians().sin()).abs() < 1e-6,
            "{rolled:?}"
        );
        // Half a tick later the phase is -0.825, so the hand is somewhere else.
        let mid = hand_root_matrix(&h, (0.0, 0.0), 0.5).transform_point3(Vec3::ZERO);
        assert!((mid - t).length() > 0.01, "{mid:?} vs {t:?}");
        // Standing still: no bob, so no walk bob either.
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
        let wide = arm_model(false);
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
        let slim = arm_model(true);
        assert_eq!(slim.root.cubes[0].origin, Vec3::new(-2.0, -2.0, -2.0));
        assert_eq!(slim.root.cubes[0].size, Vec3::new(3.0, 12.0, 4.0));
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
