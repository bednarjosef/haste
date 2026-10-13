//! Server-sent particles (`level_particles`), first version and deliberately approximate:
//! a fixed-capacity pool simulated at 20 Hz on the game thread ([`Particles`]), drawn as
//! camera-facing quads textured from the particle atlas in one batched draw inside the main
//! pass ([`ParticleRenderer`], reusing the entity shader: sprite rects by textureLoad,
//! vertex light colour, fog).
//!
//! Approximations (per the brief; all later steps):
//! - Motion is generic: velocity, gravity (-0.04 blocks/tick²), drag (0.98/tick), a random
//!   20-40 tick lifetime fading out over the last 4 ticks. Per-type vanilla behaviour (flame
//!   rising, smoke drifting) is not modelled.
//! - `level_particles` semantics (minecraft.wiki "Java Edition protocol/Packets" >
//!   Particle): count 0 spawns one particle at the position with velocity
//!   `offset * max_speed`; count > 0 spawns that many at `pos + gaussian * offset` with
//!   velocity `gaussian * max_speed`. The `randomization` mode and the type-specific data
//!   (`Particle::data`: block states, dust colours) are ignored.
//! - Simulation runs on the game thread at 20 Hz; the renderer interpolates positions per
//!   frame (sprite choice and fade step at 20 Hz, like vanilla's per-tick particle ticks).
//! - Quad size follows the sprite's width (about 0.2 blocks for an 8 px sprite), not
//!   vanilla's per-type `quadSize`.
//! - Unknown particle types (mods, or vanilla types without atlas sprites like block/item
//!   particles) are skipped, logged once per type.

use std::sync::Arc;

use anyhow::Result;
use glam::{DVec3, Vec3};
use haste_assets::{Assets, ParticleAtlas};
use haste_core::view::BlockView;
use haste_core::{BlockPos, Identifier};

use crate::entity::EntityVertex;
use crate::renderer::Camera;

/// Pool capacity (vanilla caps its engine around 16384 with a distance squash we do not
/// model; extra spawns are dropped).
pub const MAX_PARTICLES: usize = 4096;

/// Gravity per tick (vanilla's generic particle gravity, blocks/tick²).
const GRAVITY: f32 = 0.04;
/// Velocity retained per tick (vanilla's generic particle friction).
const DRAG: f32 = 0.98;
/// Sky-light value meaning "not sampled yet" (resampled on the first frame).
const UNSAMPLED: u8 = 255;

/// A particle spawn request from `level_particles`: the resolved type name and the packet's
/// spawn parameters. The game thread queues these, the shared pool consumes them.
#[derive(Clone, Debug)]
pub struct ParticleSpawn {
    /// Particle type (`minecraft:flame`).
    pub kind: Identifier,
    /// Spawn position.
    pub pos: DVec3,
    /// Offset per axis (spread box for count > 0, direction for count 0).
    pub offset: [f32; 3],
    /// Max speed per axis.
    pub max_speed: [f32; 3],
    /// Count (0: one particle at `pos` with velocity `offset * max_speed`).
    pub count: i32,
}

/// The sprite list of one particle type: rectangles in the particle atlas, in file order.
struct ParticleType {
    name: Identifier,
    sprites: Vec<[u16; 4]>,
}

/// Every particle type that has a particle json (name -> atlas sprite rectangles), plus the
/// packed atlas pixels. Built once from the registries' particle type ids; the game thread
/// resolves spawn names against it, the renderer uploads its pixels as one texture.
pub struct ParticleTypes {
    types: Vec<ParticleType>,
    by_name: ahash::HashMap<Identifier, u32>,
    pixels: Vec<u8>,
    size: u32,
}

impl ParticleTypes {
    /// Packs the sprites of `ids` into one atlas (types without a definition or with no
    /// placed sprites contribute nothing: block/item particles are drawn from block/item
    /// textures in vanilla, none of that here).
    pub fn build(assets: &mut Assets, ids: &[Identifier]) -> Result<ParticleTypes> {
        let atlas = ParticleAtlas::build(assets, ids)?;
        let mut types = Vec::new();
        for id in ids {
            let sprites: Vec<[u16; 4]> = atlas
                .sprites_of(id)
                .iter()
                .map(|r| r.pixel_rect.map(|v| v as u16))
                .collect();
            if !sprites.is_empty() {
                types.push(ParticleType {
                    name: id.clone(),
                    sprites,
                });
            }
        }
        let by_name = types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i as u32))
            .collect();
        Ok(ParticleTypes {
            types,
            by_name,
            pixels: atlas.pixels().to_vec(),
            size: atlas.size(),
        })
    }

    /// Index of a particle type, `None` when it has no atlas sprites.
    fn index(&self, name: &Identifier) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    fn sprites(&self, i: u32) -> &[[u16; 4]] {
        &self.types[i as usize].sprites
    }

    /// The packed atlas RGBA pixels and its side length (for the texture upload).
    pub fn atlas(&self) -> (&[u8], u32) {
        (&self.pixels, self.size)
    }
}

/// One live particle (a pool slot). Positions are world-space f32; the renderer subtracts
/// the camera in f64 before narrowing.
struct ParticleState {
    /// Index into `ParticleTypes::types`.
    kind: u32,
    pos: Vec3,
    prev_pos: Vec3,
    vel: Vec3,
    /// Age and lifetime in ticks.
    age: f32,
    lifetime: f32,
    /// Block whose light is cached (resampled when the particle leaves it).
    light_at: BlockPos,
    sky_light: u8,
    block_light: u8,
}

/// The particle pool (game state, no GPU): the game thread spawns into it from
/// `level_particles` and ticks it at 20 Hz; the render thread reads it each frame (short
/// lock). Fixed capacity, no allocation per particle or per frame after the pool warms up.
pub struct Particles {
    types: Option<Arc<ParticleTypes>>,
    /// Alive particles; expiry swaps the last in, so this stays compact.
    pool: Vec<ParticleState>,
    /// Types logged as unknown already (log once per type).
    unknown: Vec<Identifier>,
    rng: Rng,
}

impl Particles {
    /// An empty pool drawing types from `types`.
    pub fn new(types: Arc<ParticleTypes>) -> Particles {
        Particles {
            types: Some(types),
            pool: Vec::new(),
            unknown: Vec::new(),
            rng: Rng(0x853C_49E6_748F_EA9B),
        }
    }

    /// Number of live particles.
    pub fn alive(&self) -> usize {
        self.pool.len()
    }

    /// Drops every particle (dimension change).
    pub fn clear(&mut self) {
        self.pool.clear();
    }

    /// Spawns per the packet's semantics (see the module docs). Unknown types are skipped,
    /// logged once per type.
    pub fn spawn(&mut self, s: &ParticleSpawn) {
        let Some(types) = self.types.as_deref() else {
            return;
        };
        let Some(kind) = types.index(&s.kind) else {
            if !self.unknown.contains(&s.kind) {
                self.unknown.push(s.kind.clone());
                tracing::warn!("particle type {} has no atlas sprites; skipped", s.kind);
            }
            return;
        };
        if s.count <= 0 {
            // One particle at the position, velocity = offset * max speed.
            let vel = Vec3::from(s.offset) * Vec3::from(s.max_speed);
            self.push(kind, s.pos, vel);
        } else {
            for _ in 0..s.count {
                if self.pool.len() >= MAX_PARTICLES {
                    break;
                }
                let pos = DVec3::new(
                    s.pos.x + (self.rng.gaussian() * s.offset[0]) as f64,
                    s.pos.y + (self.rng.gaussian() * s.offset[1]) as f64,
                    s.pos.z + (self.rng.gaussian() * s.offset[2]) as f64,
                );
                let vel = Vec3::new(
                    self.rng.gaussian() * s.max_speed[0],
                    self.rng.gaussian() * s.max_speed[1],
                    self.rng.gaussian() * s.max_speed[2],
                );
                self.push(kind, pos, vel);
            }
        }
    }

    fn push(&mut self, kind: u32, pos: DVec3, vel: Vec3) {
        if self.pool.len() >= MAX_PARTICLES {
            return;
        }
        let pos32 = pos.as_vec3();
        self.pool.push(ParticleState {
            kind,
            pos: pos32,
            prev_pos: pos32,
            vel,
            age: 0.0,
            lifetime: 20.0 + self.rng.next_f32() * 20.0,
            light_at: BlockPos::new(
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            ),
            sky_light: UNSAMPLED,
            block_light: 0,
        });
    }

    /// One 20 Hz step: expire, then move (generic gravity and drag). The renderer
    /// interpolates between `prev_pos` and `pos` within the tick.
    pub fn tick(&mut self) {
        let mut i = 0;
        while i < self.pool.len() {
            let p = &mut self.pool[i];
            p.age += 1.0;
            if p.age >= p.lifetime {
                // Swap the last alive particle in and retest this slot.
                let last = self.pool.len() - 1;
                self.pool.swap(i, last);
                self.pool.truncate(last);
                continue;
            }
            p.prev_pos = p.pos;
            p.vel.y -= GRAVITY;
            p.pos += p.vel;
            p.vel *= DRAG;
            i += 1;
        }
    }
}

/// xorshift64*, deterministically seeded (tests rely on the repetition).
struct Rng(u64);

impl Rng {
    /// Uniform f32 in [0, 1).
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 16_777_216.0
    }

    /// Approximate standard gaussian: the sum of three uniforms in (-1, 1) (variance 1,
    /// range ±3), standing in for vanilla's `Random.nextGaussian`.
    fn gaussian(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0 + (self.next_f32() * 2.0 - 1.0) + (self.next_f32() * 2.0 - 1.0)
    }
}

/// Draws the pool: camera-facing quads over the particle atlas, one draw call for all
/// particles. The vertex/pipeline shape is the entity renderer's (same shader, textureLoad
/// sprite rects, vertex light colour, fog); particles differ in culling (billboards face
/// the camera) and depth (blended, no depth write).
pub struct ParticleRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
    vertex_buffer: wgpu::Buffer,
    capacity: usize,
    vertices: Vec<EntityVertex>,
    /// Frames since the last debug log of the drawn count.
    log_clock: u32,
    pub drawn: u32,
}

impl ParticleRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
    ) -> ParticleRenderer {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle-layout"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle"),
            source: wgpu::ShaderSource::Wgsl(include_str!("entity.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle-pipeline-layout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle"),
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
                // Billboards are seen from whichever side faces the camera.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let capacity = 4096;
        let vertex_buffer = make_vertex_buffer(&device, capacity);
        ParticleRenderer {
            device,
            queue,
            pipeline,
            layout,
            bind_group: None,
            vertex_buffer,
            capacity,
            vertices: Vec::new(),
            log_clock: 0,
            drawn: 0,
        }
    }

    /// Uploads the atlas of `types` as the particle texture (one texture for all types).
    pub fn set_types(&mut self, types: &Arc<ParticleTypes>) {
        let (pixels, size) = types.atlas();
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle-atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle-bind"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            }],
        }));
    }

    /// Poses every particle as a camera-facing quad into the vertex buffer (positions
    /// interpolated within the tick; light resampled when a particle leaves its block).
    /// The camera bind group of the main pass is at group 0; roll is ignored (billboards
    /// do not tilt with the hurt tilt).
    pub fn prepare(
        &mut self,
        particles: &mut Particles,
        camera: Camera,
        partial: f32,
        world: &dyn BlockView,
    ) {
        self.vertices.clear();
        self.drawn = 0;
        // Debug visibility: how many particles the frame poses (once a second).
        self.log_clock += 1;
        let log = self.log_clock.is_multiple_of(60);
        let types = particles.types.clone();
        let Some(types) = types.as_deref() else {
            return;
        };
        let (right, up) = billboard_axes(camera.yaw, camera.pitch);
        for p in &mut particles.pool {
            let pos = p.prev_pos.lerp(p.pos, partial);
            // Light at the particle's position, resampled when it crosses a block boundary.
            let block = BlockPos::new(
                pos.x.floor() as i32,
                pos.y.floor() as i32,
                pos.z.floor() as i32,
            );
            if p.sky_light == UNSAMPLED || block != p.light_at {
                p.light_at = block;
                p.sky_light = world.sky_light(block);
                p.block_light = world.block_light(block);
            }
            let sprites = types.sprites(p.kind);
            if sprites.is_empty() {
                continue;
            }
            // The sprite for the particle's age (multi-sprite types animate over the list).
            let index = ((p.age / p.lifetime) * sprites.len() as f32).floor() as usize;
            let rect = sprites[index.min(sprites.len() - 1)];
            // Fade out over the last 4 ticks.
            let alpha = ((p.lifetime - p.age) / 4.0).clamp(0.0, 1.0);
            let light = crate::entity::lightmap(p.sky_light, p.block_light);
            let color = [light, light, light, alpha];
            let rel = (pos.as_dvec3() - camera.pos).as_vec3();
            // About 0.2 blocks across for an 8 px sprite (vanilla's generic quad size).
            let half = rect[2] as f32 / 8.0 * 0.1;
            let r = right * half;
            let u = up * half;
            let corners = [rel - r - u, rel + r - u, rel + r + u, rel - r + u];
            let (x, y, w, h) = (
                rect[0] as f32,
                rect[1] as f32,
                rect[2] as f32,
                rect[3] as f32,
            );
            let uvs = [[x, y + h], [x + w, y + h], [x + w, y], [x, y]];
            let rect16 = rect;
            for i in [0usize, 1, 2, 0, 2, 3] {
                self.vertices.push(EntityVertex {
                    pos: corners[i].to_array(),
                    uv: uvs[i],
                    color,
                    rect: rect16,
                });
            }
            self.drawn += 1;
        }
        if log && (self.drawn > 0 || particles.alive() > 0) {
            tracing::debug!(
                "particles: {} drawn of {} alive, {} vertices",
                self.drawn,
                particles.alive(),
                self.vertices.len()
            );
        }
        if self.vertices.len() > self.capacity {
            self.capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = make_vertex_buffer(&self.device, self.capacity);
        }
        if !self.vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }

    /// Draws into the main pass after the entities (camera bind group must be set at
    /// group 0). One draw call for every particle.
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        let Some(bind) = &self.bind_group else {
            return 0;
        };
        if self.vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, bind, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertices.len() as u32, 0..1);
        1
    }
}

fn make_vertex_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle-vertices"),
        size: (capacity * std::mem::size_of::<EntityVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Billboard axes for a camera look (yaw/pitch in degrees, Minecraft's convention):
/// screen-right horizontal, up perpendicular to the look. Never degenerate (right and look
/// are orthogonal unit vectors).
fn billboard_axes(yaw: f32, pitch: f32) -> (Vec3, Vec3) {
    let yaw = yaw.to_radians();
    let pitch = pitch.to_radians();
    let look = Vec3::new(
        -pitch.cos() * yaw.sin(),
        -pitch.sin(),
        pitch.cos() * yaw.cos(),
    );
    let right = Vec3::new(-yaw.cos(), 0.0, -yaw.sin());
    let up = right.cross(look).normalize();
    (right, up)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A type table with hand-written sprite lists (no assets needed).
    fn types_for_tests(entries: &[(&str, &[[u16; 4]])]) -> Arc<ParticleTypes> {
        let mut types = ParticleTypes {
            types: Vec::new(),
            by_name: Default::default(),
            pixels: Vec::new(),
            size: 0,
        };
        for (name, sprites) in entries {
            types.types.push(ParticleType {
                name: Identifier::minecraft(name),
                sprites: sprites.to_vec(),
            });
            types
                .by_name
                .insert(Identifier::minecraft(name), (types.types.len() - 1) as u32);
        }
        Arc::new(types)
    }

    fn spawn_request(kind: &str, count: i32) -> ParticleSpawn {
        ParticleSpawn {
            kind: Identifier::minecraft(kind),
            pos: DVec3::new(10.0, 64.0, -5.0),
            offset: [0.5, 0.0, 0.25],
            max_speed: [0.1, 0.0, 0.05],
            count,
        }
    }

    #[test]
    fn count_zero_spawns_one_at_the_position_with_offset_velocity() {
        let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
        particles.spawn(&spawn_request("flame", 0));
        assert_eq!(particles.alive(), 1);
        let p = &particles.pool[0];
        assert_eq!(p.pos, Vec3::new(10.0, 64.0, -5.0));
        // velocity = offset * max speed, per component.
        assert_eq!(p.vel, Vec3::new(0.05, 0.0, 0.0125));
        assert_eq!(p.prev_pos, p.pos);
    }

    #[test]
    fn count_n_spreads_within_three_sigmas() {
        let mut particles = Particles::new(types_for_tests(&[("smoke", &[[0, 0, 4, 4]])]));
        particles.spawn(&spawn_request("smoke", 50));
        assert_eq!(particles.alive(), 50);
        for p in &particles.pool {
            // The gaussian approximations stay within ±3 sigmas.
            for (a, o) in p.pos.to_array().iter().zip([10.0, 64.0, -5.0]) {
                assert!((a - o).abs() <= 3.0 * 0.5 + 1e-5, "pos {:?}", p.pos);
            }
            for (v, m) in p.vel.to_array().iter().zip([0.1, 0.0, 0.05]) {
                assert!(v.abs() <= 3.0 * m + 1e-5, "vel {:?}", p.vel);
            }
            assert!(
                (20.0..=40.0).contains(&p.lifetime),
                "lifetime {}",
                p.lifetime
            );
        }
    }

    #[test]
    fn pool_caps_at_max_particles() {
        let mut particles = Particles::new(types_for_tests(&[("smoke", &[[0, 0, 4, 4]])]));
        particles.spawn(&spawn_request("smoke", (MAX_PARTICLES * 2) as i32));
        assert_eq!(particles.alive(), MAX_PARTICLES);
    }

    #[test]
    fn particles_expire_within_their_lifetime() {
        let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
        particles.spawn(&spawn_request("flame", 0));
        let lifetime = particles.pool[0].lifetime.ceil() as usize;
        // Alive through the lifetime (min 20 ticks)...
        for _ in 0..lifetime.saturating_sub(1) {
            particles.tick();
        }
        assert_eq!(particles.alive(), 1);
        // ...expired one tick later.
        particles.tick();
        assert_eq!(particles.alive(), 0);
    }

    #[test]
    fn unknown_types_are_skipped_and_logged_once() {
        let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
        particles.spawn(&spawn_request("glow", 10));
        assert_eq!(particles.alive(), 0);
        assert_eq!(particles.unknown.len(), 1);
        // The same unknown type does not log a second time.
        particles.spawn(&spawn_request("glow", 10));
        assert_eq!(particles.unknown.len(), 1);
        // Types with an empty sprite list count as unknown too.
        particles.spawn(&spawn_request("flame", 1));
        assert_eq!(particles.alive(), 1);
    }

    #[test]
    fn tick_moves_with_gravity_and_drag() {
        let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
        let mut request = spawn_request("flame", 0);
        request.offset = [0.0, 1.0, 0.0];
        request.max_speed = [0.0, 1.0, 0.0];
        particles.spawn(&request);
        let vel = particles.pool[0].vel;
        particles.tick();
        let p = &particles.pool[0];
        assert_eq!(p.prev_pos, Vec3::new(10.0, 64.0, -5.0));
        // Moved by the velocity after gravity; the velocity then lost drag.
        let moved = vel - Vec3::Y * GRAVITY;
        assert_eq!(p.pos, p.prev_pos + moved);
        assert_eq!(p.vel, moved * DRAG);
    }

    #[test]
    fn gaussian_is_bounded() {
        let mut rng = Rng(0x853C_49E6_748F_EA9B);
        for _ in 0..1000 {
            let g = rng.gaussian();
            assert!((-3.0..=3.0).contains(&g), "{g}");
        }
        let mut rng = Rng(1);
        for _ in 0..1000 {
            let g = rng.gaussian();
            assert!((-3.0..=3.0).contains(&g), "{g}");
        }
    }

    #[test]
    fn billboard_axes_stay_orthogonal_at_all_pitches() {
        for pitch in [-90.0, -45.0, 0.0, 45.0, 90.0] {
            let (right, up) = billboard_axes(30.0, pitch);
            assert!(right.is_normalized(), "{right:?}");
            assert!(up.is_normalized(), "{up:?}");
            assert!(right.cross(up).length() > 0.99);
        }
    }
}
