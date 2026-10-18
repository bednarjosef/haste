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

mod atlas;
mod block_entities;
mod cape;
mod equipment;
mod item;
mod item_frame;
mod painting;
mod pipelines;
mod piston;
mod shadow;
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

/// The vertex range the upload after `append_block_entities` must cover: only the tail
/// past `start` (the entities' vertices were already uploaded by `prepare`), or everything
/// when the buffer had to grow — a fresh buffer holds no head, so it is rewritten in full.
/// An empty range means nothing was appended and no upload happens at all.
pub(crate) fn append_upload_range(
    len: usize,
    capacity: usize,
    start: usize,
) -> std::ops::Range<usize> {
    if len > capacity {
        0..len
    } else {
        start..len
    }
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
    /// Item registry names by protocol id (for the held item).
    pub fn set_item_names(&mut self, names: Vec<Identifier>) {
        self.item_names = names;
        self.item_models.clear();
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
