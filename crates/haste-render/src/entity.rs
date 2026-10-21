// Entity drawing: every `EntityInstance` posed on the CPU into one world-space vertex buffer over
// one lazily filled entity atlas, drawn in one call inside the main pass (depth-tested, before
// translucent terrain). Cube UVs follow vanilla's public box-UV skin layout (minecraft.wiki
// "Skin": head top at (8,0), front at (8,8), right side at (0,8), ...).

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use haste_assets::Assets;
use haste_core::model::{
    ArrowDecoration, BodyOp, BodyTransform, CubeModel, Decoration, DroppedItem, EntityInstance,
    FallingBlockDecoration, ItemFrameDecoration, ModelPart, PaintingDecoration, PoseOverrides,
    PrimedTntDecoration, ThrownItemDecoration,
};
use haste_core::parts::{self, PartId};
use haste_core::view::BlockView;
use haste_core::{BlockPos, EntityId, Identifier};

use crate::gui::Shelf;

mod arrow;
mod atlas;
mod block_entities;
mod block_model;
mod boat;
mod cape;
mod dragon_fireball;
mod equipment;
#[cfg(test)]
mod equipment_tests;
mod experience_orb;
mod falling_block;
mod first_person;
mod flame;
#[cfg(test)]
mod flame_tests;
mod item;
mod item_entity;
mod item_frame;
pub mod map;
mod map_color;
#[cfg(test)]
mod map_color_tests;
mod map_decoration_types;
#[cfg(test)]
mod map_tests;
mod minecart;
#[cfg(test)]
mod minecart_tests;
mod painting;
mod pipelines;
mod piston;
mod shadow;
mod sign;
#[cfg(test)]
mod tests;
mod thrown_item;
#[cfg(test)]
mod thrown_item_tests;
mod tnt;

mod armor_stand;
mod hand;
mod humanoid;
mod item_display;
mod model_parts;
mod vertex;

pub(crate) use hand::{arm_matrix, custom_head_layer_matrix, held_item_layer_matrix};
#[cfg(test)]
use hand::{arm_model, hand_root_matrix, item_arm_matrix};
pub(crate) use humanoid::{elytra_model, humanoid_armor};
use humanoid::{missing_item_model, missing_texture, placeholder_texture};
use item_display::item_model_bbox;
pub(crate) use item_display::{
    argb_linear, item_display_matrix, item_frame_item_transform, item_frame_transform,
    painting_transform,
};
pub use map::{MapData, MapMarker};
pub(crate) use model_parts::{
    aabb_in_frustum, emit_part, emit_part_tinted, model_extents, model_top, part_matrix,
    pose_without_hidden,
};
#[cfg(test)]
use vertex::HURT_RED_STRENGTH;
pub(crate) use vertex::{
    entity_shade, hand_normal_rotation, hurt_color, max_local_raw_brightness, push_flat_quad,
    push_world_quad,
};
pub use vertex::{lightmap_brightness, name_tag_visible, shadow_piece_alpha, EntityVertex};
use vertex::{make_vertex_buffer, parse_tex, push_shadow_quad};

const ATLAS_SIZE: u32 = 2048;
/// Vanilla `Entity.shouldRenderAtSqrDistance`: whether an entity whose model is `size`
/// blocks and has entity `scale` should be drawn at squared distance `dist_sq`, given the
/// `view_scale` (`Entity.viewScale`, from `options.entityDistanceScaling`).
pub fn should_render_at_sqr_distance(
    size: f32,
    entity_scale: f32,
    dist_sq: f64,
    view_scale: f32,
) -> bool {
    let size = if size.is_nan() { 1.0 } else { size };
    let limit = 64.0 * size.max(1.0) * entity_scale * view_scale;
    dist_sq < f64::from(limit * limit)
}

pub(crate) use item_entity::{
    item_entity_bob, item_entity_copies, item_entity_lift, item_entity_spin,
};

/// Vanilla renders the player model at 15/16 scale.
const PLAYER_SCALE: f32 = 0.9375;

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

/// What the first-person view holds (vanilla `FirstPersonHandsAndItemsRenderState`): see
/// [`first_person`](self::first_person).
pub use first_person::FirstPerson;

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
    pub nametags: Vec<Nametag>,
    hand_buffer: wgpu::Buffer,
    hand_capacity: usize,
    hand: Vec<EntityVertex>,
    item_names: Vec<Identifier>,
    item_models: HashMap<u32, Option<Arc<haste_assets::ItemModel>>>,
    /// The first-person arm models, by left, [`FirstPerson::slim`] and
    /// [`FirstPerson::sleeve`] (built once on first use; the left arm serves the two-handed
    /// map pose).
    arm_models: [Option<Arc<CubeModel>>; 8],
    /// The item frame block model quads, by `(glow, map)`, built once on first use.
    frame_models: HashMap<(bool, bool), Arc<Vec<haste_assets::BakedQuad>>>,
    /// The moving pistons' moved-block quads, by the state string (resolved once per state a
    /// piston ever moves).
    piston_quads: ahash::AHashMap<String, Option<Arc<Vec<crate::blockentity::PistonQuad>>>>,
    /// The terrain's baked block models and its atlas: the moving-block path
    /// (`FallingBlockRenderer`, `TntRenderer` — see `entity/block_model.rs`) draws block
    /// states through them, textures resolved into the entity atlas like the piston's.
    baked: Arc<haste_assets::BakedBlockModels>,
    terrain_atlas: Arc<haste_assets::AtlasBuilder>,
    /// The minecarts' display-block geometry, by state id (resolved once per state a cart
    /// ever shows; vanilla's `BlockModelResolver` caches the same way).
    minecart_quads: ahash::AHashMap<u32, Option<Arc<minecart::DisplayGeometry>>>,
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
    view_scale: f32,
    /// The boats' water patches (vanilla `water_mask`): depth-only triangles that cull the
    /// water inside a floating hull ([`boat`](self::boat)).
    mask_vertices: Vec<crate::entity::boat::MaskVertex>,
    mask_buffer: wgpu::Buffer,
    mask_capacity: usize,
    water_mask_pipeline: wgpu::RenderPipeline,
    /// The beam and portal pipelines (see [`crate::blockentity::BeamPipelines`]).
    pub beams: crate::blockentity::BeamPipelines,
    /// The default font, for world-space sign text (glyph pages land in the entity atlas).
    font: Arc<haste_assets::BitmapFont>,
    /// Equipment asset info (`equipment/<asset>.json`), by asset id, loaded once (`None` for an
    /// asset that failed to load, so it is not retried every frame).
    equipment_infos: HashMap<Identifier, Option<Arc<haste_assets::EquipmentInfo>>>,
    /// The maps to draw (`MapTextureManager`'s data side): published by the client through an
    /// `Arc` swap whenever a map's packets changed it.
    maps: Arc<HashMap<i32, map::MapData>>,
    /// Each drawn map's atlas entry (see `map::MapTexture`).
    map_textures: HashMap<i32, map::MapTexture>,
    /// Each map decoration type's sprite rectangle, resolved once (`None`: no sprite).
    map_decoration_rects: HashMap<u32, Option<[u32; 4]>>,
    /// `minecraft:map_decoration_type` entries in id order, like `item_names` for items.
    map_decoration_names: Vec<Identifier>,
    pub drawn: u32,
}

impl EntityRenderer {
    /// The beams' radius scale for this frame (vanilla `BeaconRenderer.extract`: 1 within 96
    /// blocks or while scoping, else the horizontal distance / 96).
    pub fn set_beam_radius_scale(&mut self, scale: f32) {
        self.beam_radius_scale = scale;
    }
    /// Vanilla `Entity.viewScale` for the entity render distance (from
    /// `options.entityDistanceScaling`, 0.5..=5.0, default 1.0).
    pub fn set_view_scale(&mut self, scale: f32) {
        self.view_scale = scale;
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
        orientation: glam::Quat,
        assets: &mut Assets,
    ) {
        let view_scale = self.view_scale;
        self.vertices.clear();
        self.mask_vertices.clear();
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
            // `AbstractArrow.shouldRenderAtSqrDistance` overrides the generic limit with the
            // bounding box diagonal times ten, so arrows stay visible much farther out.
            let visible = if matches!(&e.decoration, Some(Decoration::Arrow(_))) {
                arrow::arrow_should_render(dist2, e.width, e.height, view_scale)
            } else {
                should_render_at_sqr_distance(size, e.scale, dist2, view_scale)
            };
            if !visible {
                continue;
            }
            if !aabb_in_frustum(frustum, rel, half + 0.5) {
                continue;
            }
            let (sky, block) = e.light;
            // The instance's light is (sky, block); the vertex wants (block, sky).
            let light = (block, sky);
            if let Some(item) = &e.item {
                let start = self.vertices.len();
                self.emit_dropped_item(e, item, camera, light, assets);
                // `EntityRenderDispatcher.submit`: the fire after the entity's own renderer.
                if e.display_fire {
                    self.emit_flame(e, (e.pos - camera).as_vec3(), orientation, assets);
                }
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
                    Decoration::Minecart(c) => self.emit_minecart(e, c, camera, light, assets),
                    Decoration::ThrownItem(t) => {
                        self.emit_thrown_item(e, t, camera, orientation, light, assets)
                    }
                    Decoration::DragonFireball => {
                        self.emit_dragon_fireball(e, camera, orientation, light, assets)
                    }
                    Decoration::ExperienceOrb(o) => self.emit_experience_orb(
                        e,
                        o.value,
                        o.age,
                        camera,
                        light,
                        orientation,
                        assets,
                    ),
                    Decoration::Arrow(a) => self.emit_arrow(e, a, camera, light, assets),
                    // Vanilla `FallingBlockRenderer` / `TntRenderer`: the entity's block
                    // state drawn as the block model (see the renderer modules).
                    Decoration::FallingBlock(f) => {
                        self.emit_falling_block(e, f, camera, light, assets)
                    }
                    Decoration::PrimedTnt(t) => self.emit_primed_tnt(e, t, camera, light, assets),
                }
                // `EntityRenderDispatcher.submit`: the fire after the entity's own renderer —
                // a burning small fireball carries its `displayFireAnimation` flames on top
                // of the thrown-item sprite.
                if e.display_fire {
                    self.emit_flame(e, (e.pos - camera).as_vec3(), orientation, assets);
                }
                if self.vertices.len() > start {
                    self.drawn += 1;
                }
                continue;
            }
            let (texture, scale) = texture_and_scale(e);
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
            // `LivingEntityRenderer.submit`/`getRenderType`: an invisible entity draws no body
            // (`isBodyVisible` false -> null render type; the translucent ghost a teammate or
            // spectator would see needs the team inputs, not modelled yet) and no copy-model
            // overlay layers (`RenderLayer.coloredCutoutModelCopyLayerRender` skips invisible
            // states). The equipment layers stay: `HumanoidArmorLayer`, `ItemInHandLayer`,
            // `CustomHeadLayer` and `WingsLayer` do not check invisibility, so invisible
            // entities still wear armour and hold items.
            if !e.invisible {
                let rect = self.texture_rect(&texture, assets);
                // A small armour stand's model is authored with big cubes and small pivots;
                // its root children go through the small layer's per-part scale (heads 3/4,
                // the rest 1/2, about each pivot), which the cubes' UV layout needs vanilla's
                // original sizes for.
                if armor_stand::small_of(&e.model).is_some() {
                    armor_stand::emit_small_body(
                        &mut self.vertices,
                        &e.model,
                        &e.pose,
                        root,
                        rect,
                        light,
                        e.hurt,
                    );
                } else {
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
                }
                // Overlay layers: the same transform and pose, each with its own texture and
                // tint (vanilla's RenderLayers, e.g. SheepWoolLayer's dyed copy of the model).
                for overlay in e.overlays.iter().flatten() {
                    // The water patch is not a coloured layer: its triangles go to the
                    // depth-only mask buffer (`BoatRenderer.submitTypeAdditions`).
                    if boat::is_water_patch(&overlay.model) {
                        self.emit_water_mask(&overlay.model, root);
                        continue;
                    }
                    let Some((oroot, hurt)) = self.overlay_root(e, &overlay.model, root, assets)
                    else {
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
            }
            self.emit_equipment(e, root, light, assets);
            // `EntityRenderDispatcher.submit`: the fire after the entity's own renderer.
            if e.display_fire {
                self.emit_flame(e, (e.pos - camera).as_vec3(), orientation, assets);
            }
            if self.vertices.len() > start {
                self.drawn += 1;
            }
            if let Some(name) = e.nametag.as_ref().filter(|_| !e.invisible) {
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
        if self.mask_vertices.len() > self.mask_capacity {
            self.mask_capacity = self.mask_vertices.len().next_power_of_two();
            self.mask_buffer = boat::make_mask_buffer(&self.device, self.mask_capacity);
        }
        if !self.mask_vertices.is_empty() {
            self.queue.write_buffer(
                &self.mask_buffer,
                0,
                bytemuck::cast_slice(&self.mask_vertices),
            );
        }
    }

    /// Draws into the main pass (camera bind group at group 0); the boats' water masks ride
    /// along after the models (vanilla submits the patch inside the boat's own submission).
    pub fn draw<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.vertices.is_empty() && self.mask_vertices.is_empty() {
            return 0;
        }
        let mut calls = 0;
        if !self.vertices.is_empty() {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(1, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertices.len() as u32, 0..1);
            calls += 1;
        }
        calls + boat::draw_masks(self, pass)
    }
}
