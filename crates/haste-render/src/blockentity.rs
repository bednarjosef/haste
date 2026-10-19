// Block entity drawing: chests, ender chests, trapped chests and shulker boxes posed on the CPU
// into the entity renderer's vertex stream (one draw call for all of them, like entities), plus
// the object-model block entities (lectern and enchanting table books, bells, decorated pots and
// conduits) whose layers are y-up world-aligned and go through [`emit_object`].
//
// Follows vanilla's renderers (26.3 decompiled source, `net.minecraft.client.renderer.blockentity`):
// `ChestRenderer` (model transformation per facing, the lid angle curve, `Sheets.chooseSprite`'s
// texture per material and chest half, the Christmas rule from `SpecialDates.isExtendedChristmas`)
// and `ShulkerBoxRenderer` (its `createModelTransform` per facing, the lid's rise and spin from
// `ShulkerBoxBlockEntity.getProgress`). The models are vanilla's `ChestModel` /
// `ShulkerModel.createBoxLayer` layer definitions expressed as data.
//
// Haste's `CubeModel` parts are in vanilla *entity* convention (y down, the front at -z); vanilla's
// chest layer is y-up (its pose stack is world-aligned), so the chest cubes below have their y
// negated against the vanilla `addBox` values and the root matrix ends with the y mirror that
// undoes it. The shulker layer is already in entity convention; its root carries vanilla's own
// `scale(1, -1, -1)` flip instead.

use glam::{Mat4, Vec3};
use haste_core::model::{CubeModel, ModelPart, PoseOverrides};
use haste_core::{BlockPos, Direction, Identifier};

use crate::entity::{EntityVertex, TickPair};

#[cfg(test)]
use haste_core::model::Cube;
#[cfg(test)]
use haste_core::parts;

mod anim;
mod banner;
mod beacon;
mod bell;
mod book;
mod chest;
mod conduit;
mod decorated_pot;
mod enchant_table;
mod end_gateway;
mod items;
mod lectern;
mod object;
mod piston;
mod shulker;
mod sign;
mod skull;
mod view_distance;

pub use anim::BlockEntityAnim;
pub(crate) use banner::emit_banner;
pub use banner::{banner_flag_rotation, banner_wave_phase};
#[cfg(test)]
use banner::{banner_model, banner_root};
pub(crate) use beacon::emit_beacon_beam;
pub use beacon::{
    beam_animation_time, beam_radius_scale, beam_tex_v1, beam_tex_v_offset, BeamData, BeamSection,
    BEAM_MAX_RENDER_Y,
};
pub use bell::{bell_shake_rotation, bell_texture};
pub use book::{book_openness, book_texture};
#[cfg(test)]
use chest::local_month_day;
pub(crate) use chest::single_chest_model;
pub use chest::{chest_open_curve, chest_texture, xmas_textures, ChestHalf, ChestMaterial};
pub use conduit::{
    conduit_bob, conduit_cage_texture, conduit_eye_texture, conduit_shell_texture, conduit_spin,
    conduit_wind_texture,
};
pub use decorated_pot::{pot_base_texture, pot_side_texture};
pub use enchant_table::enchant_page_flip;
pub(crate) use end_gateway::emit_gateway_beam;
pub use end_gateway::{end_gateway_faces, GatewayBeam};
#[cfg(test)]
use items::spawner_root;
pub(crate) use items::{
    brushable_item_matrix, campfire_item_matrix, emit_spawner, item_kind, shelf_item_matrix,
    vault_root, ItemBlockEntity,
};
pub use items::{
    shelf_item_offset_y, spawner_scale, spawner_spin_at, BrushableItem, CampfireItems, ShelfItems,
    SpawnerDisplay, VaultDisplay, CAMPFIRE_ITEM_SIZE, SHELF_ALIGN_OFFSET, SHELF_ITEM_SIZE,
    SPAWNER_SCALE,
};
pub(crate) use object::emit_object_part;
pub(crate) use object::ObjectPose;
use object::{part, pose_of};
pub(crate) use piston::{piston_drawn_states, piston_extended_progress, piston_quads, PistonQuad};
pub use piston::{piston_progress, PistonData};
use shulker::shulker_color;
pub use shulker::shulker_texture;
#[cfg(test)]
use sign::BLACK_TEXT_OUTLINE_COLOR;
pub use sign::{sign_dark_color, sign_glow_color, sign_line_metrics_pub, DYE_DIFFUSE_COLOR};
pub(crate) use sign::{sign_line_metrics, sign_text_transform};
use skull::skull_kind_of;

pub use skull::{skull_dragon_jaw_rotation, skull_piglin_ear_rotations, skull_texture, SkullKind};
#[cfg(test)]
use skull::{skull_model, skull_root};
pub use view_distance::within_view_distance;

#[cfg(test)]
mod tests;

/// One side of a sign's text, vanilla `SignText`: up to 4 lines, a dye colour and the glowing
/// flag. Lines are already the *rendered* strings: the caller applied vanilla's wrap rule
/// (`Font.split` to the sign's max width, first wrapped line only) when parsing the NBT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignSideText {
    /// The rendered line (vanilla `SignText.getRenderMessages`, empty lines included).
    pub lines: [String; 4],
    /// The dye colour id, vanilla `DyeColor` order (sign NBT's `color`, default black = 15).
    pub color: u8,
    /// `has_glowing_text`.
    pub glowing: bool,
}

/// Both sides of a sign (vanilla `SignBlockEntity.frontText` / `backText`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignTextData {
    pub front: SignSideText,
    pub back: SignSideText,
}

/// One banner pattern layer (vanilla `BannerPatternLayers.Layer`): the pattern's registry
/// id (its texture is `entity/banner/<path>.png`) and the dye it is drawn in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerLayer {
    /// The pattern's registry id, e.g. `minecraft:stripe_bottom`.
    pub pattern: Identifier,
    /// The dye colour id, vanilla `DyeColor` order.
    pub color: u8,
}

/// A decorated pot's four sherds (vanilla `PotDecorations`), as the pottery pattern each side
/// was made from: the pattern's asset id (vanilla `DecoratedPotPattern.assetId`, e.g.
/// `minecraft:angler`), whose side texture is `entity/decorated_pot/<path>`. `None` is an empty
/// side, drawn with vanilla's blank `DECORATED_POT_SIDE`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PotSherds {
    pub front: Option<Identifier>,
    pub back: Option<Identifier>,
    pub left: Option<Identifier>,
    pub right: Option<Identifier>,
}

/// A banner's render data: the base colour (from the block id, vanilla `BannerBlock.getColor`)
/// and up to 16 pattern layers (vanilla draws at most `MAX_PATTERNS`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BannerData {
    /// The base dye colour id.
    pub base: u8,
    /// The pattern layers, in order.
    pub layers: Vec<BannerLayer>,
}

/// What to draw for one block entity (the per-type registry's rows; later renderers for beds
/// and heads plug in here).
#[derive(Clone, Debug, PartialEq)]
pub enum BlockEntityKind {
    Chest(ChestMaterial),
    Shulker(Option<u8>),
    /// A sign or hanging sign: the board is the chunk's block model; the renderer draws the
    /// text. `hanging` picks `HangingSignRenderer`'s transform and line metrics.
    Sign {
        hanging: bool,
    },
    /// A banner: `wall` picks `WallBannerBlock`'s transform (the bar without the pole,
    /// offset to the wall) over the standing one.
    Banner {
        wall: bool,
    },
    /// A skull/head on the floor or a wall (vanilla `SkullBlockRenderer`; the wall flag comes
    /// from the block id like the type does).
    Skull {
        kind: SkullKind,
        wall: bool,
    },
    /// A campfire: the items cooking on its grill, drawn by `CampfireRenderer`.
    Campfire(CampfireItems),
    /// A spawner or trial spawner: the mob spinning inside it,
    /// `TrialSpawnerRenderer.extractSpawnerData` + `SpawnerRenderer.submitEntityInSpawner`.
    Spawner(SpawnerDisplay),
    /// A vault: its displayed item spinning above it (`VaultRenderer`).
    Vault(VaultDisplay),
    /// A shelf block: up to three items standing on the board (`ShelfRenderer`).
    Shelf(ShelfItems),
    /// A brushable block (suspicious sand/gravel): the item sticking out of it while it is
    /// dusted, `BrushableBlockRenderer`.
    Brushable(BrushableItem),
    /// A lectern's book (vanilla `LecternRenderer`, drawn only when the block state's
    /// `has_book` is true).
    Lectern,
    /// An enchanting table's book (vanilla `EnchantTableRenderer`).
    EnchantTable,
    /// A bell (vanilla `BellRenderer`).
    Bell,
    /// A decorated pot (vanilla `DecoratedPotRenderer`): the base layer plus four sherd sides.
    DecoratedPot,
    /// A conduit (vanilla `ConduitRenderer`): its shell when inactive, cage, wind and eye when
    /// active.
    Conduit,
    /// A beacon's beam (vanilla `BeaconRenderer`): the beam sections' colours and heights ride
    /// in [`BlockEntityInstance::beam`].
    Beacon,
    /// An end portal (vanilla `TheEndPortalRenderer`): the flat 1x0.375x1 slab of the layered
    /// starfield shader; the visible faces ride in [`BlockEntityInstance::portal_faces`].
    EndPortal,
    /// An end gateway (vanilla `TheEndGatewayRenderer`): the full-cube starfield plus its
    /// spawning/cooldown beam in [`BlockEntityInstance::beam`].
    EndGateway,
    /// A moving piston (vanilla `PistonHeadRenderer`): the moved block drawn at the offset the
    /// block entity's progress interpolates; the data rides in
    /// [`BlockEntityInstance::piston`].
    MovingPiston,
}

/// One drawable block entity instance handed to the renderer each frame. `open` is the lid
/// animation as the ends of the last tick (vanilla's `ChestLidController.getOpenness` /
/// `ShulkerBoxBlockEntity.getProgress`), already combined over a double chest's halves;
/// `light` is (sky, block) at the block. Signs carry their text as a shared handle (no
/// per-frame allocation: the client caches one per block entity and re-clones the handle).
#[derive(Clone, Debug)]
pub struct BlockEntityInstance {
    pub pos: BlockPos,
    /// The block's `facing` property (horizontal for chests, any for shulker boxes); for
    /// standing signs the `rotation` segment as a horizontal facing, wall signs the wall's
    /// `facing`.
    pub facing: Direction,
    pub half: ChestHalf,
    pub kind: BlockEntityKind,
    pub open: TickPair,
    pub light: (u8, u8),
    /// The sign's yaw in vanilla degrees (standing: `RotationSegment.convertToDegrees` of the
    /// `rotation` property; wall: the wall `facing.toYRot()`), for `BlockEntityKind::Sign`.
    pub sign_angle: f32,
    /// Sign text (both sides), for `BlockEntityKind::Sign`.
    pub text: Option<std::sync::Arc<SignTextData>>,
    /// The banner's wave phase (vanilla `BannerRenderer.extractRenderState`: the position hash
    /// plus game time, over 100), for `BlockEntityKind::Banner`.
    pub banner_phase: f32,
    /// Banner patterns, for `BlockEntityKind::Banner`.
    pub banner: Option<std::sync::Arc<BannerData>>,
    /// The skull's animation tick as the ends of the last tick (vanilla
    /// `SkullBlockEntity.getAnimation`, counting while the block is powered), for
    /// `BlockEntityKind::Skull`.
    pub skull_anim: TickPair,
    /// The object-model block entities' animation state (book, bell shake, pot wobble, conduit
    /// spin), for the kinds from `Lectern` on.
    pub anim: Option<BlockEntityAnim>,
    /// The decorated pot's sherds, for `BlockEntityKind::DecoratedPot`.
    pub sherds: Option<std::sync::Arc<PotSherds>>,
    /// The beacon's beam sections (computed by the game thread's glass walk), for
    /// `BlockEntityKind::Beacon`.
    pub beam: Option<std::sync::Arc<BeamData>>,
    /// The beams' animation time this frame (vanilla `BeaconRenderState.animationTime`:
    /// `floorMod(gameTime, 40) + partialTicks`), for `BlockEntityKind::Beacon` and
    /// `EndGateway`.
    pub beam_time: f32,
    /// The end gateway's beam, for `BlockEntityKind::EndGateway`.
    pub gateway_beam: Option<GatewayBeam>,
    /// The end portal's/gateway's visible faces, a bitmask over `Direction` ordinals (vanilla
    /// `shouldRenderFace`: Y only for the portal, `Block.shouldRenderFace` for the gateway),
    /// for `BlockEntityKind::EndPortal`/`EndGateway`.
    pub portal_faces: u8,
    /// The moving piston's state, for `BlockEntityKind::MovingPiston`.
    pub piston: Option<std::sync::Arc<PistonData>>,
}

/// Resolves a block entity type name (from the synced `minecraft:block_entity_type` registry)
/// plus the block it is in into what to draw. `None` draws nothing (logged once by the caller).
/// Vanilla `BlockEntityRenderers` maps the type to its renderer; `ChestRenderer.getChestMaterial`
/// picks the material (ender chest by entity type, Christmas by the calendar, trapped by entity
/// type); the shulker colour comes from the block id (`ShulkerBoxBlock.getColor`).
pub fn kind_of(be_type: &Identifier, block: &Identifier, xmas: bool) -> Option<BlockEntityKind> {
    if be_type.namespace() != "minecraft" {
        return None;
    }
    match be_type.path() {
        "chest" => Some(BlockEntityKind::Chest(if xmas {
            ChestMaterial::Christmas
        } else {
            ChestMaterial::Normal
        })),
        "trapped_chest" => Some(BlockEntityKind::Chest(if xmas {
            ChestMaterial::Christmas
        } else {
            ChestMaterial::Trapped
        })),
        "ender_chest" => Some(BlockEntityKind::Chest(ChestMaterial::Ender)),
        "shulker_box" => Some(BlockEntityKind::Shulker(shulker_color(block))),
        "sign" => Some(BlockEntityKind::Sign { hanging: false }),
        "hanging_sign" => Some(BlockEntityKind::Sign { hanging: true }),
        "banner" => Some(BlockEntityKind::Banner {
            wall: block.path().ends_with("_wall_banner"),
        }),
        "skull" => skull_kind_of(block),
        "campfire" => Some(BlockEntityKind::Campfire(CampfireItems::default())),
        "mob_spawner" | "trial_spawner" => Some(BlockEntityKind::Spawner(SpawnerDisplay {
            model: None,
            spin: TickPair::fixed(0.0),
            scale: SPAWNER_SCALE,
        })),
        "vault" => Some(BlockEntityKind::Vault(VaultDisplay {
            item: haste_core::RegistryIndex(0),
            count: 0,
            spin: TickPair::fixed(0.0),
        })),
        "shelf" => Some(BlockEntityKind::Shelf(ShelfItems::default())),
        "brushable_block" => Some(BlockEntityKind::Brushable(BrushableItem {
            item: haste_core::RegistryIndex(0),
            hit_direction: None,
            dusted: 0,
        })),
        "lectern" => Some(BlockEntityKind::Lectern),
        "enchanting_table" => Some(BlockEntityKind::EnchantTable),
        "bell" => Some(BlockEntityKind::Bell),
        "decorated_pot" => Some(BlockEntityKind::DecoratedPot),
        "conduit" => Some(BlockEntityKind::Conduit),
        "beacon" => Some(BlockEntityKind::Beacon),
        "end_portal" => Some(BlockEntityKind::EndPortal),
        "end_gateway" => Some(BlockEntityKind::EndGateway),
        "piston" => Some(BlockEntityKind::MovingPiston),
        _ => None,
    }
}

/// Whether `block` is one of the blocks whose `block_event` action 1 drives a lid (used by the
/// client to filter events; copper chests are not drawn yet and are excluded).
pub fn is_lid_block(block: &Identifier) -> bool {
    block.namespace() == "minecraft"
        && matches!(block.path(), "chest" | "trapped_chest" | "ender_chest")
        || (block.namespace() == "minecraft" && block.path().ends_with("shulker_box"))
}

/// Whether `block`'s `block_event` action 1 is a bell ring (vanilla `BellBlockEntity.onHit`),
/// whose parameter is the click direction's 3D data value.
pub fn is_bell_block(block: &Identifier) -> bool {
    block.namespace() == "minecraft" && block.path() == "bell"
}

/// Whether `block`'s `block_event` action 1 is a decorated pot wobble (vanilla
/// `DecoratedPotBlockEntity.wobble`), whose parameter is the wobble style's ordinal.
pub fn is_pot_block(block: &Identifier) -> bool {
    block.namespace() == "minecraft" && block.path() == "decorated_pot"
}

/// Vanilla `Direction.toYRot`: south 0, west 90, north 180, east 270.
fn to_y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        Direction::Down | Direction::Up => 0.0,
    }
}

/// The entity renderer's cube emitter, as a type (light and shading are its job).
pub(crate) type EmitPart<'a> = &'a dyn Fn(
    &mut Vec<EntityVertex>,
    &CubeModel,
    &ModelPart,
    &PoseOverrides,
    Mat4,
    [u32; 4],
    (u8, u8),
);

/// The end portal's visible-face rule (vanilla `TheEndPortalBlockEntity.shouldRenderFace`):
/// only the horizontal faces.
pub fn end_portal_faces() -> u8 {
    (1 << Direction::Up as u8) | (1 << Direction::Down as u8)
}

/// Poses one block entity into `out` (camera-relative, the entity stream's convention).
/// `rect` is the texture's atlas rectangle, `light` the lightmap coordinates (block, sky).
pub(crate) fn emit_block_entity(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    rect: [u32; 4],
    emit_part: EmitPart,
) {
    // The instance stores (sky, block); the vertex wants (block, sky).
    let (sky, block) = inst.light;
    let light = (block, sky);
    match inst.kind {
        // The object-model block entities (books, bells, decorated pots, conduits) go through
        // `emit_model_block_entity`, which owns their textures and transforms.
        BlockEntityKind::Lectern
        | BlockEntityKind::EnchantTable
        | BlockEntityKind::Bell
        | BlockEntityKind::DecoratedPot
        | BlockEntityKind::Conduit => {}
        BlockEntityKind::Chest(_material) => {
            chest::submit(out, inst, camera, rect, light);
        }
        BlockEntityKind::Shulker(_color) => {
            shulker::submit(out, inst, camera, rect, light, emit_part);
        }
        // Signs draw no cubes here: the board is the chunk's block model and the text goes
        // through the entity renderer's glyph pass (`emit_sign_text`), which needs the font.
        BlockEntityKind::Sign { .. } => {}
        // Banners likewise draw per-pattern layers with a tint, handled by
        // `EntityRenderer::emit_banner`.
        BlockEntityKind::Banner { .. } => {}
        // The item and mob block entities draw through the entity renderer's item and cube
        // emitters (they need the item models and the atlas), not the block entity's texture.
        BlockEntityKind::Campfire(_)
        | BlockEntityKind::Spawner(_)
        | BlockEntityKind::Vault(_)
        | BlockEntityKind::Shelf(_)
        | BlockEntityKind::Brushable(_) => {}
        // The beacon beam, end portals/gateways and moving pistons have their own pipelines
        // (the beam ignores the lightmap, the portal is a position-only starfield, the piston
        // resolves its moved block's model); `EntityRenderer::append_block_entities` routes
        // them there.
        BlockEntityKind::Beacon
        | BlockEntityKind::EndPortal
        | BlockEntityKind::EndGateway
        | BlockEntityKind::MovingPiston => {}
        BlockEntityKind::Skull { .. } => {
            skull::submit(out, inst, camera, rect, light, emit_part);
        }
    }
}

/// The end portal/gateway's starfield cube (vanilla `AbstractEndPortalRenderer.submitCube`):
/// one quad per visible face of the unit cube, posed by `m` (the portal's own flat transform,
/// the gateway's identity), into the portal pipeline's position vertex stream. Each vertex
/// carries the layer count (15 portal, 16 gateway).
pub(crate) fn emit_portal_faces(out: &mut Vec<[f32; 4]>, faces: u8, layers: f32, m: Mat4) {
    for d in Direction::ALL {
        if faces & (1 << d as u8) == 0 {
            continue;
        }
        // The unit cube's face corners (vanilla `FaceInfo.fromFacing` order).
        let (a, b, c, dd) = match d {
            Direction::Down => (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 0.0),
            ),
            Direction::Up => (
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 1.0),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(1.0, 1.0, 0.0),
            ),
            Direction::North => (
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ),
            Direction::South => (
                Vec3::new(0.0, 1.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(1.0, 1.0, 1.0),
            ),
            Direction::West => (
                Vec3::new(0.0, 1.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ),
            Direction::East => (
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(1.0, 1.0, 1.0),
            ),
        };
        // Two triangles per face (the pipeline is a TriangleList; vanilla's QUADS topology
        // took the four corners, here each face needs the six indices spelled out).
        let push = |out: &mut Vec<[f32; 4]>, p: Vec3| {
            let p = m.transform_point3(p);
            out.push([p.x, p.y, p.z, layers]);
        };
        for p in [a, b, c, a, c, dd] {
            push(out, p);
        }
    }
}

/// The beacon-beam and end-portal pipelines the renderer creates once (vanilla
/// `RenderPipelines.BEACON_BEAM_OPAQUE`, `BEACON_BEAM_TRANSLUCENT`, `END_PORTAL`/`END_GATEWAY`):
/// the beam's two passes over the entity atlas without the lightmap, and the portal's
/// position-only starfield.
pub struct BeamPipelines {
    /// The inner beam: opaque, depth write (vanilla `BEACON_BEAM_OPAQUE`).
    pub opaque: wgpu::RenderPipeline,
    /// The glow: alpha-blended, no depth write (`BEACON_BEAM_TRANSLUCENT`).
    pub glow: wgpu::RenderPipeline,
    /// The end portal/gateway starfield (`END_PORTAL`/`END_GATEWAY` share the shader; the
    /// layer count rides in as a pipeline constant).
    pub portal: wgpu::RenderPipeline,
    /// The end portal's own bind group: the End sky and portal textures (the beam pipelines
    /// reuse the entity atlas bind group).
    pub portal_bind_group: wgpu::BindGroup,
}

/// Vanilla `Camera.setRotation`'s orientation quaternion, which the conduit's eye billboard
/// needs: `rotationYXZ(PI - yawDeg * PI/180, -pitchDeg * PI/180, 0)`.
pub fn camera_orientation(yaw_deg: f32, pitch_deg: f32) -> glam::Quat {
    glam::Quat::from_euler(
        glam::EulerRot::YXZ,
        std::f32::consts::PI - yaw_deg.to_radians(),
        -pitch_deg.to_radians(),
        0.0,
    )
}

/// Vanilla `Direction.getClockWise` (the y-axis turn).
fn clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::East,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        Direction::East => Direction::South,
        Direction::Up | Direction::Down => d,
    }
}

pub(crate) fn emit_model_block_entity(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    orientation: glam::Quat,
    rect: [u32; 4],
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    // The instance stores (sky, block); the vertex wants (block, sky).
    let (sky, block) = inst.light;
    let light = (block, sky);
    match inst.kind {
        BlockEntityKind::Lectern => lectern::submit(out, inst, camera, light, rect),
        BlockEntityKind::EnchantTable => enchant_table::submit(out, inst, camera, light, rect),
        BlockEntityKind::Bell => bell::submit(out, inst, camera, light, rect),
        BlockEntityKind::DecoratedPot => {
            decorated_pot::submit(out, inst, camera, light, rect, rect_of)
        }
        BlockEntityKind::Conduit => {
            conduit::submit(out, inst, camera, orientation, light, rect, rect_of)
        }
        BlockEntityKind::Chest(_)
        | BlockEntityKind::Shulker(_)
        | BlockEntityKind::Sign { .. }
        | BlockEntityKind::Banner { .. }
        | BlockEntityKind::Skull { .. }
        | BlockEntityKind::Beacon
        | BlockEntityKind::EndPortal
        | BlockEntityKind::EndGateway
        | BlockEntityKind::MovingPiston => {}
        // The item and mob block entities are split out by `item_kind` and `emit_spawner`
        // before the emitter gets here.
        BlockEntityKind::Campfire(_)
        | BlockEntityKind::Spawner(_)
        | BlockEntityKind::Vault(_)
        | BlockEntityKind::Shelf(_)
        | BlockEntityKind::Brushable(_) => {}
    }
}
