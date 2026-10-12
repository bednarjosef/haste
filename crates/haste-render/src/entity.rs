// Entity drawing: every `EntityInstance` posed on the CPU into one world-space vertex buffer over
// one lazily filled entity atlas, drawn in one call inside the main pass (depth-tested, before
// translucent terrain). Cube UVs follow vanilla's public box-UV skin layout (minecraft.wiki
// "Skin": head top at (8,0), front at (8,8), right side at (0,8), ...).

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use haste_assets::Assets;
use haste_core::model::{CubeModel, EntityInstance, ModelPart, PoseOverrides};
use haste_core::{EntityId, Identifier};

use crate::gui::Shelf;

const ATLAS_SIZE: u32 = 2048;

/// Vanilla renders the player model at 15/16 scale.
const PLAYER_SCALE: f32 = 0.9375;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EntityVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub rect: [u16; 4],
}

impl EntityVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Uint16x4
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
    pub drawn: u32,
}

impl EntityRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
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
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
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
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = 8192;
        let vertex_buffer = make_vertex_buffer(&device, vertex_capacity);
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
                placeholder_texture()
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
            let (texture, scale) = texture_and_scale(e);
            let rect = self.texture_rect(&texture, assets);
            let (sky, block) = self.lights.get(&e.id).copied().unwrap_or((15, 0));
            let light = lightmap(sky, block);
            let root = Mat4::from_translation((e.pos - camera).as_vec3())
                * Mat4::from_rotation_y((180.0 - e.yaw).to_radians())
                * Mat4::from_scale(Vec3::new(-scale, -scale, scale) / 16.0);
            let start = self.vertices.len();
            emit_part(
                &mut self.vertices,
                &e.model,
                &e.model.root,
                &e.pose,
                root,
                rect,
                light,
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

/// Light level brightness, the same curve as the terrain shader (`chunk.wgsl::lightmap`).
fn lightmap(sky: u8, block: u8) -> f32 {
    let f = |l: u8| (l.min(15) as f32 / 15.0).powf(1.55) * 0.95 + 0.05;
    f(sky).max(f(block))
}

/// World-space entity shading: two directional lights at 0.6 and 0.4 ambient (the same
/// lighting model as GUI items, with the light directions in world space).
fn entity_shade(n: Vec3) -> f32 {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    ((n.dot(l0).max(0.0) + n.dot(l1).max(0.0)) * 0.6 + 0.4).min(1.0)
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
/// the pivot, pose overrides add to the base rotation.
pub(crate) fn emit_part(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    part: &ModelPart,
    pose: &PoseOverrides,
    parent: Mat4,
    rect: [u32; 4],
    light: f32,
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
            let s = entity_shade(nw) * light;
            let color = [s, s, s, 1.0];
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
                });
            }
        }
    }
    for child in &part.children {
        emit_part(out, model, child, pose, m, rect, light);
    }
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
            1.0,
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
}
