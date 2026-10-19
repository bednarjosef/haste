//! Minecarts: the model (`MinecartModel.createBodyLayer`), the render state
//! `AbstractMinecartRenderer.extractRenderState` builds (`MinecartRenderState`), and the
//! old-behavior rail snapping (`OldMinecartBehavior.getPos`/`getPosOffs`) that poses the cart
//! along its track. The new-style track movement packet (`NewMinecartBehavior`) is a separate
//! task; only the old path is ported here, which is what a vanilla server runs.
//!
//! The synched data this reads (index = `Entity`'s eight slots 0-7, `VehicleEntity`'s
//! hurt/hurtDir/damage 8-10, `AbstractMinecart`'s custom display block 11 and display offset 12,
//! `MinecartFurnace`'s fuel 13) is interpreted in [`crate::metadata::Metadata`]'s minecart
//! fields; the TNT cart's fuse is primed by entity event 70 and counts down client-side
//! (`MinecartTNT.tick`).
use std::sync::{Arc, Mutex, OnceLock};

use ahash::AHashMap;
use haste_core::entity::MetadataValue;
use haste_core::math::Vec3;
use haste_core::model::{BlockDisplay, Cube, CubeModel, MinecartDecoration, ModelPart};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, DVec3, Identifier, RegistryIndex};
use haste_data::BlockStates;

use crate::metadata::Metadata;

/// `AbstractMinecart.DATA_ID_DISPLAY_OFFSET` (metadata index 12).
pub const DISPLAY_OFFSET_INDEX: u8 = 12;
/// The TNT cart's fuse starts at 80 ticks (`MinecartTNT.primeFuse`, entity event 70).
pub const FUSE_TICKS: i32 = 80;

/// Which minecart an entity is: the type names vanilla registers (`Minecart`,
/// `MinecartChest`, ...), each with the display block its renderer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Plain,
    Chest,
    Furnace,
    Hopper,
    Tnt,
    Spawner,
    CommandBlock,
}

/// The variant of an entity type name, `None` for everything that is not a minecart.
pub fn variant_for_name(name: &str) -> Option<Variant> {
    match name {
        "minecraft:minecart" => Some(Variant::Plain),
        "minecraft:chest_minecart" => Some(Variant::Chest),
        "minecraft:furnace_minecart" => Some(Variant::Furnace),
        "minecraft:hopper_minecart" => Some(Variant::Hopper),
        "minecraft:tnt_minecart" => Some(Variant::Tnt),
        "minecraft:spawner_minecart" => Some(Variant::Spawner),
        "minecraft:command_block_minecart" => Some(Variant::CommandBlock),
        _ => None,
    }
}

/// The variant of a tracked entity's type.
pub fn variant_for_kind(kind: RegistryIndex) -> Option<Variant> {
    crate::registry::entity_type_name(kind).and_then(variant_for_name)
}

/// `MinecartModel.createBodyLayer` (26.3): the cart's five boxes over the one 64x32 texture,
/// in the entity model convention (y down, pivots and box coords vanilla's `PartPose`/
/// `addBox` values, rotations in radians). All seven minecart layers bake this same layer.
pub fn model() -> Arc<CubeModel> {
    static MODEL: OnceLock<Arc<CubeModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            fn part(
                name: &str,
                pivot: [f32; 3],
                rotation: [f32; 3],
                uv: [u32; 2],
                origin: [f32; 3],
                size: [f32; 3],
            ) -> ModelPart {
                ModelPart {
                    name: name.to_owned(),
                    id: haste_core::parts::intern(name),
                    pivot: pivot.into(),
                    rotation: rotation.into(),
                    visible: true,
                    cubes: vec![Cube {
                        origin: origin.into(),
                        size: size.into(),
                        inflate: 0.0,
                        uv,
                        mirror: false,
                    }],
                    children: Vec::new(),
                }
            }
            // front, back, left and right share one box shape (vanilla reuses the same
            // `CubeListBuilder`); only the bottom's differs.
            let side = |name: &str, pivot: [f32; 3], y_rot: f32| {
                part(
                    name,
                    pivot,
                    [0.0, y_rot, 0.0],
                    [0, 0],
                    [-8.0, -9.0, -1.0],
                    [16.0, 8.0, 2.0],
                )
            };
            Arc::new(CubeModel {
                texture: Identifier::minecraft("entity/minecart/minecart"),
                tex_width: 64,
                tex_height: 32,
                root: ModelPart {
                    name: "root".to_owned(),
                    id: haste_core::parts::ROOT,
                    pivot: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    visible: true,
                    cubes: Vec::new(),
                    children: vec![
                        part(
                            "bottom",
                            [0.0, 4.0, 0.0],
                            [std::f32::consts::FRAC_PI_2, 0.0, 0.0],
                            [0, 10],
                            [-10.0, -8.0, -1.0],
                            [20.0, 16.0, 2.0],
                        ),
                        side("front", [-9.0, 4.0, 0.0], 3.0 * std::f32::consts::FRAC_PI_2),
                        side("back", [9.0, 4.0, 0.0], std::f32::consts::FRAC_PI_2),
                        side("left", [0.0, 4.0, -7.0], std::f32::consts::PI),
                        side("right", [0.0, 4.0, 7.0], 0.0),
                    ],
                },
            })
        })
        .clone()
}

/// `AbstractMinecartRenderer.submit`'s per-entity jitter seed, from the entity id (vanilla
/// `entity.getId() * 493286711L`, squared and mixed with `* 4392167121L + * 98761L`; Java
/// long overflow wraps).
pub fn offset_seed(entity_id: i32) -> i64 {
    let seed = (entity_id as i64).wrapping_mul(493286711);
    seed.wrapping_mul(seed).wrapping_mul(4392167121) + seed.wrapping_mul(98761)
}

/// The render state `AbstractMinecartRenderer.extractRenderState` collects (the parts that
/// come from the synched data; the rail positions are filled per frame by [`snap_rails`]).
/// `partial` is the frame's partial tick.
pub fn decoration(m: &Metadata, variant: Variant, partial: f32) -> MinecartDecoration {
    // `getHurtTime() - partialTicks`; `getDamage()` floors at 0 like the renderer's
    // `Math.max(... - partialTicks, 0.0F)`. Vanilla's default hurt direction is 1
    // (`VehicleEntity.defineSynchedData`); an all-zero slot (a cart that was never hurt)
    // reads as that.
    let hurt_dir = if m.minecart_hurt_dir == 0 {
        1.0
    } else {
        m.minecart_hurt_dir as f32
    };
    // `TntMinecartRenderer.extractRenderState`: fuse - partial + 1 while primed, else -1.
    let fuse = match m.minecart_fuse {
        Some(f) if f > -1 => f as f32 - partial + 1.0,
        _ => -1.0,
    };
    MinecartDecoration {
        display: display(m, variant),
        display_offset: match m.get(DISPLAY_OFFSET_INDEX).map(|e| &e.value) {
            Some(MetadataValue::VarInt(v)) => *v,
            _ => default_display_offset(variant),
        },
        hurt_time: m.minecart_hurt_time as f32 - partial,
        hurt_dir,
        damage_time: (m.minecart_damage - partial).max(0.0),
        fuse,
        rail: None,
    }
}

/// `AbstractMinecart.getDisplayOffset`'s default per type: 8 for a chest cart, 1 for a hopper
/// cart, 6 for everything else.
fn default_display_offset(variant: Variant) -> i32 {
    match variant {
        Variant::Chest => 8,
        Variant::Hopper => 1,
        _ => 6,
    }
}

/// `AbstractMinecart.getDisplayBlockState`: the custom display block when the synched data set
/// one, else the type's default (`getDefaultDisplayBlockState`). `None` means air: the cart
/// draws no cargo.
pub fn display(m: &Metadata, variant: Variant) -> Option<Arc<BlockDisplay>> {
    let states = states()?;
    let state = match m.minecart_custom_display {
        Some(s) => s,
        None => default_display_state(variant, m.minecart_fuel)?,
    };
    block_display(states, state)
}

/// The default display state of a variant (`getDefaultDisplayBlockState` of each vanilla
/// minecart class): a chest facing north, a furnace facing north and lit with the fuel flag,
/// a hopper, TNT, a spawner or a command block; nothing in a plain cart.
fn default_display_state(variant: Variant, fuel: bool) -> Option<BlockStateId> {
    states().and_then(|s| default_display_state_in(s, variant, fuel))
}

/// [`default_display_state`] against a given table (tests pass a small one).
pub(crate) fn default_display_state_in(
    states: &BlockStates,
    variant: Variant,
    fuel: bool,
) -> Option<BlockStateId> {
    let state = |block: &str| states.default_state(&Identifier::minecraft(block));
    match variant {
        Variant::Plain => None,
        Variant::Chest => Some(
            state("chest")
                .and_then(|s| states.with_property(s, "facing", "north"))
                .or_else(|| state("chest"))?,
        ),
        Variant::Furnace => Some(
            state("furnace")
                .and_then(|s| states.with_property(s, "facing", "north"))
                .and_then(|s| states.with_property(s, "lit", if fuel { "true" } else { "false" }))
                .or_else(|| state("furnace"))?,
        ),
        Variant::Hopper => state("hopper"),
        Variant::Tnt => state("tnt"),
        Variant::Spawner => state("spawner"),
        Variant::CommandBlock => state("command_block"),
    }
}

/// The block-state table, loaded once from the data-generator report (`blocks.json`, the same
/// file the client loads at startup; the entity store does not hold it yet). A missing report
/// logs once and means minecarts draw without cargo until one appears and the process restarts.
fn states() -> Option<&'static BlockStates> {
    static STATES: OnceLock<Option<BlockStates>> = OnceLock::new();
    STATES
        .get_or_init(|| {
            let path = haste_core::paths::datagen_reports().join("blocks.json");
            match std::fs::read_to_string(&path) {
                Ok(json) => match BlockStates::parse(&json, None) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        tracing::warn!("minecart display blocks: bad {}: {e:#}", path.display());
                        None
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        "minecart display blocks: {} unreadable ({e}) — run `haste setup`",
                        path.display()
                    );
                    None
                }
            }
        })
        .as_ref()
}

/// A state's block and properties, cached per state id: only when the synched data changes,
/// never per frame. Air resolves to `None`.
fn block_display(states: &BlockStates, state: BlockStateId) -> Option<Arc<BlockDisplay>> {
    if state == BlockStateId::AIR {
        return None;
    }
    static CACHE: OnceLock<Mutex<AHashMap<u32, Option<Arc<BlockDisplay>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(AHashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(&state.0) {
        return hit.clone();
    }
    let entry = Arc::new(BlockDisplay {
        state,
        block: states.block_of(state).clone(),
        props: states.properties(state),
    });
    cache.lock().unwrap().insert(state.0, Some(entry.clone()));
    Some(entry)
}

/// `AbstractMinecart.tick`'s client-side countdowns: hurt time and damage tick down by one,
/// the TNT cart's fuse by one while positive.
pub(crate) fn tick(m: &mut Metadata) {
    if m.minecart_hurt_time > 0 {
        m.minecart_hurt_time -= 1;
    }
    if m.minecart_damage > 0.0 {
        m.minecart_damage -= 1.0;
    }
    if let Some(fuse) = m.minecart_fuse {
        if fuse > 0 {
            m.minecart_fuse = Some(fuse - 1);
        }
    }
}

/// `MinecartTNT.primeFuse` (entity event 70): the fuse starts at 80 ticks.
pub fn prime(m: &mut Metadata) {
    m.minecart_fuse = Some(FUSE_TICKS);
}

// -------------------------------------------------------------------------------------------
// The old-behavior rail math (`OldMinecartBehavior.getPos`/`getPosOffs`): the cart is posed on
// the rail segment it stands on, at the height of the track's line, facing along it.

/// A rail's shape (`RailShape`): the ten states a `shape` property can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    NorthSouth,
    EastWest,
    AscendingEast,
    AscendingWest,
    AscendingNorth,
    AscendingSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}

impl Shape {
    fn parse(v: &str) -> Option<Shape> {
        Some(match v {
            "north_south" => Shape::NorthSouth,
            "east_west" => Shape::EastWest,
            "ascending_east" => Shape::AscendingEast,
            "ascending_west" => Shape::AscendingWest,
            "ascending_north" => Shape::AscendingNorth,
            "ascending_south" => Shape::AscendingSouth,
            "south_east" => Shape::SouthEast,
            "south_west" => Shape::SouthWest,
            "north_west" => Shape::NorthWest,
            "north_east" => Shape::NorthEast,
            _ => return None,
        })
    }

    /// `RailShape.isSlope`.
    fn is_slope(self) -> bool {
        matches!(
            self,
            Shape::AscendingEast
                | Shape::AscendingWest
                | Shape::AscendingNorth
                | Shape::AscendingSouth
        )
    }

    /// `AbstractMinecart.exits`: the two blocks the shape's track runs to, ascending exits
    /// carrying the y step (`Direction.below()`).
    fn exits(self) -> ((i32, i32, i32), (i32, i32, i32)) {
        let west = (-1, 0, 0);
        let east = (1, 0, 0);
        let north = (0, 0, -1);
        let south = (0, 0, 1);
        match self {
            Shape::NorthSouth => (north, south),
            Shape::EastWest => (west, east),
            Shape::AscendingEast => ((-1, -1, 0), east),
            Shape::AscendingWest => (west, (1, -1, 0)),
            Shape::AscendingNorth => (north, (0, -1, 1)),
            Shape::AscendingSouth => ((0, -1, -1), south),
            Shape::SouthEast => (south, east),
            Shape::SouthWest => (south, west),
            Shape::NorthWest => (north, west),
            Shape::NorthEast => (north, east),
        }
    }
}

/// The shape of a rail block state, read from its `shape` property (the property every
/// `BaseRailBlock` — vanilla's four rails and a mod's — carries). `None` when the state is not
/// a rail we know.
fn rail_shape(blocks: &BlockStates, state: BlockStateId) -> Option<Shape> {
    Shape::parse(blocks.property_value(state, "shape")?)
}

/// `OldMinecartBehavior.getPos`: the position snapped onto the rail's line — the endpoints of
/// the track (`AbstractMinecart.exits`) at rail height plus 1/16, the cart projected onto the
/// segment.
fn rail_pos(view: &dyn BlockView, blocks: &BlockStates, x: f64, y: f64, z: f64) -> Option<DVec3> {
    let (xt, yt, zt) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let yt = if rail_shape(blocks, view.block_state(BlockPos::new(xt, yt - 1, zt))).is_some() {
        yt - 1
    } else {
        yt
    };
    let state = view.block_state(BlockPos::new(xt, yt, zt));
    let shape = rail_shape(blocks, state)?;
    let (exit0, exit1) = shape.exits();
    // The two track endpoints, at 1/16 above the rail block's floor (plus half the y step an
    // ascending exit carries).
    let (x0, y0, z0) = (
        xt as f64 + 0.5 + exit0.0 as f64 * 0.5,
        yt as f64 + 0.0625 + exit0.1 as f64 * 0.5,
        zt as f64 + 0.5 + exit0.2 as f64 * 0.5,
    );
    let (x1, y1, z1) = (
        xt as f64 + 0.5 + exit1.0 as f64 * 0.5,
        yt as f64 + 0.0625 + exit1.1 as f64 * 0.5,
        zt as f64 + 0.5 + exit1.2 as f64 * 0.5,
    );
    let (xd, yd, zd) = (x1 - x0, (y1 - y0) * 2.0, z1 - z0);
    // The cart's progress along the segment: its position projected onto the track's axis.
    let progress = if xd == 0.0 {
        z - zt as f64
    } else if zd == 0.0 {
        x - xt as f64
    } else {
        ((x - x0) * xd + (z - z0) * zd) * 2.0
    };
    let mut y = y0 + yd * progress;
    if yd < 0.0 {
        y += 1.0;
    } else if yd > 0.0 {
        y += 0.5;
    }
    Some(DVec3::new(x0 + xd * progress, y, z0 + zd * progress))
}

/// `OldMinecartBehavior.getPosOffs`: the position `offs` blocks along the track from the cart
/// (`getPos` of the moved point, so the same rail segment geometry applies).
#[allow(clippy::too_many_arguments)]
fn rail_pos_offs(
    view: &dyn BlockView,
    blocks: &BlockStates,
    x: f64,
    y: f64,
    z: f64,
    offs: f64,
) -> Option<DVec3> {
    let (xt, yt, zt) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let yt = if rail_shape(blocks, view.block_state(BlockPos::new(xt, yt - 1, zt))).is_some() {
        yt - 1
    } else {
        yt
    };
    let state = view.block_state(BlockPos::new(xt, yt, zt));
    let shape = rail_shape(blocks, state)?;
    let mut y = yt as f64;
    if shape.is_slope() {
        y += 1.0;
    }
    let (exit0, exit1) = shape.exits();
    let mut xd = (exit1.0 - exit0.0) as f64;
    let mut zd = (exit1.2 - exit0.2) as f64;
    let dd = (xd * xd + zd * zd).sqrt();
    xd /= dd;
    zd /= dd;
    let (mut x, mut z) = (x, z);
    x += xd * offs;
    z += zd * offs;
    if exit0.1 != 0 && x.floor() as i32 - xt == exit0.0 && z.floor() as i32 - zt == exit0.2 {
        y += exit0.1 as f64;
    } else if exit1.1 != 0 && x.floor() as i32 - xt == exit1.0 && z.floor() as i32 - zt == exit1.2 {
        y += exit1.1 as f64;
    }
    rail_pos(view, blocks, x, y, z)
}

/// The cart's rail positions for one frame (`oldExtractState`): the snapped position and the
/// points 0.3 blocks front and back the renderer derives the cart's rotation from, `None` off
/// the rails.
pub fn rail_positions(
    view: &dyn BlockView,
    blocks: &BlockStates,
    pos: DVec3,
) -> Option<(DVec3, DVec3, DVec3)> {
    let on_rail = rail_pos(view, blocks, pos.x, pos.y, pos.z)?;
    let front = rail_pos_offs(view, blocks, pos.x, pos.y, pos.z, 0.3).unwrap_or(on_rail);
    let back = rail_pos_offs(view, blocks, pos.x, pos.y, pos.z, -0.3).unwrap_or(on_rail);
    Some((on_rail, front, back))
}

/// Fills each minecart instance's rail positions from the world (`oldExtractState`'s
/// `behavior.getPos` calls, which read the level): the render thread's per-frame step where
/// the world is available. Four block lookups per cart, no allocation.
pub fn snap_rails(
    instances: &mut [haste_core::model::EntityInstance],
    view: &dyn BlockView,
    blocks: &BlockStates,
) {
    for e in instances {
        if let Some(haste_core::model::Decoration::Minecart(cart)) = &mut e.decoration {
            cart.rail = rail_positions(view, blocks, e.pos);
        }
    }
}
