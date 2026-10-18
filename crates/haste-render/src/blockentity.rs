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

use std::sync::{Arc, OnceLock};

use glam::{Mat3, Mat4, Vec3};
use haste_core::model::{Cube, CubeModel, ModelPart, PoseOverrides};
use haste_core::parts;
use haste_core::{BlockPos, Direction, Identifier};

use crate::entity::{entity_shade, hurt_color, EntityVertex, TickPair};

mod anim;
mod banner;
mod beacon;
mod book;
mod chest;
mod enchant_table;
mod end_gateway;
mod lectern;
mod shulker;
mod sign;

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
pub use book::{book_openness, book_texture};
#[cfg(test)]
use chest::local_month_day;
pub use chest::{chest_open_curve, chest_texture, xmas_textures, ChestHalf, ChestMaterial};
pub use enchant_table::enchant_page_flip;
pub(crate) use end_gateway::emit_gateway_beam;
pub use end_gateway::{end_gateway_faces, GatewayBeam};
use shulker::shulker_color;
pub use shulker::shulker_texture;
#[cfg(test)]
use sign::BLACK_TEXT_OUTLINE_COLOR;
pub use sign::{sign_dark_color, sign_glow_color, sign_line_metrics_pub, DYE_DIFFUSE_COLOR};
pub(crate) use sign::{sign_line_metrics, sign_text_transform};

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

/// A moving piston's render data, as vanilla's block entity keeps it: the moved block's state
/// string (from the NBT, full `block[facing=north,...]` form), the progress at the ends of the
/// last tick (vanilla ticks it 0.5 per tick to 1.0), and the direction flags. The renderer
/// resolves the state's quads once and caches them.
#[derive(Clone, Debug, PartialEq)]
pub struct PistonData {
    /// The moved block state as vanilla's NBT `blockState` string (adjusted by the renderer
    /// for the piston head's `short` and the base's `extended`, like `PistonHeadRenderer`
    /// does before baking).
    pub block_state: String,
    /// `progressO`/`progress`, the extension at the ends of the last tick.
    pub progress: TickPair,
    /// The piston's own `facing` (NBT `facing`, the direction the head moves to).
    pub facing: Direction,
    /// NBT `extending`.
    pub extending: bool,
    /// NBT `source` (this block entity is the piston base itself).
    pub source: bool,
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

/// The items on a campfire's grill, by slot (vanilla `CampfireBlockEntity.getItems()`, at most
/// four). Fixed size and `Copy`: no allocation per frame or per block entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CampfireItems {
    /// The item in each slot, by item registry index (`None` for an empty slot).
    pub items: [Option<haste_core::RegistryIndex>; 4],
}

impl CampfireItems {
    /// The item in `slot`, if any.
    pub fn get(&self, slot: usize) -> Option<haste_core::RegistryIndex> {
        self.items.get(slot).copied().flatten()
    }
}

/// The mob spinning inside a spawner (vanilla `SpawnerRenderState`): the display entity's
/// model (the client resolves it from `SpawnData`), the yaw spin in degrees and the scale.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnerDisplay {
    /// The display entity's cube model; `None` draws nothing (vanilla leaves the render
    /// state's `displayEntity` null when the spawn data names no spawnable entity).
    pub model: Option<std::sync::Arc<CubeModel>>,
    /// The display entity's spin as the ends of the last tick, in the units vanilla's
    /// `BaseSpawner.spin` / `TrialSpawnerStateData.spin` accumulate (see
    /// [`spawner_spin_at`] for the degrees the renderer draws).
    pub spin: TickPair,
    /// The scale vanilla derives from the display entity's bounding box
    /// (`0.53125` divided by the larger of its width and height when that exceeds 1).
    pub scale: f32,
}

/// The degrees a spawner's display mob is turned to at `partial`, vanilla
/// `TrialSpawnerRenderer.extractSpawnerData`'s `Mth.lerp(partialTicks, oSpin, spin) * 10`
/// (vanilla keeps the accumulating spin in its own units and scales it only for the pose).
pub fn spawner_spin_at(spawner: &SpawnerDisplay, partial: f32) -> f32 {
    spawner.spin.at(partial) * 10.0
}

/// A vault's displayed item (vanilla `VaultRenderState`): the item and how many copies of the
/// stack to draw, plus the spin in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VaultDisplay {
    pub item: haste_core::RegistryIndex,
    /// `ItemClusterRenderState.getRenderedAmount` of the stack's count.
    pub count: u32,
    pub spin: TickPair,
}

impl VaultDisplay {
    /// How many copies of the stack vanilla draws (1 for a single item, then one more per
    /// full 16 of the count, up to 5): `ItemClusterRenderState.getRenderedAmount`.
    pub fn copies(&self) -> u32 {
        match self.count {
            ..=0 => 0,
            1 => 1,
            2..=16 => 2,
            17..=32 => 3,
            33..=48 => 4,
            _ => 5,
        }
    }

    /// The yaw the displayed item is turned to at `partial`, vanilla `VaultRenderer`'s
    /// `Mth.rotLerp(partialTicks, clientData.previousSpin(), clientData.currentSpin())`: the
    /// lerp wraps at 180 degrees so the item keeps turning the short way when the spin rolls
    /// over from its 10 degrees a tick (`VaultClientData.updateDisplayItemSpin`).
    pub fn spin_at(&self, partial: f32) -> f32 {
        let from = self.spin.start;
        let mut delta = (self.spin.end - from) % 360.0;
        if delta >= 180.0 {
            delta -= 360.0;
        }
        if delta < -180.0 {
            delta += 360.0;
        }
        from + partial * delta
    }
}

/// The items on a shelf block (vanilla `ShelfBlockEntity`): three slots plus the
/// `align_items_to_bottom` flag the board's NBT carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ShelfItems {
    pub items: [Option<haste_core::RegistryIndex>; 3],
    pub align_to_bottom: bool,
}

impl ShelfItems {
    /// The item in `slot`, if any.
    pub fn get(&self, slot: usize) -> Option<haste_core::RegistryIndex> {
        self.items.get(slot).copied().flatten()
    }
}

/// The item a brushable block reveals (vanilla `BrushableBlockRenderState`): the item, the
/// block face it is brushed from, and the `dusted` state (0..3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrushableItem {
    pub item: haste_core::RegistryIndex,
    /// `BrushableBlockEntity.getHitDirection()`; `None` while the block has not been brushed
    /// (vanilla then draws nothing).
    pub hit_direction: Option<Direction>,
    /// The block state's `dusted` property (vanilla's completion state, 0..3).
    pub dusted: u8,
}

/// The head types vanilla renders (vanilla `SkullBlock.Types`); the block id picks one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkullKind {
    Skeleton,
    WitherSkeleton,
    Zombie,
    Creeper,
    Dragon,
    Piglin,
    Player,
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

/// The head kind and wall flag from a skull block id (vanilla `SkullBlock.Types`'s blocks:
/// `skeleton_skull`, `wither_skeleton_skull`, `zombie_head`, `creeper_head`, `dragon_head`,
/// `piglin_head`, `player_head`; the wall variants insert `_wall_`, e.g.
/// `creeper_wall_head`).
fn skull_kind_of(block: &Identifier) -> Option<BlockEntityKind> {
    if block.namespace() != "minecraft" {
        return None;
    }
    let path = block.path();
    let (name, wall) = match path.split_once("_wall_") {
        // `skeleton_wall_skull` -> `skeleton_skull`, `creeper_wall_head` -> `creeper_head`.
        Some((head, tail)) => (format!("{head}_{tail}"), true),
        None => (path.to_string(), false),
    };
    let kind = match name.as_str() {
        "skeleton_skull" => SkullKind::Skeleton,
        "wither_skeleton_skull" => SkullKind::WitherSkeleton,
        "zombie_head" => SkullKind::Zombie,
        "creeper_head" => SkullKind::Creeper,
        "dragon_head" => SkullKind::Dragon,
        "piglin_head" => SkullKind::Piglin,
        "player_head" => SkullKind::Player,
        _ => return None,
    };
    Some(BlockEntityKind::Skull { kind, wall })
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

/// Vanilla `TrialSpawnerRenderer.extractSpawnerData`'s base scale before the bounding-box
/// correction (`state.scale = 0.53125F`).
pub const SPAWNER_SCALE: f32 = 0.53125;

/// Vanilla `CampfireRenderer.SIZE`: the scale an item on a campfire's grill is drawn at.
pub const CAMPFIRE_ITEM_SIZE: f32 = 0.375;

/// Vanilla `ShelfRenderer.ITEM_SIZE` and `ALIGN_ITEMS_TO_BOTTOM`.
pub const SHELF_ITEM_SIZE: f32 = 0.25;
pub const SHELF_ALIGN_OFFSET: f32 = -0.25;

/// The extent of a cube model's geometry at rest (model units), as the bounding box of every
/// cube transformed by its part pivots and rotations (the same matrices `emit_part` composes,
/// with no pose). Used to size a spawner's display entity.
pub(crate) fn cube_model_extent(model: &CubeModel) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut see = |p: Vec3| {
        min = min.min(p);
        max = max.max(p);
    };
    extent_of(&model.root, Mat4::IDENTITY, &mut see);
    if min.x.is_finite() {
        (min, max)
    } else {
        (Vec3::ZERO, Vec3::ZERO)
    }
}

fn extent_of(part: &ModelPart, parent: Mat4, see: &mut impl FnMut(Vec3)) {
    let m = parent
        * Mat4::from_translation(part.pivot)
        * Mat4::from_rotation_z(part.rotation.z)
        * Mat4::from_rotation_y(part.rotation.y)
        * Mat4::from_rotation_x(part.rotation.x);
    for cube in &part.cubes {
        let o = cube.origin + cube.inflate;
        let s = cube.size + 2.0 * cube.inflate;
        for x in [o.x, o.x + s.x] {
            for y in [o.y, o.y + s.y] {
                for z in [o.z, o.z + s.z] {
                    see(m.transform_point3(Vec3::new(x, y, z)));
                }
            }
        }
    }
    for child in &part.children {
        extent_of(child, m, see);
    }
}

/// Vanilla `TrialSpawnerRenderer.extractSpawnerData`'s scale for `model`: `0.53125`, divided by
/// the larger of the display entity's bounding box width and height when that exceeds one
/// block. Haste has no per-type bounding box table yet (haste-entity's `EntityDimensions`
/// falls back to its default box, `crates/haste-entity/src/dims.rs:49`), so the box comes from
/// the display model's own geometry, which vanilla's mob models are built to match.
pub fn spawner_scale(model: &CubeModel) -> f32 {
    let (min, max) = cube_model_extent(model);
    let longest = (max.x - min.x).max(max.y - min.y) / 16.0;
    if longest > 1.0 {
        SPAWNER_SCALE / longest
    } else {
        SPAWNER_SCALE
    }
}

/// Vanilla `CampfireRenderer.submit`'s per-slot matrix: the block's centre, turned so the slot
/// looks outwards (`Direction.from2DDataValue((slot + facing.get2DDataValue()) % 4)`'s
/// `-toYRot()`), laid flat (the +90 degrees about X) and offset to its own quadrant of the
/// grill, at vanilla's `SIZE` scale. The item's own `display.fixed` transform is the
/// renderer's job (`emit_item`).
pub(crate) fn campfire_item_matrix(
    pos: BlockPos,
    facing: Direction,
    slot: usize,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    // The slot's direction around the block (`from2DDataValue` of `(slot + facing) % 4`): the
    // 2D order is south 0, west 1, north 2, east 3, so the item turns by `-toYRot` of it.
    let two_d = (slot + facing.to_2d_data_value() as usize) % 4;
    let y_rot = -(two_d as f32 * 90.0);
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.44921875, 0.5))
        * Mat4::from_rotation_y(y_rot.to_radians())
        * Mat4::from_rotation_x(90.0f32.to_radians())
        * Mat4::from_translation(Vec3::new(-0.3125, -0.3125, 0.0))
        * Mat4::from_scale(Vec3::splat(CAMPFIRE_ITEM_SIZE))
}

/// Vanilla `SpawnerRenderer.submitEntityInSpawner` followed by
/// `LivingEntityRenderer.submit`'s transform for the static display entity: the block's centre
/// turned by the spawner's spin, the mob dropped a little and tipped back 30 degrees, then
/// vanilla's `setupRotations` yaw (the display entity never turns its body, so it is the
/// fixed `180 - bodyRot`), the model's y/z flip and the /16 that turns the model's pixel units
/// into blocks. Haste's entity models place the feet at their y 0, so vanilla's extra
/// `-1.501` block translate is not needed here.
pub(crate) fn spawner_root(pos: BlockPos, spin: f32, scale: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.4, 0.5))
        * Mat4::from_rotation_y(spin.to_radians())
        * Mat4::from_translation(Vec3::new(0.0, -0.2, 0.0))
        * Mat4::from_rotation_x((-30.0f32).to_radians())
        * Mat4::from_scale(Vec3::splat(scale))
        * Mat4::from_rotation_y(180.0f32.to_radians())
        * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0)
}

/// Poses a spawner's display entity model into `out` (the entity motion is not animated: the
/// mob only turns with the spawner's spin, like vanilla's `EntityRenderDispatcher.submit` on
/// the static display entity).
pub(crate) fn emit_spawner(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    model: &CubeModel,
    rect: [u32; 4],
    camera: glam::DVec3,
    partial: f32,
) {
    let BlockEntityKind::Spawner(ref spawner) = inst.kind else {
        return;
    };
    use crate::entity::emit_part_tinted;
    let (sky, block) = inst.light;
    let root = spawner_root(
        inst.pos,
        spawner_spin_at(spawner, partial),
        spawner.scale,
        camera,
    );
    emit_part_tinted(
        out,
        model,
        &model.root,
        &PoseOverrides::default(),
        root,
        rect,
        (block, sky),
        0.0,
        [1.0; 3],
    );
}

/// Vanilla `VaultRenderer.submit`: the block's centre at 0.4, turned by the vault's spin. The
/// item cluster's copies come from the caller.
pub(crate) fn vault_root(pos: BlockPos, spin: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.4, 0.5))
        * Mat4::from_rotation_y(spin.to_radians())
}

/// Vanilla `ShelfRenderer.submitItem`: the block's centre turned by the shelf's facing (a
/// vertical facing uses vanilla's 180 degrees), then the slot's x offset and the standing
/// plane's -0.25, at vanilla's `ITEM_SIZE` scale. `offset_y` is vanilla's bounding-box
/// alignment (see [`shelf_item_offset_y`]); it is applied in the same scaled space, after the
/// scale, exactly like the renderer's `poseStack.translate(0, offsetY, 0)`.
pub(crate) fn shelf_item_matrix(
    pos: BlockPos,
    facing: Direction,
    slot: usize,
    align_to_bottom: bool,
    offset_y: f32,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let y_rot = if facing.is_horizontal() {
        -facing.to_y_rot()
    } else {
        180.0
    };
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.5, 0.5))
        * Mat4::from_rotation_y(y_rot.to_radians())
        * Mat4::from_translation(Vec3::new(
            (slot as f32 - 1.0) * 0.3125,
            if align_to_bottom {
                SHELF_ALIGN_OFFSET
            } else {
                0.0
            },
            -0.25,
        ))
        * Mat4::from_scale(Vec3::splat(SHELF_ITEM_SIZE))
        * Mat4::from_translation(Vec3::new(0.0, offset_y, 0.0))
}

/// Vanilla `ShelfRenderer.submitItem`'s vertical alignment: the model's box is lifted by its
/// min y, and centred on the board when the shelf does not align its items to the bottom.
/// `bbox` is the model's bounding box through its `on_shelf` display transform.
pub fn shelf_item_offset_y(bbox: (Vec3, Vec3), align_to_bottom: bool) -> f32 {
    let (min, max) = bbox;
    let mut offset = -min.y;
    if !align_to_bottom {
        offset += -(max.y - min.y) / 2.0;
    }
    offset
}

/// Vanilla `BrushableBlockRenderer.submit`'s matrix: half a block up, translated out of the
/// brushed face by the completion state, then vanilla's fixed 75 + 11 (90 more for a face on
/// the x axis) degrees of yaw, at half scale. The item's own `display.fixed` transform is the
/// renderer's job.
pub(crate) fn brushable_item_matrix(
    pos: BlockPos,
    hit_direction: Direction,
    dusted: u8,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    // Vanilla `translations`: each face's own resting translation, plus the completion offset
    // out of it (`completionState / 10 * 0.75`).
    let mut t = Vec3::new(0.5, 0.0, 0.5);
    let offset = dusted as f32 / 10.0 * 0.75;
    match hit_direction {
        Direction::East => t.x = 0.73 + offset,
        Direction::West => t.x = 0.25 - offset,
        Direction::Up => t.y = 0.25 + offset,
        Direction::Down => t.y = -0.23 - offset,
        Direction::North => t.z = 0.25 - offset,
        Direction::South => t.z = 0.73 + offset,
    }
    let east_west = matches!(hit_direction, Direction::East | Direction::West);
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.0, 0.5, 0.0))
        * Mat4::from_translation(t)
        * Mat4::from_rotation_y(75.0f32.to_radians())
        * Mat4::from_rotation_y((if east_west { 90.0f32 } else { 0.0 } + 11.0).to_radians())
        * Mat4::from_scale(Vec3::splat(0.5))
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

/// Vanilla `PistonMovingBlockEntity.getProgress`: `lerp(partial, progressO, progress)` clamped
/// at partial 1.
pub fn piston_progress(p: &PistonData, partial: f32) -> f32 {
    p.progress.at(partial.min(1.0))
}

/// Vanilla `PistonMovingBlockEntity.getExtendedProgress`: `progress - 1` extending,
/// `1 - progress` retracting.
pub fn piston_extended_progress(p: &PistonData, progress: f32) -> f32 {
    if p.extending {
        progress - 1.0
    } else {
        1.0 - progress
    }
}

/// The moved block's state string after vanilla `PistonHeadRenderer.extractRenderState`'s
/// adjustments: a piston head picks `short` by the progress (<= 0.5), and the source piston's
/// retracting base draws `extended=true` with a head above it.
pub fn piston_drawn_states(p: &PistonData, progress: f32) -> (String, Option<String>) {
    let (block, props) = parse_state_string(&p.block_state);
    let mut props = props;
    let is_head = block.path() == "piston_head";
    let is_piston = matches!(block.path(), "piston" | "sticky_piston");
    if is_head && progress <= 4.0 {
        set_prop(&mut props, "short", &(progress <= 0.5).to_string());
        return (format_state(&block, &props), None);
    }
    if p.source && !p.extending && is_piston {
        // The head that retracts with the base (vanilla builds a piston_head state with the
        // piston's own type and facing, `short` by the progress).
        let sticky = block.path() == "sticky_piston";
        let facing = props
            .iter()
            .find(|(k, _)| k == "facing")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "north".into());
        let head = format_state(
            &Identifier::minecraft("piston_head"),
            &[
                ("facing".to_string(), facing.clone()),
                ("short".to_string(), (progress >= 0.5).to_string()),
                (
                    "type".to_string(),
                    if sticky { "sticky" } else { "normal" }.to_string(),
                ),
            ],
        );
        set_prop(&mut props, "extended", "true");
        return (format_state(&block, &props), Some(head));
    }
    (p.block_state.clone(), None)
}

/// Parses a vanilla block state string `block[facing=north,...]` into its id and properties.
pub(crate) fn parse_state_string(s: &str) -> (Identifier, Vec<(String, String)>) {
    let (name, props) = match s.split_once('[') {
        Some(n) => (n.0, n.1.trim_end_matches(']')),
        None => (s, ""),
    };
    let id = Identifier::parse(name).unwrap_or_else(|_| Identifier::minecraft("stone"));
    let props = props
        .split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    (id, props)
}

/// Formats a block id and properties back into vanilla's state string form.
fn format_state(block: &Identifier, props: &[(String, String)]) -> String {
    if props.is_empty() {
        return block.to_string();
    }
    let joined: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}[{}]", block, joined.join(","))
}

fn set_prop(props: &mut Vec<(String, String)>, name: &str, value: &str) {
    if let Some(p) = props.iter_mut().find(|(k, _)| k == name) {
        p.1 = value.to_string();
    } else {
        props.push((name.to_string(), value.to_string()));
    }
}

/// The texture a head kind is drawn with (vanilla `SkullBlockRenderer.SKIN_BY_TYPE`; the
/// player head uses the default skin, custom profiles are not modelled yet).
pub fn skull_texture(kind: SkullKind) -> Identifier {
    match kind {
        SkullKind::Skeleton => Identifier::minecraft("entity/skeleton/skeleton"),
        SkullKind::WitherSkeleton => Identifier::minecraft("entity/skeleton/wither_skeleton"),
        SkullKind::Zombie => Identifier::minecraft("entity/zombie/zombie"),
        SkullKind::Creeper => Identifier::minecraft("entity/creeper/creeper"),
        SkullKind::Dragon => Identifier::minecraft("entity/enderdragon/dragon"),
        SkullKind::Piglin => Identifier::minecraft("entity/piglin/piglin"),
        SkullKind::Player => Identifier::minecraft("entity/player/wide/steve"),
    }
}

/// The head models as data. Skeleton, wither skeleton and creeper share `SkullModel`'s
/// mob head layer (one cube, 64x32 texture); the player and zombie add the hat cube on the
/// humanoid head layer (64x64); the piglin head has its own part set with the ears (64x64);
/// the dragon head is scaled 0.75 with the jaw child (256x256).
fn skull_model(kind: SkullKind) -> &'static CubeModel {
    static MOB: OnceLock<CubeModel> = OnceLock::new();
    static HUMANOID: OnceLock<CubeModel> = OnceLock::new();
    static PIGLIN: OnceLock<CubeModel> = OnceLock::new();
    static DRAGON: OnceLock<CubeModel> = OnceLock::new();
    let slot = match kind {
        SkullKind::Skeleton | SkullKind::WitherSkeleton | SkullKind::Creeper => &MOB,
        SkullKind::Zombie | SkullKind::Player => &HUMANOID,
        SkullKind::Piglin => &PIGLIN,
        SkullKind::Dragon => &DRAGON,
    };
    slot.get_or_init(|| {
        let part =
            |name: &str, pivot: Vec3, cubes: Vec<Cube>, children: Vec<ModelPart>| ModelPart {
                name: name.into(),
                id: parts::intern(name),
                pivot,
                rotation: Vec3::ZERO,
                visible: true,
                cubes,
                children,
            };
        let cube = |origin: Vec3, size: Vec3, uv: [u32; 2], inflate: f32| Cube {
            origin,
            size,
            inflate,
            uv,
            mirror: false,
        };
        match kind {
            // SkullModel.createMobHeadLayer: head only, 64x32.
            SkullKind::Skeleton | SkullKind::WitherSkeleton | SkullKind::Creeper => CubeModel {
                texture: Identifier::minecraft("entity/skeleton/skeleton"),
                tex_width: 64,
                tex_height: 32,
                root: part(
                    "root",
                    Vec3::ZERO,
                    Vec::new(),
                    vec![part(
                        "head",
                        Vec3::ZERO,
                        vec![cube(
                            Vec3::new(-4.0, -8.0, -4.0),
                            Vec3::new(8.0, 8.0, 8.0),
                            [0, 0],
                            0.0,
                        )],
                        Vec::new(),
                    )],
                ),
            },
            // SkullModel.createHumanoidHeadLayer: head + hat, 64x64.
            SkullKind::Zombie | SkullKind::Player => CubeModel {
                texture: Identifier::minecraft("entity/player/wide/steve"),
                tex_width: 64,
                tex_height: 64,
                root: part(
                    "root",
                    Vec3::ZERO,
                    Vec::new(),
                    vec![part(
                        "head",
                        Vec3::ZERO,
                        vec![
                            cube(
                                Vec3::new(-4.0, -8.0, -4.0),
                                Vec3::new(8.0, 8.0, 8.0),
                                [0, 0],
                                0.0,
                            ),
                            cube(
                                Vec3::new(-4.0, -8.0, -4.0),
                                Vec3::new(8.0, 8.0, 8.0),
                                [32, 0],
                                0.25,
                            ),
                        ],
                        Vec::new(),
                    )],
                ),
            },
            // AbstractPiglinModel.addHead: the head with snout and nostrils, the ears on
            // their pivots (their z rotation is `PiglinHeadModel.setupAnim`'s, applied per
            // instance — it replaces the layer's ±30-degree base), 64x64.
            SkullKind::Piglin => CubeModel {
                texture: Identifier::minecraft("entity/piglin/piglin"),
                tex_width: 64,
                tex_height: 64,
                root: part(
                    "root",
                    Vec3::ZERO,
                    Vec::new(),
                    vec![part(
                        "head",
                        Vec3::ZERO,
                        vec![
                            cube(
                                Vec3::new(-5.0, -8.0, -4.0),
                                Vec3::new(10.0, 8.0, 8.0),
                                [0, 0],
                                0.0,
                            ),
                            cube(
                                Vec3::new(-2.0, -4.0, -5.0),
                                Vec3::new(4.0, 4.0, 1.0),
                                [31, 1],
                                0.0,
                            ),
                            cube(
                                Vec3::new(2.0, -2.0, -5.0),
                                Vec3::new(1.0, 2.0, 1.0),
                                [2, 4],
                                0.0,
                            ),
                            cube(
                                Vec3::new(-3.0, -2.0, -5.0),
                                Vec3::new(1.0, 2.0, 1.0),
                                [2, 0],
                                0.0,
                            ),
                        ],
                        vec![
                            part(
                                "left_ear",
                                Vec3::new(4.5, -6.0, 0.0),
                                vec![cube(
                                    Vec3::new(0.0, 0.0, -2.0),
                                    Vec3::new(1.0, 5.0, 4.0),
                                    [51, 6],
                                    0.0,
                                )],
                                Vec::new(),
                            ),
                            part(
                                "right_ear",
                                Vec3::new(-4.5, -6.0, 0.0),
                                vec![cube(
                                    Vec3::new(-1.0, 0.0, -2.0),
                                    Vec3::new(1.0, 5.0, 4.0),
                                    [39, 6],
                                    0.0,
                                )],
                                Vec::new(),
                            ),
                        ],
                    )],
                ),
            },
            // DragonHeadModel.createHeadLayer: head at PartPose.offset(0, -7.986666, 0) scaled
            // 0.75, the jaw a child at (0, 4, -8), 256x256. The scale cubes are mirrored in
            // vanilla (mirror(true) before the first scale/nostril pair); Haste's Cube mirror
            // swaps the side UVs, which for these single-face boxes is the same drawing.
            SkullKind::Dragon => {
                let head_cubes = vec![
                    cube(
                        Vec3::new(-6.0, -1.0, -24.0),
                        Vec3::new(12.0, 5.0, 16.0),
                        [176, 44],
                        0.0,
                    ),
                    cube(
                        Vec3::new(-8.0, -8.0, -10.0),
                        Vec3::new(16.0, 16.0, 16.0),
                        [112, 30],
                        0.0,
                    ),
                    cube(
                        Vec3::new(-5.0, -12.0, -4.0),
                        Vec3::new(2.0, 4.0, 6.0),
                        [0, 0],
                        0.0,
                    ),
                    cube(
                        Vec3::new(-5.0, -3.0, -22.0),
                        Vec3::new(2.0, 2.0, 4.0),
                        [112, 0],
                        0.0,
                    ),
                    cube(
                        Vec3::new(3.0, -12.0, -4.0),
                        Vec3::new(2.0, 4.0, 6.0),
                        [0, 0],
                        0.0,
                    ),
                    cube(
                        Vec3::new(3.0, -3.0, -22.0),
                        Vec3::new(2.0, 2.0, 4.0),
                        [112, 0],
                        0.0,
                    ),
                ];
                CubeModel {
                    texture: Identifier::minecraft("entity/enderdragon/dragon"),
                    tex_width: 256,
                    tex_height: 256,
                    root: part(
                        "root",
                        Vec3::ZERO,
                        Vec::new(),
                        vec![part(
                            "head",
                            // `PartPose.offset(0, -7.986666, 0).scaled(0.75)`.
                            Vec3::new(0.0, -7.986_666 * 0.75, 0.0),
                            head_cubes,
                            vec![part(
                                "jaw",
                                Vec3::new(0.0, 4.0, -8.0),
                                vec![cube(
                                    Vec3::new(-6.0, 0.0, -16.0),
                                    Vec3::new(12.0, 4.0, 16.0),
                                    [176, 65],
                                    0.0,
                                )],
                                Vec::new(),
                            )],
                        )],
                    ),
                }
            }
        }
    })
}

/// Vanilla `SkullBlockRenderer.createWallTransformation` / `createGroundTransformation`:
/// wall: translate `(0.5 - stepX*0.25, 0.25, 0.5 - stepZ*0.25)`, rotate
/// `-opposite(facing).toYRot()`, scale (-1, -1, 1); ground: translate (0.5, 0, 0.5), rotate
/// `-RotationSegment.convertToDegrees(rotation)`, the same scale. The model renderer's 1/16
/// (pixels to blocks) is folded in after, like the other block entity models.
fn skull_root(
    pos: BlockPos,
    kind_wall: bool,
    facing: Direction,
    rotation_segment: i32,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let flip = Mat4::from_scale(Vec3::new(-1.0 / 16.0, -1.0 / 16.0, 1.0 / 16.0));
    if kind_wall {
        // `facing` is the wall `facing` property; the head looks out of the wall, i.e. the
        // opposite direction's yaw.
        let opposite = facing.opposite();
        let (sx, sz) = (facing.step_x(), facing.step_z());
        Mat4::from_translation(
            p.as_vec3() + Vec3::new(0.5 - sx as f32 * 0.25, 0.25, 0.5 - sz as f32 * 0.25),
        ) * Mat4::from_rotation_y((-to_y_rot(opposite)).to_radians())
            * flip
    } else {
        // `RotationSegment.convertToDegrees`, wrapped from 180 to -180.
        let d = rotation_segment.rem_euclid(16) as f32 * 22.5;
        let d = if d >= 180.0 { d - 360.0 } else { d };
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
            * Mat4::from_rotation_y((-d).to_radians())
            * flip
    }
}

/// Vanilla `DragonHeadModel.setupAnim`'s jaw: `(sin(anim * PI * 0.2) + 1) * 0.2` radians
/// about x.
pub fn skull_dragon_jaw_rotation(anim: f32) -> f32 {
    ((anim * std::f32::consts::PI * 0.2).sin() + 1.0) * 0.2
}

/// Vanilla `PiglinHeadModel.setupAnim`'s ears: the left
/// `-(cos(anim * PI * 0.2 * 1.2) + 2.5) * 0.2` and the right
/// `(cos(anim * PI * 0.2) + 2.5) * 0.2` radians about z (replacing the layer's base ±30
/// degrees).
pub fn skull_piglin_ear_rotations(anim: f32) -> (f32, f32) {
    let left = -((anim * std::f32::consts::PI * 0.2 * 1.2).cos() + 2.5) * 0.2;
    let right = ((anim * std::f32::consts::PI * 0.2).cos() + 2.5) * 0.2;
    (left, right)
}

/// The block entity kinds that draw an item or a stack of items: they go through the entity
/// renderer's item emitters (same models as held and dropped items) rather than the block
/// entity's own texture, so the emitter splits them out first.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ItemBlockEntity {
    Campfire(CampfireItems),
    Shelf(ShelfItems),
    Vault(VaultDisplay),
    Brushable(BrushableItem),
}

/// The item-drawing payload of a block entity kind, when it has one.
pub(crate) fn item_kind(kind: &BlockEntityKind) -> Option<ItemBlockEntity> {
    match kind {
        BlockEntityKind::Campfire(items) => Some(ItemBlockEntity::Campfire(*items)),
        BlockEntityKind::Shelf(items) => Some(ItemBlockEntity::Shelf(*items)),
        BlockEntityKind::Vault(vault) => Some(ItemBlockEntity::Vault(*vault)),
        BlockEntityKind::Brushable(b) => Some(ItemBlockEntity::Brushable(*b)),
        _ => None,
    }
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
            chest::submit(out, inst, camera, rect, light, emit_part);
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
        BlockEntityKind::Skull { kind, wall } => {
            let model = skull_model(kind);
            let mut pose = PoseOverrides::default();
            // The animation value: vanilla lerps `animationTickCount + partial`; the
            // tick's end value is used here (the chest lids' precedent).
            let anim = inst.skull_anim.end;
            match kind {
                SkullKind::Dragon => {
                    // The jaw swings about its pivot.
                    pose.set_rotation(
                        parts::intern("jaw"),
                        Vec3::new(skull_dragon_jaw_rotation(anim), 0.0, 0.0),
                    );
                }
                SkullKind::Piglin => {
                    let (left, right) = skull_piglin_ear_rotations(anim);
                    pose.set_rotation(parts::intern("left_ear"), Vec3::new(0.0, 0.0, left));
                    pose.set_rotation(parts::intern("right_ear"), Vec3::new(0.0, 0.0, right));
                }
                _ => {}
            }
            // The ground rotation angle rides in `sign_angle` (degrees, vanilla's
            // `RotationSegment.convertToDegrees`).
            let segment = (inst.sign_angle / 22.5).round() as i32;
            let root = skull_root(inst.pos, wall, inst.facing, segment, camera);
            for part in &model.root.children {
                emit_part(out, model, part, &pose, root, rect, light);
            }
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

/// One moved-block quad with its variant rotation: `(quad, x, y)` from the blockstate ref.
pub(crate) type PistonQuad = (haste_assets::BakedQuad, i32, i32);

/// The moved block's quads for a state string, resolved once and cached by the caller (the
/// states a piston ever moves are few; vanilla's model resolver caches the same way). Variant
/// x/y rotation is left to the emit matrix; each quad's texture resolves to its entity-atlas
/// rect per frame (`texture_rect` caches those itself).
pub(crate) fn piston_quads(
    assets: &mut haste_assets::Assets,
    state: &str,
) -> Option<Arc<Vec<PistonQuad>>> {
    let (block, props) = parse_state_string(state);
    let quads = (|| -> Option<Vec<PistonQuad>> {
        let def = assets.blockstate(&block).ok()?;
        let refs = def.model_refs_for(&props);
        let mut quads = Vec::new();
        for r in refs {
            let id = Identifier::parse(&r.model).ok()?;
            let q = assets.model_quads(&id).ok()?;
            for quad in q {
                quads.push((quad, r.x, r.y));
            }
        }
        Some(quads)
    })();
    quads.map(Arc::new)
}

// -------------------------------------------------------------------------------------------
// Object-model block entities: the lectern and enchanting table books, bells, decorated pots
// and conduits (vanilla `LecternRenderer`, `EnchantTableRenderer`, `BellRenderer`,
// `DecoratedPotRenderer`, `ConduitRenderer`).
//
// Their layers (`BookModel.createBodyLayer`, `BellModel.createBodyLayer`,
// `DecoratedPotRenderer.createBaseLayer`/`createSidesLayer`, `ConduitRenderer.create*Layer`) are
// *object* models, not entity models: y up and world aligned (vanilla's block entity pose stack
// carries no y flip), the model's front at -z, one model unit = 1/16 block, and vanilla's own
// cube UV layout (the polygon at the layer's min-y end takes the texture's top slot).
// `emit_part` is the entity convention (y down), so these go through `emit_object`, which emits
// vanilla's polygons as they are — same corners, same texture rectangles, same winding fix-up —
// and the root matrix carries only the renderer's own translate/rotate plus the 1/16.

/// One part under construction: pivot, base rotation, and its cubes appended by
/// [`ObjectPart::cube`].
struct ObjectPart {
    name: String,
    pivot: Vec3,
    rotation: Vec3,
    cubes: Vec<Cube>,
}

impl ObjectPart {
    fn cube(mut self, origin: Vec3, size: Vec3, inflate: f32, uv: [u32; 2]) -> Self {
        self.cubes.push(Cube {
            origin,
            size,
            inflate,
            uv,
            mirror: false,
        });
        self
    }

    fn done(self, children: Vec<ModelPart>) -> ModelPart {
        ModelPart {
            id: parts::intern(&self.name),
            name: self.name,
            pivot: self.pivot,
            rotation: self.rotation,
            visible: true,
            cubes: self.cubes,
            children,
        }
    }
}

fn part(name: &str, pivot: Vec3, rotation: Vec3) -> ObjectPart {
    ObjectPart {
        name: name.into(),
        pivot,
        rotation,
        cubes: Vec::new(),
    }
}

fn empty_root(children: Vec<ModelPart>) -> ModelPart {
    ModelPart {
        name: "root".into(),
        id: parts::ROOT,
        pivot: Vec3::ZERO,
        rotation: Vec3::ZERO,
        visible: true,
        cubes: Vec::new(),
        children,
    }
}

/// Vanilla `BellModel.createBodyLayer` (32x32): the body with the base child at the block's
/// corner; the shake rotation is per instance.
fn bell_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/bell/bell_body"),
        tex_width: 32,
        tex_height: 32,
        root: empty_root(vec![part(
            "bell_body",
            Vec3::new(8.0, 12.0, 8.0),
            Vec3::ZERO,
        )
        .cube(
            Vec3::new(-3.0, -6.0, -3.0),
            Vec3::new(6.0, 7.0, 6.0),
            0.0,
            [0, 0],
        )
        .done(vec![
            // The base hangs from the block's corner (the -8, -12, -8 offset cancels
            // BellModel's parent pivot).
            part("bell_base", Vec3::new(-8.0, -12.0, -8.0), Vec3::ZERO)
                .cube(
                    Vec3::new(4.0, 4.0, 4.0),
                    Vec3::new(8.0, 2.0, 8.0),
                    0.0,
                    [0, 13],
                )
                .done(Vec::new()),
        ])]),
    })
}

/// Vanilla `DecoratedPotRenderer.createBaseLayer` (32x32): the neck (a deflated ring plus the
/// inflated flare) above the block's top, and the top and bottom planes capping it.
fn pot_base_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/decorated_pot/decorated_pot_base"),
        tex_width: 32,
        tex_height: 32,
        root: empty_root(vec![
            part(
                "neck",
                Vec3::new(0.0, 37.0, 16.0),
                Vec3::new(std::f32::consts::PI, 0.0, 0.0),
            )
            .cube(
                Vec3::new(4.0, 17.0, 4.0),
                Vec3::new(8.0, 3.0, 8.0),
                -0.1,
                [0, 0],
            )
            .cube(
                Vec3::new(5.0, 20.0, 5.0),
                Vec3::new(6.0, 1.0, 6.0),
                0.2,
                [0, 5],
            )
            .done(Vec::new()),
            // Both planes share vanilla's `topBottomPlane` (texOffs(-14, 13)); the negative u
            // is vanilla's own, stored wrapped in the cube's `u32` offset.
            part("top", Vec3::new(1.0, 16.0, 1.0), Vec3::ZERO)
                .cube(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(14.0, 0.0, 14.0),
                    0.0,
                    [-14i32 as u32, 13],
                )
                .done(Vec::new()),
            part("bottom", Vec3::new(1.0, 0.0, 1.0), Vec3::ZERO)
                .cube(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(14.0, 0.0, 14.0),
                    0.0,
                    [-14i32 as u32, 13],
                )
                .done(Vec::new()),
        ]),
    })
}

/// Vanilla `DecoratedPotRenderer.createSidesLayer` (16x16): one side plane per direction, the
/// same 14x16 quad turned onto each wall. `front` faces the block's `facing`.
fn pot_sides_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| {
        let side = |name: &str, pivot: Vec3, rotation: Vec3| {
            part(name, pivot, rotation)
                .cube(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(14.0, 16.0, 0.0),
                    0.0,
                    [1, 0],
                )
                .done(Vec::new())
        };
        CubeModel {
            texture: Identifier::minecraft("entity/decorated_pot/decorated_pot_side"),
            tex_width: 16,
            tex_height: 16,
            root: empty_root(vec![
                side(
                    "back",
                    Vec3::new(15.0, 16.0, 1.0),
                    Vec3::new(0.0, 0.0, std::f32::consts::PI),
                ),
                side(
                    "left",
                    Vec3::new(1.0, 16.0, 1.0),
                    Vec3::new(0.0, -std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
                ),
                side(
                    "right",
                    Vec3::new(15.0, 16.0, 15.0),
                    Vec3::new(0.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
                ),
                side(
                    "front",
                    Vec3::new(1.0, 16.0, 15.0),
                    Vec3::new(std::f32::consts::PI, 0.0, 0.0),
                ),
            ]),
        }
    })
}

/// Vanilla `ConduitRenderer.createShellLayer` (32x16): the inactive conduit's shell.
fn conduit_shell_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/base"),
        tex_width: 32,
        tex_height: 16,
        root: empty_root(vec![part("shell", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-3.0, -3.0, -3.0),
                Vec3::new(6.0, 6.0, 6.0),
                0.0,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// Vanilla `ConduitRenderer.createCageLayer` (32x16): the active conduit's spinning cage.
fn conduit_cage_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/cage"),
        tex_width: 32,
        tex_height: 16,
        root: empty_root(vec![part("shell", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-4.0, -4.0, -4.0),
                Vec3::new(8.0, 8.0, 8.0),
                0.0,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// Vanilla `ConduitRenderer.createWindLayer` (64x32): the wind box, drawn twice (the second copy
/// counter-rotated by the renderer).
fn conduit_wind_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/wind"),
        tex_width: 64,
        tex_height: 32,
        root: empty_root(vec![part("wind", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-8.0, -8.0, -8.0),
                Vec3::new(16.0, 16.0, 16.0),
                0.0,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// Vanilla `ConduitRenderer.createEyeLayer` (16x16): the billboarded eye plane, inflated by
/// vanilla's `CubeDeformation(0.01)`.
fn conduit_eye_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/conduit/open_eye"),
        tex_width: 16,
        tex_height: 16,
        root: empty_root(vec![part("eye", Vec3::ZERO, Vec3::ZERO)
            .cube(
                Vec3::new(-4.0, -4.0, 0.0),
                Vec3::new(8.0, 8.0, 0.0),
                0.01,
                [0, 0],
            )
            .done(Vec::new())]),
    })
}

/// The texture a bell is drawn with (vanilla `BellRenderer.BELL_TEXTURE`).
pub fn bell_texture() -> Identifier {
    Identifier::minecraft("entity/bell/bell_body")
}

/// The decorated pot's base texture (vanilla `Sheets.DECORATED_POT_BASE`).
pub fn pot_base_texture() -> Identifier {
    Identifier::minecraft("entity/decorated_pot/decorated_pot_base")
}

/// The texture an undecorated pot side is drawn with (vanilla `Sheets.DECORATED_POT_SIDE`).
pub fn pot_side_texture(sherd: Option<&Identifier>) -> Identifier {
    match sherd {
        // Vanilla `DecoratedPotRenderer.getSideSprite`: the blank sprite without a pattern.
        None => Identifier::minecraft("entity/decorated_pot/decorated_pot_side"),
        // `Sheets.DECORATED_POT_MAPPER` applied to the pattern's asset id.
        Some(id) => Identifier::new(
            id.namespace(),
            &format!("entity/decorated_pot/{}", id.path()),
        ),
    }
}

/// The conduit's shell texture (vanilla `ConduitRenderer.SHELL_TEXTURE`).
pub fn conduit_shell_texture() -> Identifier {
    Identifier::minecraft("entity/conduit/base")
}

/// The conduit's cage texture (vanilla `ConduitRenderer.ACTIVE_SHELL_TEXTURE`).
pub fn conduit_cage_texture() -> Identifier {
    Identifier::minecraft("entity/conduit/cage")
}

/// The conduit's wind texture (vanilla `ConduitRenderer.WIND_TEXTURE`, or the vertical one for
/// animation phase 1).
pub fn conduit_wind_texture(phase: i32) -> Identifier {
    if phase == 1 {
        Identifier::minecraft("entity/conduit/wind_vertical")
    } else {
        Identifier::minecraft("entity/conduit/wind")
    }
}

/// The conduit's eye texture: the open one while it attacks (vanilla
/// `ConduitRenderer.OPEN_EYE_TEXTURE`), the closed one otherwise.
pub fn conduit_eye_texture(hunting: bool) -> Identifier {
    if hunting {
        Identifier::minecraft("entity/conduit/open_eye")
    } else {
        Identifier::minecraft("entity/conduit/closed_eye")
    }
}

/// Vanilla `ConduitRenderer.submit`'s `hh`: `sin(animTime * 0.1) / 2 + 0.5`, then squared plus
/// itself.
pub fn conduit_bob(anim_time: f32) -> f32 {
    let hh = ((anim_time * 0.1) as f64).sin() as f32 / 2.0 + 0.5;
    hh * hh + hh
}

/// Vanilla `ConduitBlockEntity.getActiveRotation`: the per-tick counter times `-0.0375`, in
/// degrees (`ConduitRenderer` spins by it directly).
pub fn conduit_spin(active_rotation: f32) -> f32 {
    active_rotation * -0.0375
}

/// Vanilla `BellModel.setupAnim`: `sin(ticks / PI) / (4 + ticks / 3)`, as the (x, z) rotation of
/// the bell body; the click direction picks the axis and sign.
pub fn bell_shake_rotation(ticks: f32, click: Option<Direction>) -> (f32, f32) {
    let Some(click) = click else {
        return (0.0, 0.0);
    };
    let base = (((ticks / std::f32::consts::PI) as f64).sin() as f32) / (4.0 + ticks / 3.0);
    match click {
        Direction::North => (-base, 0.0),
        Direction::South => (base, 0.0),
        Direction::East => (0.0, -base),
        Direction::West => (0.0, base),
        Direction::Up | Direction::Down => (0.0, 0.0),
    }
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

/// Vanilla `DecoratedPotRenderer.modelTransformation`: `180 - facing.toYRot()` degrees about
/// the block centre (its pose stack is the block's corner, so the rotation is about 0.5, 0.5,
/// 0.5).
fn pot_direction_rotation(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3())
        * rotate_around_y(
            (180.0 - to_y_rot(facing)).to_radians(),
            Vec3::new(0.5, 0.5, 0.5),
        )
}

/// A rotation about y by `angle`, around the point `p`.
fn rotate_around_y(angle: f32, p: Vec3) -> Mat4 {
    Mat4::from_translation(p) * Mat4::from_rotation_y(angle) * Mat4::from_translation(-p)
}

/// A rotation about `axis` by `angle`, around the point `p`.
fn rotate_around(angle: f32, axis: Vec3, p: Vec3) -> Mat4 {
    Mat4::from_translation(p) * Mat4::from_axis_angle(axis, angle) * Mat4::from_translation(-p)
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

/// Vanilla `BellRenderer.submit` carries no transform of its own: the block entity's pose stack
/// is the block's corner, so the root is the block position and the 1/16 (the shake rides on
/// the `bell_body` part).
fn bell_root(pos: BlockPos, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3()) * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `DecoratedPotRenderer.submit`: the direction turn, then the wobble. 26.3's
/// `extractRenderState` never assigns `state.wobbleStyle`, so the tilt branch is unreachable
/// and the pot turns about (0.5, 0, 0.5) by `sin(-progress * 3PI) * 0.125 * (1 - progress)`
/// instead.
fn pot_root(pos: BlockPos, facing: Direction, progress: f32, camera: glam::DVec3) -> Mat4 {
    let wobble = if (0.0..=1.0).contains(&progress) {
        let turn = ((-progress * 3.0 * std::f32::consts::PI) as f64).sin() as f32 * 0.125;
        let decay = 1.0 - progress;
        rotate_around(turn * decay, Vec3::Y, Vec3::new(0.5, 0.0, 0.5))
    } else {
        Mat4::IDENTITY
    };
    pot_direction_rotation(pos, facing, camera) * wobble * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `ConduitRenderer.submit`'s inactive shell: (0.5, 0.5, 0.5) and the spin about y.
fn conduit_shell_root(pos: BlockPos, spin: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5))
        * Mat4::from_rotation_y(spin.to_radians())
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The active cage: (0.5, 0.3 + hh * 0.2, 0.5) and the spin about the normalised (0.5, 1, 0.5)
/// axis. `spin` is already an angle in radians: vanilla converts
/// `getActiveRotation`'s value to degrees and straight back to radians, so the cage turns by the
/// same number read as radians.
fn conduit_cage_root(pos: BlockPos, spin: f32, hh: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let axis = Vec3::new(0.5, 1.0, 0.5).normalize();
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.3 + hh * 0.2, 0.5))
        * Mat4::from_axis_angle(axis, spin)
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The wind box: (0.5, 0.5, 0.5) with the animation phase's quarter turn about x (phase 1) or z
/// (phase 2).
fn conduit_wind_root(pos: BlockPos, phase: i32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let mut m = Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5));
    if phase == 1 {
        m *= Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    } else if phase == 2 {
        m *= Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
    }
    m * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The wind box's second copy: scaled 0.875 and turned by `rotationXYZ(PI, 0, PI)`, which is a
/// half turn about y.
fn conduit_wind_back_root(pos: BlockPos, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5))
        * Mat4::from_scale(Vec3::splat(0.875))
        * Mat4::from_rotation_y(std::f32::consts::PI)
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The eye: bobbed like the cage, scaled 0.5, turned to face the camera, then
/// `rotationZ(PI).rotateY(PI)` (a half turn about x) and scaled by vanilla's 1.3333334.
fn conduit_eye_root(pos: BlockPos, hh: f32, orientation: glam::Quat, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.3 + hh * 0.2, 0.5))
        * Mat4::from_scale(Vec3::splat(0.5))
        * Mat4::from_quat(orientation)
        * Mat4::from_rotation_x(std::f32::consts::PI)
        * Mat4::from_scale(Vec3::splat(1.3333334))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Poses one object-model block entity into `out`, one part at a time. `offset` is added
/// to every pivot (vanilla's `ModelPart.x/y/z` slide, which the book's pages use); `pose` holds
/// the per-frame part rotations, keyed by part name; `rect_of` resolves one texture rectangle
/// per part (the pot's sherd sides differ, everything else repeats the same one). The parts and
/// their names come from the model itself, so nothing is allocated per block entity.
#[allow(clippy::too_many_arguments)]
fn emit_object(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    offset: Vec3,
    pose: &[(&'static str, Vec3)],
    root: Mat4,
    light: (u8, u8),
    rect_of: &mut dyn FnMut(&str) -> [u32; 4],
) {
    for part in &model.root.children {
        let rect = rect_of(&part.name);
        emit_object_part(out, model, part, pose, root, rect, offset, light);
    }
}

/// One part and its children, vanilla's `ModelPart.render` then `ModelPart.Cube.compile`.
#[allow(clippy::too_many_arguments)]
fn emit_object_part(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    part: &ModelPart,
    pose: &[(&'static str, Vec3)],
    parent: Mat4,
    rect: [u32; 4],
    offset: Vec3,
    light: (u8, u8),
) {
    let over = pose.iter().find(|(n, _)| *n == part.name).map(|(_, r)| *r);
    let rot = part.rotation + over.unwrap_or(Vec3::ZERO);
    let m = parent
        * Mat4::from_translation(part.pivot + offset)
        * Mat4::from_rotation_z(rot.z)
        * Mat4::from_rotation_y(rot.y)
        * Mat4::from_rotation_x(rot.x);
    let normal_matrix = Mat3::from_mat4(m).inverse().transpose();
    let kx = rect[2] as f32 / model.tex_width.max(1) as f32;
    let ky = rect[3] as f32 / model.tex_height.max(1) as f32;
    let r16 = rect.map(|v| v as u16);
    for c in &part.cubes {
        let g = c.inflate;
        let (x0, y0, z0) = (c.origin.x - g, c.origin.y - g, c.origin.z - g);
        let (x1, y1, z1) = (
            c.origin.x + c.size.x + g,
            c.origin.y + c.size.y + g,
            c.origin.z + c.size.z + g,
        );
        let (u, v) = (c.uv[0] as i32 as f32, c.uv[1] as i32 as f32);
        let (w, h, d) = (c.size.x, c.size.y, c.size.z);
        // Vanilla `ModelPart.Cube`'s corner and UV layout (object convention, y up).
        let (t0, t1, t2, t3) = (
            Vec3::new(x0, y0, z0),
            Vec3::new(x1, y0, z0),
            Vec3::new(x1, y1, z0),
            Vec3::new(x0, y1, z0),
        );
        let (l0, l1, l2, l3) = (
            Vec3::new(x0, y0, z1),
            Vec3::new(x1, y0, z1),
            Vec3::new(x1, y1, z1),
            Vec3::new(x0, y1, z1),
        );
        let (u0, u1, u2, u22, u3, u4) = (
            u,
            u + d,
            u + d + w,
            u + d + 2.0 * w,
            u + d + w + d,
            u + d + w + d + w,
        );
        let (v0, v1, v2) = (v, v + d, v + d + h);
        // (corners in vanilla's order, uv rectangle, normal), one per direction.
        let faces: [([Vec3; 4], [f32; 4], Vec3); 6] = [
            ([l1, l0, t0, t1], [u1, v0, u2, v1], Vec3::NEG_Y),
            ([t2, t3, l3, l2], [u2, v1, u22, v0], Vec3::Y),
            ([t0, l0, l3, t3], [u0, v1, u1, v2], Vec3::NEG_X),
            ([t1, t0, t3, t2], [u1, v1, u2, v2], Vec3::NEG_Z),
            ([l1, t1, t2, l2], [u2, v1, u3, v2], Vec3::X),
            ([l0, l1, l2, l3], [u3, v1, u4, v2], Vec3::Z),
        ];
        for (corners, uvr, n) in faces {
            let p = corners.map(|c| m.transform_point3(c));
            let nw = (normal_matrix * n).normalize_or_zero();
            let s = entity_shade(nw);
            let [r, g, b] = hurt_color(s, 0.0);
            let color = [r, g, b, 1.0];
            // Vanilla's `ModelPart.Polygon` remap: the first and last corners take
            // (u1, v0)/(u1, v1), the middle two (u0, v0)/(u0, v1).
            let uv = [
                [rect[0] as f32 + uvr[2] * kx, rect[1] as f32 + uvr[1] * ky],
                [rect[0] as f32 + uvr[0] * kx, rect[1] as f32 + uvr[1] * ky],
                [rect[0] as f32 + uvr[0] * kx, rect[1] as f32 + uvr[3] * ky],
                [rect[0] as f32 + uvr[2] * kx, rect[1] as f32 + uvr[3] * ky],
            ];
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
        emit_object_part(out, model, child, pose, m, rect, offset, light);
    }
}

/// One model's part rotations for the shared pose, as name-rotation pairs. The object models
/// have a handful of parts, so a fixed array keeps the pose off the heap (per frame and per
/// block entity); the unused slots carry a name no part has.
type ObjectPose = [(&'static str, Vec3); 8];

/// The pose for `entries`, padded with names no part has.
fn pose_of(entries: &[(&'static str, Vec3)]) -> ObjectPose {
    let mut out: ObjectPose = [("", Vec3::ZERO); 8];
    for (slot, e) in out.iter_mut().zip(entries) {
        *slot = *e;
    }
    out
}

/// Poses one object-model block entity (the five kinds from [`BlockEntityKind::Lectern`]) into
/// the entity vertex stream: one batched path with everything else. `rect_of` resolves the
/// textures that differ per part (the pot's sherd sides); `orientation` is the camera's
/// orientation quaternion for the conduit's billboarded eye.
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
        BlockEntityKind::Bell => {
            let model = bell_model();
            let (ticks, click) = match inst.anim {
                Some(BlockEntityAnim::Bell { ticks, click }) => (ticks.end, Some(click)),
                _ => (0.0, None),
            };
            let (x_rot, z_rot) = bell_shake_rotation(ticks, click.flatten());
            let pose = [("bell_body", Vec3::new(x_rot, 0.0, z_rot))];
            emit_object(
                out,
                model,
                Vec3::ZERO,
                &pose,
                bell_root(inst.pos, camera),
                light,
                &mut |_| rect,
            );
        }
        BlockEntityKind::DecoratedPot => {
            // Vanilla's wobble progress: (gameTime - start + partial) / style.duration. The turn
            // branch vanilla's (unassigned) `state.wobbleStyle` leaves reachable is the NEGATIVE
            // one, but the progress still runs over the evented style's own duration.
            let root = match inst.anim {
                Some(BlockEntityAnim::Pot { wobble, duration }) => {
                    pot_root(inst.pos, inst.facing, wobble.end / duration as f32, camera)
                }
                _ => pot_root(inst.pos, inst.facing, 0.0, camera),
            };
            let base = pot_base_model();
            emit_object(out, base, Vec3::ZERO, &[], root, light, &mut |_| rect);
            // The four sherd sides, each with its own texture (the blank sprite without one).
            let sides = pot_sides_model();
            let sherds = inst.sherds.as_deref();
            emit_object(out, sides, Vec3::ZERO, &[], root, light, &mut |name| {
                let sherd = match name {
                    "front" => sherds.and_then(|s| s.front.as_ref()),
                    "back" => sherds.and_then(|s| s.back.as_ref()),
                    "left" => sherds.and_then(|s| s.left.as_ref()),
                    "right" => sherds.and_then(|s| s.right.as_ref()),
                    _ => None,
                };
                rect_of(&pot_side_texture(sherd))
            });
        }
        BlockEntityKind::Conduit => {
            let (tick_count, rotation, active, hunting) = match inst.anim {
                Some(BlockEntityAnim::Conduit {
                    tick_count,
                    rotation,
                    active,
                    hunting,
                }) => (tick_count, rotation, active, hunting),
                _ => (0.0, 0.0, false, false),
            };
            // Vanilla's `getActiveRotation`: the counter times -0.0375. The renderer names that
            // value in degrees for the shell (`rotateDegrees`) and in radians for the cage (its
            // `activeRotation * 180/PI`, then back through `* PI/180`), so the cage turns by the
            // same number read as radians.
            let spin = conduit_spin(rotation);
            let anim_time = tick_count;
            let phase = (tick_count as i32 / 66).rem_euclid(3);
            if !active {
                emit_object(
                    out,
                    conduit_shell_model(),
                    Vec3::ZERO,
                    &[],
                    conduit_shell_root(inst.pos, spin, camera),
                    light,
                    &mut |_| rect,
                );
                return;
            }
            let hh = conduit_bob(anim_time);
            // The cage: the spin in radians about the normalised (0.5, 1, 0.5) axis.
            emit_object(
                out,
                conduit_cage_model(),
                Vec3::ZERO,
                &[],
                conduit_cage_root(inst.pos, spin, hh, camera),
                light,
                &mut |_| rect_of(&conduit_cage_texture()),
            );
            // The wind, twice (the second copy counter-rotated).
            let wind = conduit_wind_model();
            let wind_rect = rect_of(&conduit_wind_texture(phase));
            emit_object(
                out,
                wind,
                Vec3::ZERO,
                &[],
                conduit_wind_root(inst.pos, phase, camera),
                light,
                &mut |_| wind_rect,
            );
            let back = conduit_wind_back_root(inst.pos, camera);
            for part in &wind.root.children {
                emit_object_part(out, wind, part, &[], back, wind_rect, Vec3::ZERO, light);
            }
            // The eye, billboarded to the camera.
            let eye_rect = rect_of(&conduit_eye_texture(hunting));
            emit_object(
                out,
                conduit_eye_model(),
                Vec3::ZERO,
                &[],
                conduit_eye_root(inst.pos, hh, orientation, camera),
                light,
                &mut |_| eye_rect,
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::emit_part;

    fn emit(inst: &BlockEntityInstance) -> Vec<EntityVertex> {
        let mut out = Vec::new();
        let f = |out: &mut Vec<EntityVertex>,
                 model: &CubeModel,
                 part: &ModelPart,
                 pose: &PoseOverrides,
                 m: Mat4,
                 rect: [u32; 4],
                 light: (u8, u8)| {
            emit_part(out, model, part, pose, m, rect, light, 0.0);
        };
        emit_block_entity(&mut out, inst, glam::DVec3::ZERO, [0, 0, 64, 64], &f);
        out
    }

    fn chest(facing: Direction, half: ChestHalf, open: f32) -> BlockEntityInstance {
        BlockEntityInstance {
            pos: BlockPos::new(0, 0, 0),
            facing,
            half,
            kind: BlockEntityKind::Chest(ChestMaterial::Normal),
            open: TickPair::fixed(open),
            light: (15, 0),
            sign_angle: 0.0,
            text: None,
            banner_phase: 0.0,
            banner: None,
            skull_anim: TickPair::fixed(0.0),
            anim: None,
            sherds: None,
            beam: None,
            beam_time: 0.0,
            gateway_beam: None,
            portal_faces: 0,
            piston: None,
        }
    }

    fn shulker(facing: Direction, progress: f32) -> BlockEntityInstance {
        BlockEntityInstance {
            pos: BlockPos::new(0, 0, 0),
            facing,
            half: ChestHalf::Single,
            kind: BlockEntityKind::Shulker(None),
            open: TickPair::fixed(progress),
            light: (15, 0),
            sign_angle: 0.0,
            text: None,
            banner_phase: 0.0,
            banner: None,
            skull_anim: TickPair::fixed(0.0),
            anim: None,
            sherds: None,
            beam: None,
            beam_time: 0.0,
            gateway_beam: None,
            portal_faces: 0,
            piston: None,
        }
    }

    /// Closed single chest: three cubes, six faces each, inside the block (the bottom spans
    /// x/z 1..15, y 0..10; the lid adds up to y 14; the lock reaches the block's front edge).
    #[test]
    fn closed_chest_fits_its_block() {
        let out = emit(&chest(Direction::South, ChestHalf::Single, 0.0));
        assert_eq!(out.len(), 3 * 6 * 6);
        for v in &out {
            let [x, y, z] = v.pos;
            assert!((0.99 / 16.0..=15.01 / 16.0).contains(&x), "x: {x}");
            assert!((-0.01 / 16.0..=14.01 / 16.0).contains(&y), "y: {y}");
            assert!((0.99 / 16.0..=16.01 / 16.0).contains(&z), "z: {z}");
        }
    }

    /// The lid swings up and over the hinge at the back: fully open, the lid's front edge
    /// sits above the block and behind the hinge line (vanilla's `-PI/2` x rotation).
    #[test]
    fn open_chest_lid_swings_back() {
        let out = emit(&chest(Direction::South, ChestHalf::Single, 1.0));
        let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!(max_y > 1.0, "lid does not rise above the block: {max_y}");
        // The lock (front, z 14..15 closed) ends up behind the block's centre.
        // Every vertex of the moved lid stays behind its hinge z = 1/16 + 14/16.
        let hinge = 15.0 / 16.0;
        let front_z = out.iter().map(|v| v.pos[2]).fold(0.0f32, f32::max);
        assert!(front_z <= hinge + 1e-4, "lid front z {front_z} > {hinge}");
    }

    /// A north-facing chest turns its front (the lock, at the block's front edge) from the
    /// south side to the north side: the rotation is vanilla's `-toYRot(north) = -180
    /// degrees` about the block centre, so the lock's face that touched z = 1 touches z = 0.
    #[test]
    fn north_facing_chest_turns_its_front() {
        let south = emit(&chest(Direction::South, ChestHalf::Single, 0.0));
        let north = emit(&chest(Direction::North, ChestHalf::Single, 0.0));
        // The lock protrudes past the box to the block edge: the chest's extreme z is its
        // front. Facing south it is z = 16/16; facing north the chest is rotated within the
        // block, so the extreme front is z = 0 and the box's back fills up to 15/16.
        let max_z = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[2]).fold(0.0f32, f32::max);
        let min_z = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[2]).fold(f32::MAX, f32::min);
        assert!(max_z(&south) > 15.5 / 16.0);
        assert!(
            min_z(&north) < 0.5 / 16.0,
            "front stays south: {}",
            min_z(&north)
        );
        assert!(max_z(&north) > 14.5 / 16.0, "chest lost its body");
    }

    /// The lid curve: vanilla `1 - (1 - open)^3`.
    #[test]
    fn chest_open_curve_matches_vanilla() {
        assert_eq!(chest_open_curve(0.0), 0.0);
        assert_eq!(chest_open_curve(1.0), 1.0);
        assert!((chest_open_curve(0.5) - 0.875).abs() < 1e-6);
        assert!((chest_open_curve(0.1) - 0.271).abs() < 1e-6);
    }

    /// Closed shulker box: base in the lower half, lid up to the block top; the 0.9995
    /// shrink keeps everything just inside the block (0..1 on every axis, centred).
    #[test]
    fn closed_shulker_fits_its_block() {
        let out = emit(&shulker(Direction::Up, 0.0));
        assert_eq!(out.len(), 2 * 6 * 6);
        for v in &out {
            let [x, y, z] = v.pos;
            assert!((-1e-4..=1.0).contains(&y), "shulker y out: {y}");
            assert!((-1e-4..=1.0 + 1e-4).contains(&x), "shulker x out: {x}");
            assert!((-1e-4..=1.0 + 1e-4).contains(&z), "shulker z out: {z}");
        }
        let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!(max_y > 0.99, "lid does not reach the block top: {max_y}");
    }

    /// Open shulker box: the lid rises half a block (vanilla `MAX_LID_HEIGHT`) and spins.
    #[test]
    fn open_shulker_lid_rises_half_a_block() {
        let closed = emit(&shulker(Direction::Up, 0.0));
        let open = emit(&shulker(Direction::Up, 1.0));
        let top = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        assert!((top(&open) - top(&closed) - 0.5).abs() < 1e-3);
    }

    /// A sideways shulker box lies in the block, not above it: every facing's rotation is
    /// vanilla's `Direction.getRotation`, and the box keeps the block's bounds.
    #[test]
    fn sideways_shulker_stays_in_the_block() {
        for facing in Direction::ALL {
            let out = emit(&shulker(facing, 0.0));
            for v in &out {
                let [x, y, z] = v.pos;
                assert!((-1e-4..=1.0 + 1e-4).contains(&y), "{facing:?} y out: {y}");
                assert!((-1e-4..=1.0 + 1e-4).contains(&x), "{facing:?} x out: {x}");
                assert!((-1e-4..=1.0 + 1e-4).contains(&z), "{facing:?} z out: {z}");
            }
        }
    }

    /// Type resolution: the four families map, unknown types and copper chests do not.
    #[test]
    fn kinds_resolve() {
        let chest = Identifier::minecraft("chest");
        let block = Identifier::minecraft("chest");
        assert_eq!(
            kind_of(&chest, &block, false),
            Some(BlockEntityKind::Chest(ChestMaterial::Normal))
        );
        assert_eq!(
            kind_of(&Identifier::minecraft("trapped_chest"), &block, false),
            Some(BlockEntityKind::Chest(ChestMaterial::Trapped))
        );
        assert_eq!(
            kind_of(&Identifier::minecraft("ender_chest"), &block, false),
            Some(BlockEntityKind::Chest(ChestMaterial::Ender))
        );
        // Christmas overrides normal and trapped (vanilla `getChestMaterial`).
        assert_eq!(
            kind_of(&chest, &block, true),
            Some(BlockEntityKind::Chest(ChestMaterial::Christmas))
        );
        // Shulker colour from the block id.
        assert_eq!(
            kind_of(
                &Identifier::minecraft("shulker_box"),
                &Identifier::minecraft("lime_shulker_box"),
                false
            ),
            Some(BlockEntityKind::Shulker(Some(5)))
        );
        assert_eq!(
            kind_of(
                &Identifier::minecraft("shulker_box"),
                &Identifier::minecraft("shulker_box"),
                false
            ),
            Some(BlockEntityKind::Shulker(None))
        );
        // Signs and hanging signs.
        assert_eq!(
            kind_of(&Identifier::minecraft("sign"), &block, false),
            Some(BlockEntityKind::Sign { hanging: false })
        );
        assert_eq!(
            kind_of(&Identifier::minecraft("hanging_sign"), &block, false),
            Some(BlockEntityKind::Sign { hanging: true })
        );
        // Banners: standing and wall (the wall from the block id).
        assert_eq!(
            kind_of(
                &Identifier::minecraft("banner"),
                &Identifier::minecraft("red_banner"),
                false
            ),
            Some(BlockEntityKind::Banner { wall: false })
        );
        assert_eq!(
            kind_of(
                &Identifier::minecraft("banner"),
                &Identifier::minecraft("blue_wall_banner"),
                false
            ),
            Some(BlockEntityKind::Banner { wall: true })
        );
        // Skulls: every kind from its block id, standing and wall.
        for (block, kind) in [
            ("skeleton_skull", SkullKind::Skeleton),
            ("wither_skeleton_skull", SkullKind::WitherSkeleton),
            ("zombie_head", SkullKind::Zombie),
            ("creeper_head", SkullKind::Creeper),
            ("dragon_head", SkullKind::Dragon),
            ("piglin_head", SkullKind::Piglin),
            ("player_head", SkullKind::Player),
        ] {
            assert_eq!(
                kind_of(
                    &Identifier::minecraft("skull"),
                    &Identifier::minecraft(block),
                    false
                ),
                Some(BlockEntityKind::Skull { kind, wall: false }),
                "{block}"
            );
            let wall_block = block.replace("_skull", "_wall_skull");
            let wall_block = if block.ends_with("_skull") {
                wall_block
            } else {
                block.replace("_head", "_wall_head")
            };
            assert_eq!(
                kind_of(
                    &Identifier::minecraft("skull"),
                    &Identifier::minecraft(&wall_block),
                    false
                ),
                Some(BlockEntityKind::Skull { kind, wall: true }),
                "{wall_block}"
            );
        }
        // Unknown and out of scope.
        assert_eq!(
            kind_of(&Identifier::minecraft("copper_chest"), &block, false),
            None
        );
        assert_eq!(
            kind_of(&Identifier::new("mod", "chest"), &block, false),
            None
        );
    }

    /// The skull models follow vanilla's layers: the mob head is one cube on 64x32, the
    /// humanoid head adds the hat on 64x64, the piglin head has snout and ears on 64x64, the
    /// dragon head is the 0.75-scaled part set with the jaw on 256x256.
    #[test]
    fn skull_models_match_vanilla_layers() {
        let mob = skull_model(SkullKind::Skeleton);
        assert_eq!((mob.tex_width, mob.tex_height), (64, 32));
        assert_eq!(mob.root.children.len(), 1);
        let head = &mob.root.children[0];
        assert_eq!(head.cubes.len(), 1);
        assert_eq!(head.cubes[0].origin, Vec3::new(-4.0, -8.0, -4.0));
        assert_eq!(head.cubes[0].size, Vec3::new(8.0, 8.0, 8.0));

        let humanoid = skull_model(SkullKind::Player);
        assert_eq!((humanoid.tex_width, humanoid.tex_height), (64, 64));
        // Head + hat.
        assert_eq!(humanoid.root.children[0].cubes.len(), 2);
        assert_eq!(humanoid.root.children[0].cubes[1].uv, [32, 0]);
        assert_eq!(humanoid.root.children[0].cubes[1].inflate, 0.25);

        let piglin = skull_model(SkullKind::Piglin);
        assert_eq!((piglin.tex_width, piglin.tex_height), (64, 64));
        let phead = &piglin.root.children[0];
        // Head, snout and two nostrils.
        assert_eq!(phead.cubes.len(), 4);
        // The ears on their pivots.
        assert_eq!(phead.children.len(), 2);
        assert_eq!(phead.children[0].pivot, Vec3::new(4.5, -6.0, 0.0));

        let dragon = skull_model(SkullKind::Dragon);
        assert_eq!((dragon.tex_width, dragon.tex_height), (256, 256));
        let dhead = &dragon.root.children[0];
        // The pivot scaled 0.75: -7.986666 * 0.75.
        assert!((dhead.pivot.y - -5.99).abs() < 0.01, "{}", dhead.pivot.y);
        assert_eq!(dhead.cubes.len(), 6);
        // The jaw child at (0, 4, -8).
        assert_eq!(dhead.children[0].pivot, Vec3::new(0.0, 4.0, -8.0));
    }

    /// The skull transform: a ground skull sits in the block's bottom half (the head cube's
    /// y -8..0 maps to 0..0.5 blocks); a north-wall skull is offset 0.25 into the block and
    /// turned to look away from the wall.
    #[test]
    fn skull_root_places_the_head() {
        let pos = BlockPos::new(0, 0, 0);
        // Ground, rotation 0 (south): the head's centre (0, -4, 0 in model space) lands at
        // (0.5, 0.25, 0.5) — the block centre, a quarter up.
        let m = skull_root(pos, false, Direction::South, 0, glam::DVec3::ZERO);
        let p = m.transform_point3(Vec3::new(0.0, -4.0, 0.0));
        assert!((p.x - 0.5).abs() < 1e-6, "x: {}", p.x);
        assert!((p.y - 0.25).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - 0.5).abs() < 1e-6, "z: {}", p.z);
        // The head's top (model y -8) is at 0.5 blocks.
        let top = m.transform_point3(Vec3::new(0.0, -8.0, 0.0));
        assert!((top.y - 0.5).abs() < 1e-6, "top: {}", top.y);
        // A south wall (the skull hangs on the south wall of the block behind it, facing
        // north): the origin is 0.25 off the wall.
        let w = skull_root(pos, true, Direction::South, 0, glam::DVec3::ZERO);
        let q = w.transform_point3(Vec3::new(0.0, -4.0, 0.0));
        // The wall's facing points south (+z): the offset moves the head north
        // (0.5 - 1*0.25 = 0.25), and it sits 0.25 + 0.25 up.
        assert!((q.x - 0.5).abs() < 1e-6, "wx: {}", q.x);
        assert!((q.z - 0.25).abs() < 1e-6, "wz: {}", q.z);
        assert!((q.y - 0.5).abs() < 1e-6, "wy: {}", q.y);
    }

    /// The dragon jaw and piglin ear animations match vanilla's formulas.
    #[test]
    fn skull_animations_match_vanilla() {
        // Jaw at anim 0: (sin(0) + 1) * 0.2 = 0.2.
        assert!((skull_dragon_jaw_rotation(0.0) - 0.2).abs() < 1e-6);
        // Ears at anim 0: left -(cos(0) + 2.5) * 0.2 = -0.7, right (cos(0) + 2.5) * 0.2 = 0.7.
        let (l, r) = skull_piglin_ear_rotations(0.0);
        assert!((l - -0.7).abs() < 1e-6, "left: {l}");
        assert!((r - 0.7).abs() < 1e-6, "right: {r}");
    }

    /// The calendar rule is December 24..26 (vanilla `SpecialDates.isExtendedChristmas`);
    /// the civil-date helper is checked on known epochs since the current date is not
    /// controllable from a test. 2025-01-01 is 1735689600.
    #[test]
    fn xmas_covers_december_24_to_26() {
        let day = |secs: u64| {
            local_month_day(
                std::time::SystemTime::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(secs))
                    .unwrap(),
            )
        };
        assert_eq!(day(1_735_689_600), (1, 1)); // 2025-01-01
        assert_eq!(day(1_734_998_400), (12, 24)); // 2024-12-24
        assert_eq!(day(1_735_084_800), (12, 25)); // 2024-12-25
        assert_eq!(day(1_735_171_200), (12, 26)); // 2024-12-26
        assert_eq!(day(1_735_257_600), (12, 27)); // 2024-12-27
        assert_eq!(day(1_734_912_000), (12, 23)); // 2024-12-23
    }

    fn side(color: u8, glowing: bool) -> SignSideText {
        SignSideText {
            lines: std::array::from_fn(|_| String::new()),
            color,
            glowing,
        }
    }

    /// A standing banner's model spans the block: the pole rises from the ground to y = 42/24
    /// blocks (scale 2/3: 42 pixels * 2/3 / 16 = 1.75), the flag hangs below the bar.
    #[test]
    fn banner_model_standing_geometry() {
        let model = banner_model(false);
        assert_eq!(model.root.children.len(), 3);
        let names: Vec<&str> = model.root.children.iter().map(|p| &*p.name).collect();
        assert_eq!(names, vec!["pole", "bar", "flag"]);
        // The pole: addBox(-1, -42, -1, 2, 42, 2).
        let pole = &model.root.children[0];
        assert_eq!(pole.cubes[0].origin, Vec3::new(-1.0, -42.0, -1.0));
        assert_eq!(pole.cubes[0].size, Vec3::new(2.0, 42.0, 2.0));
        // The bar: addBox(-10, -44, -1, 20, 2, 2).
        let bar = &model.root.children[1];
        assert_eq!(bar.cubes[0].origin, Vec3::new(-10.0, -44.0, -1.0));
        assert_eq!(bar.cubes[0].size, Vec3::new(20.0, 2.0, 2.0));
        // The flag: pivot (0, -44, 0), addBox(-10, 0, -2, 20, 40, 1).
        let flag = &model.root.children[2];
        assert_eq!(flag.pivot, Vec3::new(0.0, -44.0, 0.0));
        assert_eq!(flag.cubes[0].origin, Vec3::new(-10.0, 0.0, -2.0));
        assert_eq!(flag.cubes[0].size, Vec3::new(20.0, 40.0, 1.0));
        // The wall banner drops the pole and hangs the bar and flag on the wall's z.
        let wall = banner_model(true);
        assert_eq!(wall.root.children.len(), 2);
        assert_eq!(wall.root.children[0].cubes[0].origin.y, -20.5);
        assert_eq!(wall.root.children[0].cubes[0].origin.z, 9.5);
        assert_eq!(wall.root.children[1].pivot, Vec3::new(0.0, -20.5, 10.5));
    }

    /// Vanilla `BannerRenderer.modelTransformation`: translate (0.5, 0, 0.5), rotate -angle,
    /// scale (2/3, -2/3, -2/3) then 1/16 (model pixels to blocks). The ground point of the
    /// pole (0, 0, 0 in model space) maps to the block centre at floor level.
    #[test]
    fn banner_root_places_the_block_centre() {
        let pos = BlockPos::new(0, 0, 0);
        let m = banner_root(pos, 0.0, glam::DVec3::ZERO);
        // Model point (0, 0, 0): the pole's ground point.
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.x - 0.5).abs() < 1e-6 && p.y.abs() < 1e-6 && (p.z - 0.5).abs() < 1e-6);
        // The pole top (model y -42, flipped up by the scale) rises 42 * 2/3 / 16 = 1.75
        // blocks above the block's floor.
        let top = m.transform_point3(Vec3::new(0.0, -42.0, 0.0));
        assert!((top.y - 1.75).abs() < 1e-6, "pole top: {}", top.y);
        // The flag's x extents (±10 pixels) map to ±10 * 2/3 / 16 = ±0.4167.
        let left = m.transform_point3(Vec3::new(-10.0, -44.0, 0.0));
        assert!((left.x - (0.5 - 10.0 * 2.0 / 3.0 / 16.0)).abs() < 1e-6);
    }

    /// Vanilla `BannerFlagModel.setupAnim`'s swing and `BannerRenderer`'s phase:
    /// `(-0.0125 + 0.01 * cos(2 * PI * phase)) * PI` about x, phase over position hash and
    /// game time mod 100.
    #[test]
    fn banner_wave_matches_vanilla() {
        // phase 0: cos(0) = 1 -> (-0.0125 + 0.01) * PI = -0.0025 * PI.
        assert!(
            (banner_flag_rotation(0.0) - (-0.0025 * std::f32::consts::PI)).abs() < 1e-6,
            "{}",
            banner_flag_rotation(0.0)
        );
        // phase 0.5: cos(PI) = -1 -> (-0.0125 - 0.01) * PI = -0.0225 * PI.
        assert!(
            (banner_flag_rotation(0.5) - (-0.0225 * std::f32::consts::PI)).abs() < 1e-6,
            "{}",
            banner_flag_rotation(0.5)
        );
        // The phase: floorMod(x*7 + y*9 + z*13 + gameTime, 100), over 100, plus partial.
        let a = banner_wave_phase(BlockPos::new(0, 0, 0), 0, 0.0);
        assert_eq!(a, 0.0);
        let b = banner_wave_phase(BlockPos::new(1, 0, 0), 0, 0.0);
        assert_eq!(b, 0.07);
        let c = banner_wave_phase(BlockPos::new(0, 0, 0), 50, 0.5);
        assert_eq!(c, 0.505);
        // Negative positions wrap (floorMod).
        let d = banner_wave_phase(BlockPos::new(-1, 0, 0), 0, 0.0);
        assert_eq!(d, 0.93);
    }

    /// Vanilla `AbstractSignRenderer.getDarkColor`: non-glowing text is the dye colour scaled
    /// by 0.4 per channel; black glowing text keeps its special outline colour instead.
    #[test]
    fn sign_dark_color_scales_by_four_tenths() {
        // White (0xFFFFFF) * 0.4 = 0x666666.
        assert_eq!(sign_dark_color(&side(0, false)), 0xFF66_6666);
        // Red (0xFF0000) * 0.4 = 0x660000.
        assert_eq!(sign_dark_color(&side(14, false)), 0xFF66_0000);
        // Black glowing keeps vanilla's outline colour.
        assert_eq!(sign_dark_color(&side(15, true)), BLACK_TEXT_OUTLINE_COLOR);
        // Glowing text is drawn with the dye's own colour.
        assert_eq!(sign_glow_color(&side(0, true)), 0xFFFF_FFFF);
    }

    /// The sign text transform at yaw 0, south-facing front: text space is x right, y *up*
    /// (vanilla's -s y scale), origin near the block centre. The front's text faces +z; the
    /// back's matrix is the front's turned by 180 degrees.
    #[test]
    fn sign_text_transform_places_the_origin() {
        let pos = BlockPos::new(0, 0, 0);
        let front = sign_text_transform(false, 0.0, true, glam::DVec3::ZERO, pos);
        let p = front.transform_point3(Vec3::ZERO);
        // Standing sign: (0.5, 0.5, 0.5) + (0, 1/3, 1/21) — the text origin sits just north
        // of the board's centre, one third up.
        assert!((p.x - 0.5).abs() < 1e-6);
        assert!((p.y - (0.5 + 1.0 / 3.0)).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - (0.5 + 0.046666667)).abs() < 1e-6, "z: {}", p.z);
        // Text x maps to world +x and text y to world -y (the -s scale).
        let px = front.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        let py = front.transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!(px.x > p.x);
        assert!(py.y < p.y, "text y is up: {}", py.y);
        // The back is the front rotated 180 degrees about y.
        let back = sign_text_transform(false, 0.0, false, glam::DVec3::ZERO, pos);
        let pb = back.transform_point3(Vec3::ZERO);
        assert!((pb.x - p.x).abs() < 1e-6 && (pb.y - p.y).abs() < 1e-6);
        let pbx = back.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        assert!(pbx.x < pb.x, "back text x flips");
    }

    /// A hanging sign's text transform starts higher (0.9375 - 0.3125 - 0.32) and uses the
    /// bigger 0.0140625 scale.
    #[test]
    fn hanging_sign_text_transform() {
        let pos = BlockPos::new(0, 0, 0);
        let m = sign_text_transform(true, 0.0, true, glam::DVec3::ZERO, pos);
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.x - 0.5).abs() < 1e-6);
        assert!((p.y - (0.9375 - 0.3125 - 0.32)).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - (0.5 + 0.073)).abs() < 1e-6, "z: {}", p.z);
    }

    /// Line metrics: standing 10/90 (`SignBlockEntity`), hanging 9/60
    /// (`HangingSignBlockEntity`).
    #[test]
    fn sign_line_metrics_match_vanilla() {
        assert_eq!(sign_line_metrics(false), (10, 90));
        assert_eq!(sign_line_metrics(true), (9, 60));
    }

    /// Vanilla `CampfireRenderer.submit`: every item sits 0.44921875 up, laid flat, in its own
    /// quadrant (slot 0 at the facing's front, then clockwise), scaled by 0.375.
    #[test]
    fn campfire_item_matrix_places_each_slot() {
        let pos = BlockPos::new(0, 0, 0);
        let cam = glam::DVec3::ZERO;
        // The item model's centre (0.5, 0.5, 0.5 in model space) is shifted into the slot's
        // quadrant: -0.3125 + 0.5 * 0.375 = -0.125 in each of the two horizontal axes, laid
        // flat by the 90-degree x rotation.
        let centre = |slot: usize| {
            campfire_item_matrix(pos, Direction::South, slot, cam)
                .transform_point3(Vec3::new(0.5, 0.5, 0.5))
        };
        let s0 = centre(0);
        assert!((s0.x - 0.375).abs() < 1e-6, "x: {}", s0.x);
        // The 90-degree x rotation folds the model's own half depth (0.5 * 0.375) into y.
        assert!(
            (s0.y - (0.44921875 - 0.5 * CAMPFIRE_ITEM_SIZE)).abs() < 1e-6,
            "y: {}",
            s0.y
        );
        assert!((s0.z - 0.375).abs() < 1e-6, "z: {}", s0.z);
        // Slot 1 turns 90 degrees (west), slot 2 180 (north), slot 3 270 (east).
        let s1 = centre(1);
        assert!(
            (s1.x - 0.625).abs() < 1e-6 && (s1.z - 0.375).abs() < 1e-6,
            "{s1:?}"
        );
        let s2 = centre(2);
        assert!(
            (s2.x - 0.625).abs() < 1e-6 && (s2.z - 0.625).abs() < 1e-6,
            "{s2:?}"
        );
        let s3 = centre(3);
        assert!(
            (s3.x - 0.375).abs() < 1e-6 && (s3.z - 0.625).abs() < 1e-6,
            "{s3:?}"
        );
        // The model's 0.375 scale: one block of model space is 0.375 blocks wide.
        let edge = campfire_item_matrix(pos, Direction::South, 0, cam)
            .transform_point3(Vec3::new(1.5, 0.5, 0.5))
            - s0;
        assert!(
            (edge.x - CAMPFIRE_ITEM_SIZE).abs() < 1e-6,
            "scale: {}",
            edge.x
        );
        // A west-facing campfire's slots are shifted by one (slot 0 goes to the west).
        let shifted = campfire_item_matrix(pos, Direction::West, 0, cam)
            .transform_point3(Vec3::new(0.5, 0.5, 0.5));
        assert!((shifted.x - s1.x).abs() < 1e-6, "{} vs {}", shifted.x, s1.x);
        assert!((shifted.z - s1.z).abs() < 1e-6);
    }

    /// Vanilla `SpawnerRenderer.submitEntityInSpawner` + `LivingEntityRenderer.submit`: the
    /// block's centre at 0.4, the model tipped 30 degrees back and scaled down, with the
    /// display entity's fixed 180-degree yaw and the model flip.
    #[test]
    fn spawner_root_tips_and_scales_the_mob() {
        let m = spawner_root(
            BlockPos::new(0, 0, 0),
            0.0,
            SPAWNER_SCALE,
            glam::DVec3::ZERO,
        );
        // The feet (model origin) sit at the block's centre, dropped by 0.2.
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.x - 0.5).abs() < 1e-6, "x: {}", p.x);
        assert!((p.y - (0.4 - 0.2)).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - 0.5).abs() < 1e-6, "z: {}", p.z);
        // One model block up the y axis: `scale * cos(30)` blocks above the feet (the tip
        // takes some of it into z).
        let up = m.transform_point3(Vec3::new(0.0, -16.0, 0.0)) - p;
        assert!(
            (up.y - (SPAWNER_SCALE * 30.0f32.to_radians().cos())).abs() < 1e-6,
            "up: {}",
            up.y
        );
        assert!(
            (up.z - (-SPAWNER_SCALE * 30.0f32.to_radians().sin())).abs() < 1e-6,
            "z: {}",
            up.z
        );
        // Along the model's +z the tip bends the axis down by half its length (sin 30) and
        // shortens it by cos 30 (the yaw also turns it towards -z).
        let z = m.transform_point3(Vec3::new(0.0, 0.0, 16.0)) - p;
        assert!((z.y + SPAWNER_SCALE * 0.5).abs() < 1e-6, "tip: {}", z.y);
        assert!(
            (z.z - (-SPAWNER_SCALE * 30.0f32.to_radians().cos())).abs() < 1e-6,
            "{}",
            z.z
        );
    }

    /// Vanilla `VaultRenderer.submit`: the block's centre at 0.4 turned by the spin; at spin 0
    /// the item cluster is upright and unrotated, at 90 degrees +x points along -z.
    #[test]
    fn vault_root_turns_around_the_block_centre() {
        let m = vault_root(BlockPos::new(0, 0, 0), 0.0, glam::DVec3::ZERO);
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.x - 0.5).abs() < 1e-6 && (p.y - 0.4).abs() < 1e-6 && (p.z - 0.5).abs() < 1e-6);
        let x = m.transform_point3(Vec3::new(1.0, 0.0, 0.0)) - p;
        assert!((x.x - 1.0).abs() < 1e-6 && x.z.abs() < 1e-6, "{x:?}");
        let r = vault_root(BlockPos::new(0, 0, 0), 90.0, glam::DVec3::ZERO);
        let x90 = r.transform_point3(Vec3::new(1.0, 0.0, 0.0)) - p;
        assert!(x90.x.abs() < 1e-6 && (x90.z + 1.0).abs() < 1e-6, "{x90:?}");
    }

    /// Vanilla `ShelfRenderer.submitItem`: the slots spread along x by 0.3125, the item stands
    /// 0.25 in front of the board, and it is centred on the board unless it aligns to the
    /// bottom. The camera is taken relative (the block's own corner at 0).
    #[test]
    fn shelf_item_matrix_spreads_the_slots() {
        let pos = BlockPos::new(0, 0, 0);
        let cam = glam::DVec3::ZERO;
        let m = shelf_item_matrix(pos, Direction::North, 0, false, 0.0, cam);
        let p = m.transform_point3(Vec3::ZERO);
        // North facing: yaw 180, so the slot offset runs along -x and the board is +z.
        assert!((p.x - (0.5 + 0.3125)).abs() < 1e-6, "x: {}", p.x);
        assert!((p.y - 0.5).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - (0.5 + 0.25)).abs() < 1e-6, "z: {}", p.z);
        // Slot 2 mirrors slot 0 across the block's centre.
        let q = shelf_item_matrix(pos, Direction::North, 2, false, 0.0, cam)
            .transform_point3(Vec3::ZERO);
        assert!((q.x - (0.5 - 0.3125)).abs() < 1e-6, "{}", q.x);
        // Align to bottom drops the item by 0.25 blocks (vanilla's ALIGN_ITEMS_TO_BOTTOM,
        // applied outside the item's own scale).
        let bottom = shelf_item_matrix(pos, Direction::North, 1, true, 0.0, cam)
            .transform_point3(Vec3::ZERO);
        assert!((bottom.y - (0.5 - 0.25)).abs() < 1e-6, "y: {}", bottom.y);
        // The vertical offset rides in the same space: one model unit of `offset_y` is 0.25
        // blocks (the shelf's scale).
        let lifted = shelf_item_matrix(pos, Direction::North, 1, false, 1.0, cam)
            .transform_point3(Vec3::ZERO);
        assert!(
            (lifted.y - (0.5 + SHELF_ITEM_SIZE)).abs() < 1e-6,
            "y: {}",
            lifted.y
        );
    }

    /// Vanilla `ShelfRenderer`'s alignment: the model's box is lifted by its min y, and by
    /// half its height as well when the shelf does not align its items to the bottom.
    #[test]
    fn shelf_item_offset_centres_or_bottoms_the_model() {
        let bbox = (Vec3::new(-0.5, -0.25, 0.0), Vec3::new(0.5, 0.75, 0.0));
        assert!((shelf_item_offset_y(bbox, true) - 0.25).abs() < 1e-6);
        // Centred: 0.25 (the lift) minus half the 1.0-tall box's height (vanilla's
        // `-(maxY - minY) / 2`).
        assert!((shelf_item_offset_y(bbox, false) - -0.25).abs() < 1e-6);
    }

    /// Vanilla `BrushableBlockRenderer.submit` + `translations`: each face pushes the item out
    /// of it by 0.25/0.73 plus `dusted / 10 * 0.75`, and the item keeps vanilla's 75 + 11
    /// degrees of yaw (90 more on the x axis).
    #[test]
    fn brushable_item_matrix_follows_the_dusted_progress() {
        let pos = BlockPos::new(0, 0, 0);
        let cam = glam::DVec3::ZERO;
        // North (touched from the north face): the item sits at z 0.25 at progress 0.
        let m = brushable_item_matrix(pos, Direction::North, 0, cam);
        let p = m.transform_point3(Vec3::ZERO);
        assert!((p.y - 0.5).abs() < 1e-6, "y: {}", p.y);
        assert!((p.z - 0.25).abs() < 1e-6, "z: {}", p.z);
        assert!((p.x - 0.5).abs() < 1e-6, "x: {}", p.x);
        // Progress 3 adds 3/10 * 0.75 towards the camera (-z).
        let p3 = brushable_item_matrix(pos, Direction::North, 3, cam).transform_point3(Vec3::ZERO);
        assert!((p3.z - (0.25 - 0.225)).abs() < 1e-6, "z: {}", p3.z);
        // East is pushed the other way (0.73 + offset).
        let e = brushable_item_matrix(pos, Direction::East, 0, cam).transform_point3(Vec3::ZERO);
        assert!((e.x - 0.73).abs() < 1e-6, "x: {}", e.x);
        // Down is unique: -0.23 - offset (the item hangs below the block).
        let d = brushable_item_matrix(pos, Direction::Down, 0, cam).transform_point3(Vec3::ZERO);
        assert!((d.y - (0.5 - 0.23)).abs() < 1e-6, "y: {}", d.y);
    }

    /// The block entity types vanilla has renderers for, and the ones we do not draw (the
    /// other worker's lecterns, books, bells, pots and conduits, and the rest).
    #[test]
    fn kind_of_maps_the_item_and_mob_block_entities() {
        let campfire = Identifier::minecraft("campfire");
        assert_eq!(
            kind_of(&Identifier::minecraft("campfire"), &campfire, false),
            Some(BlockEntityKind::Campfire(CampfireItems::default()))
        );
        for (be, path) in [
            ("mob_spawner", "spawner"),
            ("trial_spawner", "trial_spawner"),
            ("vault", "vault"),
            ("shelf", "shelf"),
            ("brushable_block", "suspicious_sand"),
        ] {
            let block = Identifier::minecraft(path);
            let kind = kind_of(&Identifier::minecraft(be), &block, false).expect(be);
            let payload = matches!(
                kind,
                BlockEntityKind::Spawner(_)
                    | BlockEntityKind::Vault(_)
                    | BlockEntityKind::Shelf(_)
                    | BlockEntityKind::Brushable(_)
            );
            assert!(payload, "{be} mapped to {kind:?}");
        }
        // The object-model block entities resolve too (their own renderer draws them).
        for (be, kind) in [
            ("lectern", BlockEntityKind::Lectern),
            ("enchanting_table", BlockEntityKind::EnchantTable),
            ("bell", BlockEntityKind::Bell),
            ("decorated_pot", BlockEntityKind::DecoratedPot),
            ("conduit", BlockEntityKind::Conduit),
            ("beacon", BlockEntityKind::Beacon),
            ("end_portal", BlockEntityKind::EndPortal),
            ("end_gateway", BlockEntityKind::EndGateway),
            ("piston", BlockEntityKind::MovingPiston),
        ] {
            let block = Identifier::minecraft(be);
            assert_eq!(kind_of(&block, &block, false), Some(kind), "{be}");
        }
        // Not drawn (no vanilla renderer we have).
        for be in ["furnace", "hopper"] {
            assert_eq!(
                kind_of(
                    &Identifier::minecraft(be),
                    &Identifier::minecraft("stone"),
                    false
                ),
                None,
                "{be}"
            );
        }
    }

    /// Vanilla `ItemClusterRenderState.getRenderedAmount`: one copy for a single item, then
    /// one more per full 16 of the count, up to five.
    #[test]
    fn vault_copies_match_vanilla() {
        let vault = |count: u32| {
            VaultDisplay {
                item: haste_core::RegistryIndex(0),
                count,
                spin: TickPair::fixed(0.0),
            }
            .copies()
        };
        assert_eq!(vault(0), 0);
        assert_eq!(vault(1), 1);
        assert_eq!(vault(16), 2);
        assert_eq!(vault(17), 3);
        assert_eq!(vault(32), 3);
        assert_eq!(vault(33), 4);
        assert_eq!(vault(48), 4);
        assert_eq!(vault(49), 5);
        assert_eq!(vault(1000), 5);
    }

    /// Vanilla `TrialSpawnerRenderer.extractSpawnerData`'s `Mth.lerp(partialTicks, oSpin, spin)
    /// * 10`: the accumulating spin is only scaled for the pose.
    #[test]
    fn spawner_spin_is_scaled_by_ten() {
        let spawner = SpawnerDisplay {
            model: None,
            spin: TickPair {
                start: 3.0,
                end: 4.0,
            },
            scale: SPAWNER_SCALE,
        };
        assert!((spawner_spin_at(&spawner, 0.5) - 35.0).abs() < 1e-6);
    }

    /// Vanilla `VaultRenderer`'s `Mth.rotLerp`: the vault's displayed item keeps turning the
    /// short way when the 10-degrees-a-tick spin rolls over 360, unlike a plain lerp.
    #[test]
    fn vault_spin_wraps_through_the_short_arc() {
        let vault = |start: f32, end: f32| VaultDisplay {
            item: haste_core::RegistryIndex(0),
            count: 1,
            spin: TickPair { start, end },
        };
        // Halfway between 350 and 0 (a 10 degree turn through 360): the item is at 355, where a
        // plain lerp would be at 175 and send the item backwards three quarters of a turn.
        assert!((vault(350.0, 360.0).spin_at(0.5) - 355.0).abs() < 1e-6);
        assert!((vault(350.0, 0.0).spin_at(0.5) - 355.0).abs() < 1e-6);
        // A forward 10 degrees well inside the range interpolates plainly.
        assert!((vault(20.0, 30.0).spin_at(0.5) - 25.0).abs() < 1e-6);
        // A backwards turn (a spin the vault can never make) still takes the short arc.
        assert!((vault(100.0, 80.0).spin_at(0.5) - 90.0).abs() < 1e-6);
    }

    /// The spawner scale: vanilla's 0.53125, divided by the larger of the display entity's
    /// box width and height when it exceeds one block.
    #[test]
    fn spawner_scale_divides_big_models() {
        let small = CubeModel {
            texture: Identifier::minecraft("haste:test"),
            tex_width: 16,
            tex_height: 16,
            root: ModelPart {
                name: "root".into(),
                id: parts::ROOT,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: vec![Cube {
                    origin: Vec3::new(-4.0, -4.0, -4.0),
                    size: Vec3::new(8.0, 8.0, 8.0),
                    inflate: 0.0,
                    uv: [0, 0],
                    mirror: false,
                }],
                children: Vec::new(),
            },
        };
        // 0.5 blocks at most: no division.
        assert!((spawner_scale(&small) - SPAWNER_SCALE).abs() < 1e-6);
        let tall = CubeModel {
            texture: small.texture.clone(),
            tex_width: 16,
            tex_height: 16,
            root: ModelPart {
                cubes: vec![Cube {
                    origin: Vec3::new(-2.0, -48.0, -2.0),
                    size: Vec3::new(4.0, 48.0, 4.0),
                    inflate: 0.0,
                    uv: [0, 0],
                    mirror: false,
                }],
                name: "root".into(),
                id: parts::ROOT,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                children: Vec::new(),
            },
        };
        // 3 blocks tall: 0.53125 / 3.
        assert!(
            (spawner_scale(&tall) - SPAWNER_SCALE / 3.0).abs() < 1e-6,
            "{}",
            spawner_scale(&tall)
        );
    }

    /// The item block entity payloads fold out of their kinds (the emitter splits them off
    /// from the block entity's own texture).
    #[test]
    fn item_kind_splits_the_item_payloads() {
        let items = CampfireItems {
            items: [Some(haste_core::RegistryIndex(1)), None, None, None],
        };
        match item_kind(&BlockEntityKind::Campfire(items)) {
            Some(ItemBlockEntity::Campfire(c)) => assert_eq!(c.items[0].unwrap().0, 1),
            other => panic!("{other:?}"),
        }
        assert!(item_kind(&BlockEntityKind::Chest(ChestMaterial::Normal)).is_none());
        assert!(item_kind(&BlockEntityKind::Sign { hanging: true }).is_none());
    }

    fn lec(pos: BlockPos, kind: BlockEntityKind) -> BlockEntityInstance {
        BlockEntityInstance {
            pos,
            facing: Direction::North,
            half: ChestHalf::Single,
            kind,
            open: TickPair::fixed(0.0),
            light: (15, 0),
            sign_angle: 0.0,
            text: None,
            banner_phase: 0.0,
            banner: None,
            skull_anim: TickPair::fixed(0.0),
            anim: None,
            sherds: None,
            beam: None,
            beam_time: 0.0,
            gateway_beam: None,
            portal_faces: 0,
            piston: None,
        }
    }

    /// The five object-model block entities resolve from their type names, and unknown types
    /// still do not.
    #[test]
    fn object_model_kinds_resolve() {
        for (name, kind) in [
            ("lectern", BlockEntityKind::Lectern),
            ("enchanting_table", BlockEntityKind::EnchantTable),
            ("bell", BlockEntityKind::Bell),
            ("decorated_pot", BlockEntityKind::DecoratedPot),
            ("conduit", BlockEntityKind::Conduit),
        ] {
            let block = Identifier::minecraft(name);
            assert_eq!(kind_of(&block, &block, false), Some(kind), "{name}");
        }
    }

    /// Vanilla `BookModel.State.forAnimation`: `(sin(progress * 0.02) * 0.1 + 1.25) * openness`.
    /// At progress 0 the sine is 0, so the base is 1.25; the lectern's fixed state lifts it by
    /// its `openness` of 1.2.
    #[test]
    fn book_openness_matches_vanilla() {
        assert!((book_openness(0.0, 1.0) - 1.25).abs() < 1e-6);
        assert!((book_openness(0.0, 1.2) - 1.5).abs() < 1e-6);
        // A quarter turn of progress (PI/2 / 0.02 ticks) puts the sine at 1.
        let full = std::f32::consts::PI / 2.0 / 0.02;
        assert!(
            (book_openness(full, 1.0) - 1.35).abs() < 1e-5,
            "{}",
            book_openness(full, 1.0)
        );
    }

    /// Vanilla `EnchantTableRenderer.submit`'s page flips: `frac(flip + 0.25) * 1.6 - 0.3`
    /// clamped to 0..1 (and the same at 0.75 for the second page).
    #[test]
    fn enchant_page_flip_matches_vanilla() {
        // flip 0: frac(0.25) = 0.25 -> 0.4 - 0.3 = 0.1; frac(0.75) -> 1.2 - 0.3 = 0.9.
        assert!((enchant_page_flip(0.0, 0.25) - 0.1).abs() < 1e-6);
        assert!((enchant_page_flip(0.0, 0.75) - 0.9).abs() < 1e-6);
        // Clamped at both ends: a raw value below 0 clamps to 0 (flip 0.9 -> frac(1.15) = 0.15,
        // 0.24 - 0.3 < 0), and one above 1 clamps up (flip 0.5625 -> frac(0.8125) = 0.8125,
        // 1.3 - 0.3 > 1).
        assert_eq!(enchant_page_flip(0.9, 0.25), 0.0);
        assert_eq!(enchant_page_flip(0.5625, 0.25), 1.0);
        // `frac` wraps: flip 1.25 is flip 0.25.
        assert!((enchant_page_flip(1.25, 0.0) - 0.1).abs() < 1e-6);
    }

    /// Vanilla `ConduitRenderer.submit`'s bob: `sin(animTime * 0.1) / 2 + 0.5` in 0..1, squared
    /// plus itself, so it runs from 0.75 (sine 0) up to 2 (sine at 1, 5 PI ticks) and down to 0
    /// (sine at -1, 15 PI ticks).
    #[test]
    fn conduit_bob_matches_vanilla() {
        assert!(
            (conduit_bob(0.0) - 0.75).abs() < 1e-6,
            "{}",
            conduit_bob(0.0)
        );
        let peak = conduit_bob(std::f32::consts::PI * 5.0);
        assert!((peak - 2.0).abs() < 1e-4, "{peak}");
        let trough = conduit_bob(std::f32::consts::PI * 15.0);
        assert!(trough.abs() < 1e-4, "{trough}");
    }

    /// Vanilla `ConduitBlockEntity.getActiveRotation`: the counter times -0.0375, in degrees.
    #[test]
    fn conduit_spin_matches_vanilla() {
        assert_eq!(conduit_spin(0.0), 0.0);
        assert_eq!(conduit_spin(1.0), -0.0375);
        // A full turn takes 9600 ticks (360 / 0.0375).
        assert!((conduit_spin(9600.0) + 360.0).abs() < 1e-2);
    }

    /// Vanilla `BellModel.setupAnim`: the shake is `sin(ticks / PI) / (4 + ticks / 3)`, and the
    /// click direction picks the axis and its sign (north/south tilt about x, west/east about
    /// z); without a click direction the bell is at rest.
    #[test]
    fn bell_shake_matches_vanilla() {
        assert_eq!(bell_shake_rotation(0.0, None), (0.0, 0.0));
        // ticks 0: sin(0) = 0, so the shake is 0 whichever direction.
        assert_eq!(bell_shake_rotation(0.0, Some(Direction::North)), (0.0, 0.0));
        // ticks 1: sin(1 / PI) ~ 0.3138, over 4 + 1/3.
        let base = (1.0 / std::f32::consts::PI).sin() / (4.0 + 1.0 / 3.0);
        assert!((bell_shake_rotation(1.0, Some(Direction::South)).0 - base).abs() < 1e-4);
        assert!((bell_shake_rotation(1.0, Some(Direction::North)).0 + base).abs() < 1e-4);
        assert!(bell_shake_rotation(1.0, Some(Direction::North)).1.abs() < 1e-9);
        assert!((bell_shake_rotation(1.0, Some(Direction::East)).1 + base).abs() < 1e-4);
        assert!((bell_shake_rotation(1.0, Some(Direction::West)).1 - base).abs() < 1e-4);
        // The sine's peak at the first ticks dwarfs the decay; by tick 10 the sine has swung
        // past its peak and the growing denominator has taken over.
        let (a, _) = bell_shake_rotation(10.0, Some(Direction::South));
        assert!(a.abs() < base.abs(), "{a} vs {base}");
    }

    /// Vanilla's partial-tick interpolation of the object-model block entities' counters:
    /// `BellRenderState.ticks` is `ticks + partialTicks`, the pot's wobble progress runs over
    /// `(gameTime - wobbleStartedAtTick + partialTicks) / duration`, the enchanting table lerps
    /// its `flip`/`open`/`rot`, and the conduit's `getActiveRotation(partialTicks)` adds the
    /// partial tick to the spin counter.
    #[test]
    fn anims_lerp_by_the_partial_tick() {
        let bell = BlockEntityAnim::Bell {
            ticks: TickPair {
                start: 3.0,
                end: 4.0,
            },
            click: Some(Direction::North),
        };
        for partial in [0.0, 0.25, 0.5, 1.0] {
            match bell.at(partial) {
                BlockEntityAnim::Bell { ticks, click } => {
                    assert!((ticks.end - (3.0 + partial)).abs() < 1e-6, "{ticks:?}");
                    assert_eq!(click, Some(Direction::North));
                }
                other => panic!("{other:?}"),
            }
        }
        let pot = BlockEntityAnim::Pot {
            wobble: TickPair {
                start: 2.0,
                end: 3.0,
            },
            duration: 7,
        };
        match pot.at(0.5) {
            BlockEntityAnim::Pot { wobble, duration } => {
                assert!((wobble.end - 2.5).abs() < 1e-6);
                assert_eq!(duration, 7, "the style's duration travels with the wobble");
            }
            other => panic!("{other:?}"),
        }
        let enchant = BlockEntityAnim::EnchantTable {
            time: 10.0,
            flip: TickPair {
                start: 1.0,
                end: 2.0,
            },
            open: TickPair {
                start: 0.0,
                end: 0.5,
            },
            rot: TickPair {
                start: -1.0,
                end: 1.0,
            },
            t_rot: 0.0,
            flip_t: 0.0,
            flip_a: 0.0,
        };
        match enchant.at(0.25) {
            BlockEntityAnim::EnchantTable {
                time,
                flip,
                open,
                rot,
                ..
            } => {
                assert!((time - 10.25).abs() < 1e-6);
                assert!((flip.end - 1.25).abs() < 1e-6);
                assert!((open.end - 0.125).abs() < 1e-6);
                assert!((rot.end - (-0.5)).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        let conduit = BlockEntityAnim::Conduit {
            tick_count: 5.0,
            rotation: 100.0,
            active: true,
            hunting: true,
        };
        match conduit.at(0.5) {
            BlockEntityAnim::Conduit {
                tick_count,
                rotation,
                active,
                hunting,
            } => {
                assert!((tick_count - 5.5).abs() < 1e-6);
                assert!((rotation - 100.5).abs() < 1e-6);
                assert!(active && hunting);
            }
            other => panic!("{other:?}"),
        }
    }

    /// Vanilla `ConduitRenderer.submit`'s active cage: the spin is taken in radians where the
    /// shell's is degrees (`rotateDegrees(Axis.YP, activeRotation)` against
    /// `rotationAxis(activeRotation * 180/PI * PI/180)`), so only a counter of
    /// `2*PI / -0.0375` ticks brings the cage back to a whole turn, while the shell's own 360
    /// degrees worth of counter (`360 / -0.0375`) does not move the cage by a full turn.
    #[test]
    fn conduit_cage_spins_in_radians() {
        let cage_at = |rotation: f32| {
            let mut inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Conduit);
            inst.anim = Some(BlockEntityAnim::Conduit {
                tick_count: 0.0,
                rotation,
                active: true,
                hunting: false,
            });
            emit_object_vertices(&inst, glam::Quat::IDENTITY)
        };
        // Cage (36 quads' worth of vertices), wind twice and the eye: four cubes of six quads.
        let at_rest = cage_at(0.0);
        assert_eq!(at_rest.len(), 4 * 6 * 6);
        // One whole turn of the cage: 2*PI radians at 0.0375 radians per tick.
        let turn = cage_at(std::f32::consts::TAU / -0.0375);
        let moved = at_rest
            .iter()
            .zip(&turn)
            .map(|(a, b)| (a.pos[0] - b.pos[0]).abs() + (a.pos[1] - b.pos[1]).abs())
            .fold(0.0f32, f32::max);
        assert!(moved < 1e-3, "one turn did not return the cage: {moved}");
        // The same counter in degrees (a full shell turn, 360 degrees) is only 360 radians of
        // cage spin, so the cage is somewhere else entirely.
        let degrees_turn = cage_at(360.0 / -0.0375);
        let moved = at_rest
            .iter()
            .zip(&degrees_turn)
            .map(|(a, b)| (a.pos[0] - b.pos[0]).abs() + (a.pos[1] - b.pos[1]).abs())
            .fold(0.0f32, f32::max);
        assert!(moved > 1e-3, "the cage turned with the shell: {moved}");
        // The cage is one block wide, centred on (0.5, 0.3 + hh * 0.2, 0.5) with hh = 0.75.
        for v in at_rest.iter().take(6 * 6) {
            let c = [v.pos[0] - 0.5, v.pos[1] - 0.45, v.pos[2] - 0.5];
            assert!(
                c.iter().all(|a| a.abs() <= 0.2501),
                "cage outside its cube: {c:?}"
            );
        }
    }

    /// Vanilla `Camera.setRotation`'s orientation: the conduit's eye is billboarded through it,
    /// so its plane's normal comes out facing the camera (`ConduitRenderer.submit`'s eye pose
    /// ends with the half turn that flips the model's +z onto the view forward).
    #[test]
    fn camera_orientation_faces_forward_and_up() {
        let forward = |yaw: f32, pitch: f32| {
            let (sy, cy) = (yaw.to_radians().sin(), yaw.to_radians().cos());
            let (sp, cp) = (pitch.to_radians().sin(), pitch.to_radians().cos());
            Vec3::new(-cp * sy, -sp, cp * cy)
        };
        for (yaw, pitch) in [(0.0, 0.0), (45.0, -30.0), (180.0, 10.0), (270.0, -80.0)] {
            let q = camera_orientation(yaw, pitch);
            // The eye's plane normal (+z) through the renderer's pose: the 0.5 scale, the
            // camera orientation and the `rotationZ(PI).rotateY(PI)` half turn about x.
            let n = (Mat4::from_quat(q) * Mat4::from_rotation_x(std::f32::consts::PI))
                .transform_vector3(Vec3::Z);
            let want = forward(yaw, pitch);
            assert!(
                n.distance(want) < 1e-4,
                "yaw {yaw} pitch {pitch}: eye normal {n:?}, want {want:?}"
            );
        }
    }

    /// Vanilla `ConduitRenderer.submit`'s inactive shell: one cube inside the block, centred.
    #[test]
    fn conduit_shell_fits_its_block() {
        let mut inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Conduit);
        inst.anim = Some(BlockEntityAnim::Conduit {
            tick_count: 0.0,
            rotation: 0.0,
            active: false,
            hunting: false,
        });
        let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
        assert_eq!(out.len(), 6 * 6);
        for v in &out {
            // One 6x6x6 cube inside the block, centred: 3/16 to every side of the centre.
            let c = [v.pos[0] - 0.5, v.pos[1] - 0.5, v.pos[2] - 0.5];
            assert!(c.iter().all(|a| a.abs() <= 0.1876), "shell outside: {c:?}");
        }
    }

    /// Vanilla `BellModel.createBodyLayer`: the body hangs from its pivot at (8, 12, 8) model
    /// units and the base sits on the block's corner, so the bell occupies the block's lower
    /// three quarters, centred.
    #[test]
    fn bell_model_places_the_body_and_base() {
        let inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Bell);
        let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
        // Two cubes.
        assert_eq!(out.len(), 2 * 6 * 6);
        let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        let min_y = out.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
        // The body's top is at 12/16 + 1/16 = 0.8125; the base's bottom at 4/16 = 0.25.
        assert!((max_y - 0.8125).abs() < 1e-4, "top {max_y}");
        assert!((min_y - 0.25).abs() < 1e-4, "bottom {min_y}");
        for v in &out {
            assert!((-1e-4..=1.0 + 1e-4).contains(&v.pos[0]), "x {}", v.pos[0]);
            assert!((-1e-4..=1.0 + 1e-4).contains(&v.pos[2]), "z {}", v.pos[2]);
        }
    }

    /// Vanilla `BookModel.createBodyLayer`: the book is 12 model units wide (0.75 blocks) with
    /// the two lids either side of the seam, and the lectern's transform puts it at the
    /// lectern's top (y 1.0625) turned by the block's `getClockWise` facing.
    #[test]
    fn lectern_book_sits_on_the_lectern_top() {
        let inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Lectern);
        let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
        // Seven parts, six faces each.
        assert_eq!(out.len(), 7 * 6 * 6);
        let min_y = out.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
        let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        // The book's 10-tall lids span 5/16 around the pivot's y: 1.0625 - 0.125 - 5/16 ..
        // 1.0625 - 0.125 + 5/16.
        // The book's lids are 10 model units tall (0.625 blocks) around the pivot at
        // 1.0625 - 0.125, so nothing reaches below 0.6 even turned by the 67.5-degree z tilt.
        assert!(min_y > 0.6, "book hangs below the lectern top: {min_y}");
        assert!(max_y < 1.3, "book rises too high: {max_y}");
        // The book's width: 12 pixels wide and 10 deep, well inside a block.
        let max_x = out.iter().map(|v| v.pos[0]).fold(0.0f32, f32::max);
        let min_x = out.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min);
        assert!(
            (max_x - min_x) < 1.0,
            "book wider than a block: {max_x} {min_x}"
        );
    }

    /// The lectern's facing turns the book: `LecternRenderer.extractRenderState` reads
    /// `facing.getClockWise().toYRot()`, so a north-facing lectern (whose clockwise is east,
    /// yaw 270) turns its book a quarter turn from a south-facing one (west, 90).
    #[test]
    fn lectern_facing_turns_the_book() {
        let mut north = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Lectern);
        north.facing = Direction::North;
        let mut south = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Lectern);
        south.facing = Direction::South;
        let a = emit_object_vertices(&north, glam::Quat::IDENTITY);
        let b = emit_object_vertices(&south, glam::Quat::IDENTITY);
        // The two poses are different, and each keeps the book inside the block's column.
        let differs = a
            .iter()
            .zip(b.iter())
            .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-4 || (p.pos[2] - q.pos[2]).abs() > 1e-4);
        assert!(differs, "the facing does not turn the book");
    }

    /// Vanilla `DecoratedPotRenderer.createBaseLayer`: the base's neck ring sits above the
    /// block's top and its flare is narrower than the body's 14-pixel width.
    #[test]
    fn decorated_pot_neck_rises_above_the_block() {
        let inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::DecoratedPot);
        let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
        // The base layer's neck carries two cubes (the deflated ring and the inflated flare),
        // plus the top and bottom planes and the four side quads.
        assert_eq!(out.len(), 8 * 6 * 6);
        let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
        // The neck's flare tops out at 37/16 - 20/16 + 1/16 = 1.1875 plus its 0.2 inflation.
        assert!(max_y > 1.15, "neck does not rise above the block: {max_y}");
        assert!(max_y < 1.35, "neck rises too high: {max_y}");
        // Everything else is inside the block.
        for v in &out {
            if v.pos[1] < max_y - 1e-3 {
                assert!(
                    (-0.2..=1.2).contains(&v.pos[0]) && (-0.2..=1.2).contains(&v.pos[2]),
                    "pot part outside the block: {:?}",
                    v.pos
                );
            }
        }
    }

    /// The decorated pot's direction turn (vanilla `modelTransformation`): the sides are four
    /// separate quads around the block, and the block's `facing` rotates them.
    #[test]
    fn decorated_pot_facing_turns_the_sides() {
        let mut north = lec(BlockPos::new(0, 0, 0), BlockEntityKind::DecoratedPot);
        north.facing = Direction::North;
        let mut east = lec(BlockPos::new(0, 0, 0), BlockEntityKind::DecoratedPot);
        east.facing = Direction::East;
        let a = emit_object_vertices(&north, glam::Quat::IDENTITY);
        let b = emit_object_vertices(&east, glam::Quat::IDENTITY);
        let differs = a
            .iter()
            .zip(b.iter())
            .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-4 || (p.pos[2] - q.pos[2]).abs() > 1e-4);
        assert!(differs, "the facing does not turn the pot");
    }

    /// The enchanting table's book animates: the bob lifts it off the (0.5, 0.75, 0.5)
    /// transform by up to a hundredth, and the openness closes when no player is near.
    #[test]
    fn enchant_table_book_animates() {
        let mut inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::EnchantTable);
        inst.anim = Some(BlockEntityAnim::EnchantTable {
            time: 0.0,
            flip: TickPair::fixed(0.0),
            open: TickPair::fixed(0.5),
            rot: TickPair::fixed(0.0),
            t_rot: 0.0,
            flip_t: 0.0,
            flip_a: 0.0,
        });
        let closed = emit_object_vertices(&inst, glam::Quat::IDENTITY);
        let mut later = inst.clone();
        later.anim = Some(BlockEntityAnim::EnchantTable {
            time: 25.0,
            flip: TickPair::fixed(0.5),
            open: TickPair::fixed(0.9),
            rot: TickPair::fixed(0.4),
            t_rot: 0.4,
            flip_t: 1.0,
            flip_a: 0.1,
        });
        let open = emit_object_vertices(&later, glam::Quat::IDENTITY);
        let differs = closed.iter().zip(open.iter()).any(|(p, q)| {
            (p.pos[0] - q.pos[0]).abs() > 1e-3
                || (p.pos[1] - q.pos[1]).abs() > 1e-3
                || (p.pos[2] - q.pos[2]).abs() > 1e-3
        });
        assert!(differs, "the book does not animate");
    }

    /// A ringing bell tilts its body (the base stays put): vanilla `BellModel.setupAnim` rotates
    /// only `bell_body`.
    #[test]
    fn ringing_bell_tilts_its_body() {
        let rest = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Bell);
        let mut ringing = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Bell);
        ringing.anim = Some(BlockEntityAnim::Bell {
            ticks: TickPair::fixed(1.0),
            click: Some(Direction::South),
        });
        let a = emit_object_vertices(&rest, glam::Quat::IDENTITY);
        let b = emit_object_vertices(&ringing, glam::Quat::IDENTITY);
        let differs = a
            .iter()
            .zip(b.iter())
            .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-4 || (p.pos[2] - q.pos[2]).abs() > 1e-4);
        assert!(differs, "the bell does not shake");
    }

    /// One object-model block entity's vertices, posed through the same emitter the frame path
    /// uses (the texture rect is a unit square; only the geometry is checked here).
    fn emit_object_vertices(
        inst: &BlockEntityInstance,
        orientation: glam::Quat,
    ) -> Vec<EntityVertex> {
        let mut out = Vec::new();
        let rect = [0, 0, 64, 64];
        emit_model_block_entity(
            &mut out,
            inst,
            glam::DVec3::ZERO,
            orientation,
            rect,
            &mut |_| rect,
        );
        out
    }
}
