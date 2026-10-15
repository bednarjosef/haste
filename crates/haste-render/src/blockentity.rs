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
use haste_core::{BlockPos, BlockStateId, Direction, Identifier};

use crate::entity::{entity_shade, hurt_color, EntityVertex, TickPair};

/// Which half of a double chest a block is (vanilla `ChestType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChestHalf {
    Single,
    Left,
    Right,
}

impl ChestHalf {
    /// The opposite half (vanilla `ChestType.getOpposite`); a single chest has none and keeps
    /// its own value (callers only use this for doubles).
    pub fn other(self) -> ChestHalf {
        match self {
            ChestHalf::Left => ChestHalf::Right,
            ChestHalf::Right => ChestHalf::Left,
            ChestHalf::Single => ChestHalf::Single,
        }
    }
}

/// A chest's texture set (vanilla `ChestRenderState.ChestMaterialType`, minus copper which Haste
/// does not draw yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChestMaterial {
    Normal,
    Trapped,
    Christmas,
    Ender,
}

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

/// The per-instance animation state of the object-model block entities, as vanilla's block
/// entity tickers leave it at the ends of the last tick (the partial tick lerps the two ends).
/// Build it in the client's tick (like [`crate::TickPair`] for the chest lids).
#[derive(Clone, Copy, Debug)]
pub enum BlockEntityAnim {
    /// The enchanting table's book (vanilla `EnchantingTableBlockEntity.bookAnimationTick`):
    /// `time` is the tick counter, the three pairs are `flip`, `open` and `rot`, and the last
    /// three are the tick's own bookkeeping (`tRot`, `flipT` and `flipA`).
    EnchantTable {
        time: f32,
        flip: TickPair,
        open: TickPair,
        rot: TickPair,
        t_rot: f32,
        flip_t: f32,
        flip_a: f32,
    },
    /// A bell that was rung (vanilla `BellBlockEntity.triggerEvent` then `BellBlockEntity.tick`):
    /// the shake ticks and the direction the bell was hit from (`clickDirection`).
    Bell {
        ticks: TickPair,
        click: Option<Direction>,
    },
    /// A wobbling decorated pot (vanilla `DecoratedPotBlockEntity.triggerEvent`): ticks since
    /// the wobble started, at the ends of the last tick, and the wobble style's duration
    /// (`WobbleStyle.duration`, 7 for POSITIVE and 10 for NEGATIVE) the progress is measured in.
    Pot { wobble: TickPair, duration: u32 },
    /// A conduit (vanilla `ConduitBlockEntity.clientTick`): the tick counter, the spin counter
    /// (vanilla's `activeRotation`, which the renderer names in degrees for the shell and in
    /// radians for the cage), and the shape's active/hunting flags.
    Conduit {
        tick_count: f32,
        rotation: f32,
        active: bool,
        hunting: bool,
    },
    /// A moving piston (vanilla `PistonMovingBlockEntity.tick`): the extension progress, which
    /// advances 0.5 per tick to 1.0 and then the block entity is gone (the server replaces the
    /// moving_piston block with the moved one).
    Piston { progress: TickPair },
}

impl BlockEntityAnim {
    /// The same animation at `partial` through the tick, vanilla's `Mth.lerp(partialTicks, oX,
    /// X)` per field: the counters the renderers read (`BellBlockEntity.ticks` +
    /// `partialTicks`, `DecoratedPotBlockEntity.wobbleStartedAtTick` + `partialTicks`, the
    /// enchanting table's `flip`/`open`/`rot`). The game thread keeps the two ends; the render
    /// thread hands this the partial tick it already has.
    pub fn at(self, partial: f32) -> BlockEntityAnim {
        match self {
            BlockEntityAnim::EnchantTable {
                time,
                flip,
                open,
                rot,
                t_rot,
                flip_t,
                flip_a,
            } => BlockEntityAnim::EnchantTable {
                time: time + partial,
                flip: TickPair {
                    start: flip.start,
                    end: flip.at(partial),
                },
                open: TickPair {
                    start: open.start,
                    end: open.at(partial),
                },
                rot: TickPair {
                    start: rot.start,
                    end: rot.at(partial),
                },
                t_rot,
                flip_t,
                flip_a,
            },
            BlockEntityAnim::Bell { ticks, click } => BlockEntityAnim::Bell {
                ticks: TickPair {
                    start: ticks.start,
                    end: ticks.at(partial),
                },
                click,
            },
            BlockEntityAnim::Pot { wobble, duration } => BlockEntityAnim::Pot {
                wobble: TickPair {
                    start: wobble.start,
                    end: wobble.at(partial),
                },
                duration,
            },
            // Vanilla's `ConduitRenderState`: `animTime` is `tickCount + partialTicks` and
            // `getActiveRotation(partialTicks)` adds the partial tick to the spin counter; the
            // animation phase stays on the integer tick count.
            BlockEntityAnim::Conduit {
                tick_count,
                rotation,
                active,
                hunting,
            } => BlockEntityAnim::Conduit {
                tick_count: tick_count + partial,
                rotation: rotation + partial,
                active,
                hunting,
            },
            BlockEntityAnim::Piston { progress } => BlockEntityAnim::Piston {
                progress: TickPair {
                    start: progress.start,
                    end: progress.at(partial),
                },
            },
        }
    }
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

/// One coloured segment of a beacon beam (vanilla `BeaconBeamOwner.Section`): the ARGB colour
/// (the glass above, averaged when it changes mid-beam) and its height in blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeamSection {
    /// The section's colour, vanilla ARGB (`DyeColor.getTextureDiffuseColor` of the glass).
    pub color: u32,
    /// The section's height in blocks (1 for the beacon block's own segment).
    pub height: i32,
}

/// A beam's render data, shared between the game thread (which walks the glass above the
/// beacon, vanilla `BeaconBlockEntity.tick`) and the frame's instances: the sections only;
/// the animation time is per frame, so it rides on the instance
/// ([`BlockEntityInstance::beam_time`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BeamData {
    /// The beam sections bottom-up; empty draws no beam.
    pub sections: Vec<BeamSection>,
}

/// The end gateway beam (vanilla `EndGatewayRenderState`): the spawn/cooldown scale, the beam
/// height in blocks and the ARGB colour; the animation time rides per frame on the instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatewayBeam {
    /// Vanilla `state.scale` after its `sin(scale * PI)`.
    pub scale: f32,
    /// Vanilla `state.height`: `floor(scale * beamDistance)`, 0 hides the beam.
    pub height: i32,
    /// Magenta while spawning, purple while cooling down (`DyeColor.getTextureDiffuseColor`).
    pub color: u32,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// The dye colour id of a shulker box block id (`minecraft:white_shulker_box` -> 0), vanilla
/// `DyeColor`'s order; `None` for the plain `minecraft:shulker_box` (vanilla `getColor`'s null).
fn shulker_color(block: &Identifier) -> Option<u8> {
    const NAMES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let path = block.path();
    let color = path.strip_suffix("_shulker_box")?;
    NAMES.iter().position(|n| *n == color).map(|i| i as u8)
}

/// Vanilla `SpecialDates.isExtendedChristmas`: December 24 to 26, in the local time zone.
/// Evaluated once per process, like vanilla's `ChestRenderer` constructor does.
pub fn xmas_textures() -> bool {
    static XMAS: OnceLock<bool> = OnceLock::new();
    *XMAS.get_or_init(|| {
        let (month, day) = local_month_day(std::time::SystemTime::now());
        month == 12 && (24..=26).contains(&day)
    })
}

/// The local month (1..12) and day of the month, from the epoch time. UTC when the platform has
/// no time zone support (Rust has none in std); the calendar rule is hours-granular, so the
/// difference only matters around local midnight.
fn local_month_day(t: std::time::SystemTime) -> (u32, u32) {
    let days = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Civil-from-days (Howard Hinnant's algorithm), no year limits in range.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let _ = y;
    (m as u32, d as u32)
}

/// The texture a chest half is drawn with (vanilla `Sheets.chooseSprite`: `normal`/`trapped`/
/// `christmas` with `_left`/`_right` for the double halves, `ender` single).
pub fn chest_texture(material: ChestMaterial, half: ChestHalf) -> Identifier {
    let prefix = match material {
        ChestMaterial::Normal => "normal",
        ChestMaterial::Trapped => "trapped",
        ChestMaterial::Christmas => "christmas",
        ChestMaterial::Ender => return Identifier::minecraft("entity/chest/ender"),
    };
    let suffix = match half {
        ChestHalf::Single => "",
        ChestHalf::Left => "_left",
        ChestHalf::Right => "_right",
    };
    Identifier::minecraft(&format!("entity/chest/{prefix}{suffix}"))
}

/// The texture a shulker box is drawn with (vanilla `Sheets.getShulkerBoxSprite`:
/// `entity/shulker/shulker_<color>`, the undyed `entity/shulker/shulker` for `None`).
pub fn shulker_texture(color: Option<u8>) -> Identifier {
    const NAMES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    match color {
        None => Identifier::minecraft("entity/shulker/shulker"),
        Some(i) => Identifier::minecraft(&format!("entity/shulker/shulker_{}", NAMES[i as usize])),
    }
}

/// Vanilla `ChestModel.createSingleBodyLayer` and friends (26.3), one model per chest half.
/// Haste cube y is the vanilla `addBox` y negated (see the module doc); part pivots likewise.
fn chest_model(half: ChestHalf) -> CubeModel {
    // (bottom x, width) and the lock's x span per half: single 1..15, right 1..16 (east hidden),
    // left 0..15 (west hidden). Vanilla hides the touching face; drawing it is invisible (it
    // lies between the two halves) and saves a per-face flag in the shared `Cube` type.
    let (x, w) = match half {
        ChestHalf::Single => (1.0, 14.0),
        ChestHalf::Left => (0.0, 15.0),
        ChestHalf::Right => (1.0, 15.0),
    };
    let lock_x = match half {
        ChestHalf::Single => 7.0,
        ChestHalf::Left => 0.0,
        ChestHalf::Right => 15.0,
    };
    let lock_w = if half == ChestHalf::Single { 2.0 } else { 1.0 };
    let cube = |origin: Vec3, size: Vec3, uv: [u32; 2]| Cube {
        origin,
        size,
        inflate: 0.0,
        uv,
        mirror: false,
    };
    CubeModel {
        // The chest textures are all 64x64.
        texture: Identifier::minecraft("entity/chest/normal"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                // bottom: addBox(x, 0, 1, w, 10, 14), texOffs(0, 19)
                ModelPart {
                    name: "bottom".into(),
                    pivot: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![cube(
                        Vec3::new(x, -10.0, 1.0),
                        Vec3::new(w, 10.0, 14.0),
                        [0, 19],
                    )],
                    children: Vec::new(),
                },
                // lid: addBox(x, 0, 0, w, 5, 14) at PartPose.offset(0, 9, 1), texOffs(0, 0)
                ModelPart {
                    name: "lid".into(),
                    pivot: Vec3::new(0.0, -9.0, 1.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![cube(
                        Vec3::new(x, -5.0, 0.0),
                        Vec3::new(w, 5.0, 14.0),
                        [0, 0],
                    )],
                    children: Vec::new(),
                },
                // lock: addBox(lock_x, -2, 14, lock_w, 4, 1) at PartPose.offset(0, 9, 1), texOffs(0, 0)
                ModelPart {
                    name: "lock".into(),
                    pivot: Vec3::new(0.0, -9.0, 1.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![cube(
                        Vec3::new(lock_x, -2.0, 14.0),
                        Vec3::new(lock_w, 4.0, 1.0),
                        [0, 0],
                    )],
                    children: Vec::new(),
                },
            ],
        },
    }
}

/// The three chest halves (single, left, right), built once.
fn chest_models() -> &'static [CubeModel; 3] {
    static MODELS: OnceLock<[CubeModel; 3]> = OnceLock::new();
    MODELS.get_or_init(|| {
        [
            chest_model(ChestHalf::Single),
            chest_model(ChestHalf::Left),
            chest_model(ChestHalf::Right),
        ]
    })
}

/// Vanilla `ShulkerModel.createBoxLayer`: lid and base as separate parts at pivot (0, 24, 0),
/// entity convention (y down, ground at 24). The lid's rise is applied per instance (vanilla
/// `ShulkerBoxRenderer.ShulkerBoxModel.setupAnim` moves the pivot to `24 - progress * 8`).
fn shulker_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/shulker/shulker"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                // lid: addBox(-8, -16, -8, 16, 12, 16), texOffs(0, 0)
                ModelPart {
                    name: "lid".into(),
                    pivot: Vec3::new(0.0, 24.0, 0.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![Cube {
                        origin: Vec3::new(-8.0, -16.0, -8.0),
                        size: Vec3::new(16.0, 12.0, 16.0),
                        inflate: 0.0,
                        uv: [0, 0],
                        mirror: false,
                    }],
                    children: Vec::new(),
                },
                // base: addBox(-8, -8, -8, 16, 8, 16), texOffs(0, 28)
                ModelPart {
                    name: "base".into(),
                    pivot: Vec3::new(0.0, 24.0, 0.0),
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: vec![Cube {
                        origin: Vec3::new(-8.0, -8.0, -8.0),
                        size: Vec3::new(16.0, 8.0, 16.0),
                        inflate: 0.0,
                        uv: [0, 28],
                        mirror: false,
                    }],
                    children: Vec::new(),
                },
            ],
        },
    })
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

/// Vanilla `ChestRenderer.createModelTransformation`: rotate by `-toYRot(facing)` about the
/// block's centre column (0.5, 0, 0.5), with the chest model's y-up-to-entity-convention mirror
/// (see the module doc) folded in at the end.
fn chest_root(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
        * Mat4::from_rotation_y((-to_y_rot(facing)).to_radians())
        * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5))
        * Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0) / 16.0)
        * Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
}

/// Vanilla `Direction.getRotation` (the shulker transform's facing rotation): JOML's
/// `rotationXYZ(x, 0, z)` composites X * Z on column vectors.
fn direction_rotation(d: Direction) -> Mat4 {
    let x = |a: f32| Mat4::from_rotation_x(a.to_radians());
    let z = |a: f32| Mat4::from_rotation_z(a.to_radians());
    match d {
        Direction::Up => Mat4::IDENTITY,
        Direction::Down => x(180.0),
        Direction::North => x(90.0) * z(180.0),
        Direction::South => x(90.0),
        Direction::West => x(90.0) * z(90.0),
        Direction::East => x(90.0) * z(-90.0),
    }
}

/// Vanilla `ShulkerBoxRenderer.createModelTransform`:
/// `translation(0.5, 0.5, 0.5) * scale(0.9995) * direction.getRotation() * scale(1, -1, -1)
/// * translate(0, -1, 0)` on the model's block-unit vertices, here acting on model units
/// (the `/16` folded in before the translate, the flip after it).
fn shulker_root(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::splat(0.5))
        * Mat4::from_scale(Vec3::splat(0.9995))
        * direction_rotation(facing)
        * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0))
        * Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `ChestRenderer.submit`'s lid angle curve: `1 - (1 - open)^3`.
pub fn chest_open_curve(open: f32) -> f32 {
    1.0 - (1.0 - open).powi(3)
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

/// Vanilla `DyeColor.getTextColor` (the ARGB text colours), by dye id.
const DYE_TEXT_COLOR: [u32; 16] = [
    0xFFFF_FFFF,
    0xFFFF_681F,
    0xFFFF_00FF,
    0xFF9A_C0CD,
    0xFFFF_FF00,
    0xFFBF_FF00,
    0xFFFF_69B4,
    0xFF80_8080,
    0xFFD3_D3D3,
    0xFF00_FFFF,
    0xFFA0_20F0,
    0xFF00_00FF,
    0xFF8B_4513,
    0xFF00_FF00,
    0xFFFF_0000,
    0xFF00_0000,
];

/// The outline colour vanilla uses for black glowing text (`AbstractSignRenderer.BLACK_TEXT_OUTLINE_COLOR`).
const BLACK_TEXT_OUTLINE_COLOR: u32 = 0xFFF0_EBCC;

/// Vanilla `DyeColor.getTextureDiffuseColor` (the dye's map colour), by dye id.
pub const DYE_DIFFUSE_COLOR: [u32; 16] = [
    16383998, 16351261, 13061821, 3847130, 16701501, 8439583, 15961002, 4673362, 10329495, 1481884,
    8991416, 3949738, 8606770, 6192150, 11546150, 1908001,
];

/// Vanilla `AbstractSignRenderer.getDarkColor`: the colour non-glowing text is drawn with —
/// the dye colour scaled by 0.4 (per channel, clamped), except black glowing text which keeps
/// its special outline colour instead of scaling to near-black.
pub fn sign_dark_color(side: &SignSideText) -> u32 {
    let color = DYE_TEXT_COLOR[side.color as usize % 16];
    if side.color == 15 && side.glowing {
        BLACK_TEXT_OUTLINE_COLOR
    } else {
        let scale = |c: u32| ((c * 2) / 5).min(255);
        0xFF00_0000
            | scale((color >> 16) & 0xFF) << 16
            | scale((color >> 8) & 0xFF) << 8
            | scale(color & 0xFF)
    }
}

/// The colour glowing text is drawn with (the dye's own text colour), vanilla
/// `AbstractSignRenderer.submitSignText`.
pub fn sign_glow_color(side: &SignSideText) -> u32 {
    DYE_TEXT_COLOR[side.color as usize % 16]
}

/// Vanilla `StandingSignRenderer.textTransformation` / `HangingSignRenderer.textTransformation`:
/// the matrix a side's text is drawn through. `angle` is the sign's yaw in vanilla degrees
/// (standing: the `rotation` segment, wall: the wall's `facing.toYRot()`); `front` picks the
/// side (the back adds vanilla's extra 180-degree turn).
pub(crate) fn sign_text_transform(
    hanging: bool,
    angle: f32,
    front: bool,
    camera: glam::DVec3,
    pos: BlockPos,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let mut m = if hanging {
        // Hanging: (0.5, 0.9375, 0.5), rotate -angle, translate (0, -0.3125, 0).
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.9375, 0.5))
            * Mat4::from_rotation_y((-angle).to_radians())
            * Mat4::from_translation(Vec3::new(0.0, -0.3125, 0.0))
    } else {
        // Standing: (0.5, 0.5, 0.5), rotate -angle, wall signs drop to (0, -0.3125, -0.4375).
        Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.5, 0.5))
            * Mat4::from_rotation_y((-angle).to_radians())
    };
    if !front {
        m *= Mat4::from_rotation_y(180.0f32.to_radians());
    }
    let s = if hanging { 0.0140625 } else { 0.010416667 };
    let offset = if hanging {
        Vec3::new(0.0, -0.32, 0.073)
    } else {
        Vec3::new(0.0, 1.0 / 3.0, 0.046666667)
    };
    m * Mat4::from_translation(offset) * Mat4::from_scale(Vec3::new(s, -s, s))
}

/// Vanilla `SignBlockEntity.getTextLineHeight` (10) and `HangingSignBlockEntity` (9);
/// `getMaxTextLineWidth` is 90 standing, 60 hanging.
pub fn sign_line_metrics_pub(hanging: bool) -> (i32, i32) {
    sign_line_metrics(hanging)
}

pub(crate) fn sign_line_metrics(hanging: bool) -> (i32, i32) {
    if hanging {
        (9, 60)
    } else {
        (10, 90)
    }
}

/// Vanilla `BannerModel.createBodyLayer` + `BannerFlagModel.createFlagLayer` as one model per
/// attachment: standing has the pole (texOffs 44,0) and the bar at y -44; wall banners drop
/// the pole and hang the bar at y -20.5, z 9.5. The flag (texOffs 0,0) hangs from its pivot
/// (standing (0, -44, 0), wall (0, -20.5, 10.5)) and waves by `BannerFlagModel.setupAnim`.
fn banner_model(wall: bool) -> &'static CubeModel {
    static STANDING: OnceLock<CubeModel> = OnceLock::new();
    static WALL: OnceLock<CubeModel> = OnceLock::new();
    let model = if wall { &WALL } else { &STANDING };
    model.get_or_init(|| {
        let mut children = Vec::new();
        if !wall {
            children.push(ModelPart {
                name: "pole".into(),
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                // addBox(-1, -42, -1, 2, 42, 2), texOffs(44, 0)
                cubes: vec![Cube {
                    origin: Vec3::new(-1.0, -42.0, -1.0),
                    size: Vec3::new(2.0, 42.0, 2.0),
                    inflate: 0.0,
                    uv: [44, 0],
                    mirror: false,
                }],
                children: Vec::new(),
            });
        }
        let bar_y = if wall { -20.5 } else { -44.0 };
        let bar_z = if wall { 9.5 } else { -1.0 };
        children.push(ModelPart {
            name: "bar".into(),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            // addBox(-10, bar_y, bar_z, 20, 2, 2), texOffs(0, 42)
            cubes: vec![Cube {
                origin: Vec3::new(-10.0, bar_y, bar_z),
                size: Vec3::new(20.0, 2.0, 2.0),
                inflate: 0.0,
                uv: [0, 42],
                mirror: false,
            }],
            children: Vec::new(),
        });
        let flag_pivot = if wall {
            Vec3::new(0.0, -20.5, 10.5)
        } else {
            Vec3::new(0.0, -44.0, 0.0)
        };
        children.push(ModelPart {
            name: "flag".into(),
            pivot: flag_pivot,
            rotation: Vec3::ZERO,
            visible: true,
            // addBox(-10, 0, -2, 20, 40, 1), texOffs(0, 0)
            cubes: vec![Cube {
                origin: Vec3::new(-10.0, 0.0, -2.0),
                size: Vec3::new(20.0, 40.0, 1.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            children: Vec::new(),
        });
        CubeModel {
            texture: Identifier::minecraft("entity/banner/banner_base"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: Vec::new(),
                children,
            },
        }
    })
}

/// Vanilla `BannerRenderer.modelTransformation`: translate (0.5, 0, 0.5), rotate `-angle`
/// about y, scale (2/3, -2/3, -2/3) — the y/z flip maps the model's y-down entity convention
/// onto the world — and the model renderer's 1/16 from pixels to blocks. `angle` is the
/// `rotation` segment (standing) or the wall `facing.toYRot()`.
fn banner_root(pos: BlockPos, angle: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
        * Mat4::from_rotation_y((-angle).to_radians())
        * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `BannerFlagModel.setupAnim`: the flag swings by
/// `(-0.0125 + 0.01 * cos(2 * PI * phase)) * PI` radians about its pivot's x axis.
pub fn banner_flag_rotation(phase: f32) -> f32 {
    (-0.0125 + 0.01 * (std::f32::consts::TAU * phase).cos()) * std::f32::consts::PI
}

/// Vanilla `BannerRenderer.extractRenderState`'s phase:
/// `(floorMod(x*7 + y*9 + z*13 + gameTime, 100) + partial) / 100`.
pub fn banner_wave_phase(pos: BlockPos, game_time: i64, partial: f32) -> f32 {
    let hash = pos.x * 7 + pos.y * 9 + pos.z * 13;
    (hash.wrapping_add(game_time as i32).rem_euclid(100) as f32 + partial) / 100.0
}

/// Vanilla `BeaconRenderState.animationTime`: `floorMod(gameTime, 40) + partialTicks`.
pub fn beam_animation_time(game_time: i64, partial: f32) -> f32 {
    game_time.rem_euclid(40) as f32 + partial
}

/// Vanilla `BeaconRenderer.submit`'s per-section heights: every section draws from the beam
/// start to its own height, the last one to `MAX_RENDER_Y` (2048) — the beam runs to the build
/// limit whatever is above.
pub const BEAM_MAX_RENDER_Y: i32 = 2048;

/// The beam radius scale (vanilla `BeaconRenderer.extract`): 1 within 96 blocks (always 1
/// while scoping), else the horizontal distance / 96, so distant beams stay visible.
pub fn beam_radius_scale(distance: f32, scoping: bool) -> f32 {
    if scoping {
        1.0
    } else {
        (distance / 96.0).max(1.0)
    }
}

/// The texture V offset of a beam (vanilla `submitBeaconBeam`): the scroll by animation time,
/// `frac(scroll * 0.2 - floor(scroll * 0.1))` with `scroll = -animationTime` for an upward
/// beam (`height >= 0`).
pub fn beam_tex_v_offset(animation_time: f32, height: i32) -> f32 {
    let scroll = if height < 0 {
        animation_time
    } else {
        -animation_time
    };
    (scroll * 0.2 - (scroll * 0.1).floor()).fract()
}

/// One beam quad's two V texture coordinates (vanilla `submitBeaconBeam`): `vv2 = -1 + off`
/// at the start, and `vv1` at the end — `height * scale * (0.5 / radius)` for the inner beam,
/// `height * scale` for the glow.
pub fn beam_tex_v1(height: f32, scale: f32, radius: f32, glow: bool, vv2: f32) -> f32 {
    if glow {
        height * scale + vv2
    } else {
        height * scale * (0.5 / radius) + vv2
    }
}

/// The end portal's visible-face rule (vanilla `TheEndPortalBlockEntity.shouldRenderFace`):
/// only the horizontal faces.
pub fn end_portal_faces() -> u8 {
    (1 << Direction::Up as u8) | (1 << Direction::Down as u8)
}

/// The end gateway's visible-face rule (vanilla `TheEndGatewayBlockEntity.shouldRenderFace` ->
/// `Block.shouldRenderFace`): a face shows when the neighbour does not occlude it — an opaque
/// full cube hides it, unless it is the gateway block itself (portals draw against their own
/// type). `is_opaque` is the table's full-opaque check.
pub fn end_gateway_faces(
    get_state: impl Fn(BlockPos) -> BlockStateId,
    block_of: impl Fn(BlockStateId) -> haste_core::Identifier,
    is_opaque: impl Fn(BlockStateId) -> bool,
    state: BlockStateId,
    pos: BlockPos,
) -> u8 {
    let own = block_of(state);
    let mut faces = 0u8;
    for d in Direction::ALL {
        let neighbour = get_state(pos.offset(d));
        let same = block_of(neighbour) == own;
        if !same && is_opaque(neighbour) {
            continue;
        }
        faces |= 1 << d as u8;
    }
    faces
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
            let models = chest_models();
            let model = &models[match inst.half {
                ChestHalf::Single => 0,
                ChestHalf::Left => 1,
                ChestHalf::Right => 2,
            }];
            let open = chest_open_curve(inst.open.end.max(inst.open.start));
            // The lid and lock rotate by the open angle around their shared pivot (vanilla
            // `ChestModel.setupAnim`; the sign is the y mirror's, see the module doc).
            let mut pose = PoseOverrides::default();
            pose.rotations.push((
                "lid".into(),
                Vec3::new(open * std::f32::consts::FRAC_PI_2, 0.0, 0.0),
            ));
            pose.rotations.push((
                "lock".into(),
                Vec3::new(open * std::f32::consts::FRAC_PI_2, 0.0, 0.0),
            ));
            for part in &model.root.children {
                emit_part(
                    out,
                    model,
                    part,
                    &pose,
                    chest_root(inst.pos, inst.facing, camera),
                    rect,
                    light,
                );
            }
        }
        BlockEntityKind::Shulker(_color) => {
            let model = shulker_model();
            let progress = inst.open.end.max(inst.open.start);
            let mut pose = PoseOverrides::default();
            // Vanilla `ShulkerBoxModel.setupAnim`: the lid spins by 270 degrees * progress...
            pose.rotations.push((
                "lid".into(),
                Vec3::new(0.0, 270.0_f32.to_radians() * progress, 0.0),
            ));
            let root = shulker_root(inst.pos, inst.facing, camera);
            for part in &model.root.children {
                let parent = if part.name == "lid" {
                    // ...and rises by `progress * 0.5 * 16` model units (the pivot moves from
                    // 24 down to 16).
                    root * Mat4::from_translation(Vec3::new(0.0, -8.0 * progress, 0.0))
                } else {
                    root
                };
                emit_part(out, model, part, &pose, parent, rect, light);
            }
        }
        // Signs draw no cubes here: the board is the chunk's block model and the text goes
        // through the entity renderer's glyph pass (`emit_sign_text`), which needs the font.
        BlockEntityKind::Sign { .. } => {}
        // Banners likewise draw per-pattern layers with a tint, handled by
        // `EntityRenderer::emit_banner`.
        BlockEntityKind::Banner { .. } => {}
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
                    pose.rotations.push((
                        "jaw".into(),
                        Vec3::new(skull_dragon_jaw_rotation(anim), 0.0, 0.0),
                    ));
                }
                SkullKind::Piglin => {
                    let (left, right) = skull_piglin_ear_rotations(anim);
                    pose.rotations
                        .push(("left_ear".into(), Vec3::new(0.0, 0.0, left)));
                    pose.rotations
                        .push(("right_ear".into(), Vec3::new(0.0, 0.0, right)));
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

/// Poses one banner's layers, following vanilla `BannerRenderer.submitBanner`: the model
/// (pole, bar, flag) through `modelTransformation`, the flag waving by the phase, then one
/// pass per pattern layer (the base sprite untinted, the `base` pattern in the banner's
/// colour, then up to 16 patterns in their own), each through the same geometry with its
/// texture and diffuse tint. Draw order is back to front: bar and pole first, then the flag.
pub(crate) fn emit_banner(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    angle: f32,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    use crate::entity::emit_part_tinted;
    let wall = matches!(inst.kind, BlockEntityKind::Banner { wall: true });
    let model = banner_model(wall);
    let (sky, block) = inst.light;
    let light = (block, sky);
    let mut pose = PoseOverrides::default();
    pose.rotations.push((
        "flag".into(),
        Vec3::new(banner_flag_rotation(inst.banner_phase), 0.0, 0.0),
    ));
    let root = banner_root(inst.pos, angle, camera);
    let Some(data) = inst.banner.clone() else {
        return;
    };
    // (texture, tint as linear rgb) per layer, vanilla's submitBanner then submitPatterns.
    let mut layers: Vec<(Identifier, [f32; 3])> = Vec::new();
    let diffuse = |c: u32| {
        [
            ((c >> 16) & 0xFF) as f32 / 255.0,
            ((c >> 8) & 0xFF) as f32 / 255.0,
            (c & 0xFF) as f32 / 255.0,
        ]
    };
    // The base sprite (banner_base) is drawn white; the `base` pattern in the banner colour.
    layers.push((model.texture.clone(), [1.0; 3]));
    layers.push((
        Identifier::minecraft("entity/banner/base"),
        diffuse(DYE_DIFFUSE_COLOR[data.base as usize % 16]),
    ));
    for layer in data.layers.iter().take(16) {
        // The pattern's texture is `entity/banner/<asset id>`; the registry id's path is the
        // asset id for every vanilla pattern.
        let tex = Identifier::minecraft(&format!("entity/banner/{}", layer.pattern.path()));
        layers.push((tex, diffuse(DYE_DIFFUSE_COLOR[layer.color as usize % 16])));
    }
    for (texture, tint) in &layers {
        let rect = rect_of(texture);
        for part in &model.root.children {
            emit_part_tinted(out, model, part, &pose, root, rect, light, 0.0, *tint);
        }
    }
}

// -------------------------------------------------------------------------------------------
// Beacon beams and end gateways (vanilla `BeaconRenderer.submitBeaconBeam`,
// `TheEndGatewayRenderer`): textured quads into a beam-specific vertex stream. The beam ignores
// the lightmap (vanilla's `rendertype_beacon_beam` shader is texture * colour + fog), so these
// do not join the entity stream; the end portal's layered starfield is a pipeline of its own
// (`endportal.wgsl`).

/// One beam quad pushed as two triangles, in the beam's own (camera-relative) space: `m` poses
/// it, `rect` is the atlas region of `beacon_beam.png` / `end_gateway_beam.png`. The colour is
/// the section's ARGB with the alpha the pass wants (255 inner, 32 glow).
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_beam_quad(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    p: [Vec3; 4],
    uv: [[f32; 2]; 4],
    color: [f32; 4],
    rect: [u32; 4],
) {
    let p = p.map(|v| m.transform_point3(v));
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let r16 = rect.map(|v| v as u16);
    let order = [0, 1, 2, 0, 2, 3];
    for i in order {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            // The beam shader ignores the lightmap; the field carries the light for the format.
            light: [15, 15, 0, 0],
        });
    }
}

/// Vanilla `renderPart`: four quads around the beam axis (west-north, east-north, east-south,
/// west-south corners as `(x, z)` pairs), each from `beamStart` to `beamEnd`.
#[allow(clippy::too_many_arguments)]
fn emit_beam_part(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    color: [f32; 4],
    beam_start: f32,
    beam_end: f32,
    corners: [(f32, f32); 4],
    vv1: f32,
    vv2: f32,
    rect: [u32; 4],
) {
    let quads = [
        (corners[0], corners[1]),
        (corners[3], corners[2]),
        (corners[1], corners[2]),
        (corners[0], corners[3]),
    ];
    for &(a, b) in &quads {
        push_beam_quad(
            out,
            m,
            [
                Vec3::new(a.0, beam_end, a.1),
                Vec3::new(a.0, beam_start, a.1),
                Vec3::new(b.0, beam_start, b.1),
                Vec3::new(b.0, beam_end, b.1),
            ],
            [[1.0, vv1], [1.0, vv2], [0.0, vv2], [0.0, vv1]],
            color,
            rect,
        );
    }
}

/// Vanilla `BeaconRenderer.submitBeaconBeam` for one beam (also the gateway's, with its own
/// texture, radii and colour): the inner beam (rotated by `animationTime * 2.25 - 45`
/// degrees, opaque) and the glow (axis-aligned, alpha 32/255), both full-bright.
/// `beam_start`/`height` are in blocks; `scale` is 1 for beacons, the gateway's spawn scale.
/// The inner and glow quads go to separate streams (each is one draw with its own pipeline).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_beam(
    inner: &mut Vec<EntityVertex>,
    glow: &mut Vec<EntityVertex>,
    inst_pos: BlockPos,
    camera: glam::DVec3,
    texture: &Identifier,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
    scale: f32,
    animation_time: f32,
    beam_start: i32,
    height: i32,
    color: u32,
    radius_scale: f32,
    solid_radius: f32,
    glow_radius: f32,
) {
    let beam_end = beam_start + height;
    let tex_v_off = beam_tex_v_offset(animation_time, height);
    let root =
        Mat4::from_translation((inst_pos.as_dvec3() - camera).as_vec3() + Vec3::new(0.5, 0.0, 0.5));
    let rect = rect_of(texture);
    let argb = |c: u32, a: u32| {
        [
            ((c >> 16) & 0xFF) as f32 / 255.0,
            ((c >> 8) & 0xFF) as f32 / 255.0,
            (c & 0xFF) as f32 / 255.0,
            a as f32 / 255.0,
        ]
    };
    let solid = solid_radius * radius_scale;
    let glow_r = glow_radius * radius_scale;
    // The inner beam turns with the animation; vanilla's `rotateDegrees(YP, ...)`.
    let rot = root * Mat4::from_rotation_y((animation_time * 2.25 - 45.0).to_radians());
    let vv2 = -1.0 + tex_v_off;
    let vv1 = beam_tex_v1(height as f32, scale, solid, false, vv2);
    emit_beam_part(
        inner,
        rot,
        argb(color, 255),
        beam_start as f32,
        beam_end as f32,
        [(0.0, solid), (solid, 0.0), (0.0, -solid), (-solid, 0.0)],
        vv1,
        vv2,
        rect,
    );
    // The glow is axis-aligned around the same axis at the larger radius, translucent.
    let vv1 = beam_tex_v1(height as f32, scale, glow_r, true, vv2);
    emit_beam_part(
        glow,
        root,
        argb(color, 32),
        beam_start as f32,
        beam_end as f32,
        [
            (-glow_r, -glow_r),
            (glow_r, -glow_r),
            (glow_r, glow_r),
            (-glow_r, glow_r),
        ],
        vv1,
        vv2,
        rect,
    );
}

/// The beacon's beam: one `emit_beam` per section, each from the running start to the
/// section's height, the last to [`BEAM_MAX_RENDER_Y`] (vanilla `BeaconRenderer.submit`).
pub(crate) fn emit_beacon_beam(
    inner: &mut Vec<EntityVertex>,
    glow: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    radius_scale: f32,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    let Some(data) = inst.beam.as_deref() else {
        return;
    };
    let texture = Identifier::minecraft("entity/beacon/beacon_beam");
    let mut beam_start = 0i32;
    let last = data.sections.len().saturating_sub(1);
    for (i, section) in data.sections.iter().enumerate() {
        let height = if i == last {
            BEAM_MAX_RENDER_Y
        } else {
            section.height
        };
        emit_beam(
            inner,
            glow,
            inst.pos,
            camera,
            &texture,
            rect_of,
            1.0,
            inst.beam_time,
            beam_start,
            height,
            section.color,
            radius_scale,
            0.2,
            0.25,
        );
        beam_start += section.height;
    }
}

/// The end gateway's beam (vanilla `TheEndGatewayRenderer.submit`): the same beam shape at
/// radii 0.15/0.175, from `-height` to `+height`, scaled by the spawn/cooldown percent.
pub(crate) fn emit_gateway_beam(
    inner: &mut Vec<EntityVertex>,
    glow: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    let Some(beam) = inst.gateway_beam else {
        return;
    };
    if beam.height <= 0 {
        return;
    }
    let texture = Identifier::minecraft("entity/end_portal/end_gateway_beam");
    emit_beam(
        inner,
        glow,
        inst.pos,
        camera,
        &texture,
        rect_of,
        beam.scale,
        inst.beam_time,
        -beam.height,
        beam.height * 2,
        beam.color,
        1.0,
        0.15,
        0.175,
    );
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

/// Vanilla `BookModel.createBodyLayer` (64x32). `left_lid`, `right_lid`, `seam`, `left_pages`,
/// `right_pages` and the two `flip_page`s; the pose (`BookModel.setupAnim`) is per instance.
fn book_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/enchantment/enchanting_table_book"),
        tex_width: 64,
        tex_height: 32,
        root: ModelPart {
            name: "root".into(),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                part("left_lid", Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO)
                    .cube(
                        Vec3::new(-6.0, -5.0, -0.005),
                        Vec3::new(6.0, 10.0, 0.005),
                        0.0,
                        [0, 0],
                    )
                    .done(Vec::new()),
                part("right_lid", Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -5.0, -0.005),
                        Vec3::new(6.0, 10.0, 0.005),
                        0.0,
                        [16, 0],
                    )
                    .done(Vec::new()),
                part(
                    "seam",
                    Vec3::ZERO,
                    Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0),
                )
                .cube(
                    Vec3::new(-1.0, -5.0, 0.0),
                    Vec3::new(2.0, 10.0, 0.005),
                    0.0,
                    [12, 0],
                )
                .done(Vec::new()),
                part("left_pages", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, -0.99),
                        Vec3::new(5.0, 8.0, 1.0),
                        0.0,
                        [0, 10],
                    )
                    .done(Vec::new()),
                part("right_pages", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, -0.01),
                        Vec3::new(5.0, 8.0, 1.0),
                        0.0,
                        [12, 10],
                    )
                    .done(Vec::new()),
                part("flip_page1", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, 0.0),
                        Vec3::new(5.0, 8.0, 0.005),
                        0.0,
                        [24, 10],
                    )
                    .done(Vec::new()),
                part("flip_page2", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, 0.0),
                        Vec3::new(5.0, 8.0, 0.005),
                        0.0,
                        [24, 10],
                    )
                    .done(Vec::new()),
            ],
        },
    })
}

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

/// The texture a book is drawn with (vanilla `EnchantTableRenderer.BOOK_TEXTURE`, in the block
/// atlas under the `entity` prefix).
pub fn book_texture() -> Identifier {
    Identifier::minecraft("entity/enchantment/enchanting_table_book")
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

/// Vanilla `BookModel.State.forAnimation`'s openness: `(sin(progress * 0.02) * 0.1 + 1.25) *
/// openness` radians.
pub fn book_openness(progress: f32, openness: f32) -> f32 {
    (((progress * 0.02) as f64).sin() as f32 * 0.1 + 1.25) * openness
}

/// Vanilla `EnchantTableRenderer.submit`'s page flips: `frac(flip + offset) * 1.6 - 0.3`,
/// clamped to 0..1, at offsets 0.25 and 0.75.
pub fn enchant_page_flip(flip: f32, offset: f32) -> f32 {
    let f = flip + offset - (flip + offset).floor();
    (f * 1.6 - 0.3).clamp(0.0, 1.0)
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

/// Vanilla `LecternRenderer.submit`'s transform: (0.5, 1.0625, 0.5), `-yRot` about y, 67.5
/// degrees about z, then (0, -0.125, 0). `yRot` is the block's
/// `facing.getClockWise().toYRot()`.
fn lectern_root(pos: BlockPos, facing: Direction, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 1.0625, 0.5))
        * Mat4::from_rotation_y((-to_y_rot(clockwise(facing))).to_radians())
        * Mat4::from_rotation_z(67.5_f32.to_radians())
        * Mat4::from_translation(Vec3::new(0.0, -0.125, 0.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `EnchantTableRenderer.submit`'s transform: (0.5, 0.75, 0.5), the bob
/// `0.1 + sin(time * 0.1) * 0.01`, `-rot` about y and 80 degrees about z. `rot` is the
/// interpolated `rot` (radians).
fn enchant_root(pos: BlockPos, time: f32, rot: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.75, 0.5))
        * Mat4::from_translation(Vec3::new(
            0.0,
            0.1 + ((time * 0.1) as f64).sin() as f32 * 0.01,
            0.0,
        ))
        * Mat4::from_rotation_y(-rot)
        * Mat4::from_rotation_z(80.0_f32.to_radians())
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
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

/// The book's per-instance pose, vanilla `BookModel.setupAnim`: the lids and pages turn by the
/// openness about y (the left lid from `PI`, the right lid from 0), the two flip pages by
/// `openness - openness * 2 * flip`, and the pages slide by `sin(openness)` along x.
fn book_pose(openness: f32, flip1: f32, flip2: f32) -> (ObjectPose, Vec3) {
    let pose = pose_of(&[
        (
            "left_lid",
            Vec3::new(0.0, std::f32::consts::PI + openness, 0.0),
        ),
        ("right_lid", Vec3::new(0.0, -openness, 0.0)),
        ("left_pages", Vec3::new(0.0, openness, 0.0)),
        ("right_pages", Vec3::new(0.0, -openness, 0.0)),
        (
            "flip_page1",
            Vec3::new(0.0, openness - openness * 2.0 * flip1, 0.0),
        ),
        (
            "flip_page2",
            Vec3::new(0.0, openness - openness * 2.0 * flip2, 0.0),
        ),
    ]);
    let slide = (openness as f64).sin() as f32;
    (pose, Vec3::new(slide, 0.0, 0.0))
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
        BlockEntityKind::Lectern => {
            // Vanilla `LecternRenderer`'s fixed `BOOK_STATE`: `forAnimation(0, 0.1, 0.9, 1.2)`.
            let openness = book_openness(0.0, 1.2);
            let (pose, offset) = book_pose(openness, 0.1, 0.9);
            let model = book_model();
            emit_object(
                out,
                model,
                offset,
                &pose,
                lectern_root(inst.pos, inst.facing, camera),
                light,
                &mut |_| rect,
            );
        }
        BlockEntityKind::EnchantTable => {
            // Vanilla `EnchantTableRenderer.submit`'s book state from the block entity's tick.
            let (time, flip, open, rot) = match inst.anim {
                Some(BlockEntityAnim::EnchantTable {
                    time,
                    flip,
                    open,
                    rot,
                    ..
                }) => (time, flip.end, open.end, rot.end),
                _ => (0.0, 0.0, 0.0, 0.0),
            };
            let ff1 = enchant_page_flip(flip, 0.25);
            let ff2 = enchant_page_flip(flip, 0.75);
            let openness = book_openness(time, open);
            let (pose, offset) = book_pose(openness, ff1, ff2);
            let model = book_model();
            emit_object(
                out,
                model,
                offset,
                &pose,
                enchant_root(inst.pos, time, rot, camera),
                light,
                &mut |_| rect,
            );
        }
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
