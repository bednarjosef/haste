//! Placement-state tests (each covered vanilla class's `getStateForPlacement`, chosen
//! from the `block_classes.json` dump): plain blocks place their default state, logs
//! take the clicked face's axis, slabs go bottom/top by the click and double into a
//! same-kind slab, stairs take the player's facing and the click's half, and the
//! `waterlogged` property follows a water source at the position.
//!
//! The fixture is a small `GameData` (blocks with properties, per-state shapes with the
//! water fluid and the slab half-heights) written to a temp dir, like `tests::data`.
//! The class lookup needs the real `block_classes.json` from the cache's oracle dir:
//! without it nothing is predicted and these tests say so and return.

use std::sync::OnceLock;

use glam::DVec3;
use haste_core::entity::ItemStackData;
use haste_core::game::GameMode;
use haste_core::{Aabb, BlockPos, BlockStateId, Direction, EntityId, RegistryIndex};
use haste_physics::mining::BlockTags;
use haste_physics::raycast::BlockHit;

use super::{item, Rig};
use crate::player::interact::InteractEnv;

const STONE: BlockStateId = BlockStateId(1);
const WATER: BlockStateId = BlockStateId(3);
/// The item registry indices (the fixture registries list the blocks as items too).
const DIRT_ITEM: u32 = 2;
const OAK_LOG_ITEM: u32 = 4;
const OAK_SLAB_ITEM: u32 = 5;
const OAK_STAIRS_ITEM: u32 = 6;
const OAK_DOOR_ITEM: u32 = 7;

fn fixture() -> &'static haste_data::GameData {
    static D: OnceLock<haste_data::GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("haste-place-test-{}", std::process::id()));
        let reports = dir.join("reports");
        std::fs::create_dir_all(reports.join("minecraft").join("components").join("item"))
            .unwrap();

        // The states the property math needs, dense ids in mixed-radix order over the
        // name-sorted properties (see haste-data's blocks tests): id = first + digits.
        let stair_states: Vec<String> =
            ["north", "south", "west", "east"]
                .iter()
                .flat_map(|f| {
                    ["bottom", "top"].iter().flat_map(move |h| {
                        ["straight", "inner_left", "inner_right", "outer_left", "outer_right"]
                            .iter()
                            .flat_map(move |s| {
                                ["true", "false"].iter().map(move |w| {
                                    format!(r#"{{"facing": "{f}", "half": "{h}", "shape": "{s}", "waterlogged": "{w}"}}"#)
                                })
                            })
                    })
                })
                .enumerate()
                .map(|(i, p)| {
                    // The default: facing north, bottom, straight, dry (offset 1).
                    let default = if i == 1 { ", \"default\": true" } else { "" };
                    format!(r#"{{"id": {}, "properties": {p}{default}}}"#, 13 + i)
                })
                .collect();
        let blocks = format!(
            r#"{{
              "minecraft:air": {{"states": [{{"id": 0, "default": true}}]}},
              "minecraft:stone": {{"states": [{{"id": 1, "default": true}}]}},
              "minecraft:dirt": {{"states": [{{"id": 2, "default": true}}]}},
              "minecraft:water": {{"states": [{{"id": 3, "default": true}}]}},
              "minecraft:oak_log": {{
                "properties": {{"axis": ["x", "y", "z"]}},
                "states": [
                  {{"id": 4, "properties": {{"axis": "x"}}}},
                  {{"id": 5, "default": true, "properties": {{"axis": "y"}}}},
                  {{"id": 6, "properties": {{"axis": "z"}}}}
                ]}},
              "minecraft:oak_slab": {{
                "properties": {{"type": ["bottom", "top", "double"], "waterlogged": ["true", "false"]}},
                "states": [
                  {{"id": 7, "properties": {{"type": "bottom", "waterlogged": "true"}}}},
                  {{"id": 8, "default": true, "properties": {{"type": "bottom", "waterlogged": "false"}}}},
                  {{"id": 9, "properties": {{"type": "top", "waterlogged": "true"}}}},
                  {{"id": 10, "properties": {{"type": "top", "waterlogged": "false"}}}},
                  {{"id": 11, "properties": {{"type": "double", "waterlogged": "true"}}}},
                  {{"id": 12, "properties": {{"type": "double", "waterlogged": "false"}}}}
                ]}},
              "minecraft:oak_stairs": {{
                "properties": {{
                  "facing": ["north", "south", "west", "east"],
                  "half": ["bottom", "top"],
                  "shape": ["straight", "inner_left", "inner_right", "outer_left", "outer_right"],
                  "waterlogged": ["true", "false"]
                }},
                "states": [{}]
              }},
              "minecraft:oak_door": {{"states": [{{"id": 93, "default": true}}]}}
            }}"#,
            stair_states.join(",")
        );
        std::fs::write(reports.join("blocks.json"), blocks).unwrap();

        let names = [
            "air",
            "stone",
            "dirt",
            "water",
            "oak_log",
            "oak_slab",
            "oak_stairs",
            "oak_door",
        ];
        let entries = |ns: &[&str]| {
            ns.iter()
                .enumerate()
                .map(|(i, n)| format!(r#""minecraft:{n}": {{"protocol_id": {i}}}"#))
                .collect::<Vec<_>>()
                .join(",")
        };
        std::fs::write(
            reports.join("registries.json"),
            format!(
                r#"{{"minecraft:block": {{"entries": {{{}}}}},
                    "minecraft:item": {{"entries": {{{}}}}},
                    "minecraft:mob_effect": {{"entries": {{}}}},
                    "minecraft:data_component_type": {{"entries": {{}}}},
                    "minecraft:enchantment": {{"entries": {{}}}},
                    "minecraft:entity_type": {{"entries": {{{}}}}},
                    "minecraft:sound_event": {{"entries": {{}}}}}}"#,
                entries(&names),
                entries(&names),
                entries(&["player"]),
            ),
        )
        .unwrap();

        // Full cubes except air and water (the water row carries the fluid, a full
        // source — the `waterlogged` rule's input) and the slabs' real half heights,
        // which the obstruction check reads off the placement state.
        let full = "[[0.0,0.0,0.0,1.0,1.0,1.0]]";
        let bottom = "[[0.0,0.0,0.0,1.0,0.5,1.0]]";
        let top = "[[0.0,0.5,0.0,1.0,1.0,1.0]]";
        let mut rows = Vec::new();
        for id in 0..=93u32 {
            let row = match id {
                0 => r#"{"id": 0, "collision": [], "outline": [], "light": 0, "air": true}"#.into(),
                3 => r#"{"id": 3, "collision": [], "outline": [], "light": 0, "fluid": "minecraft:water"}"#.into(),
                7 | 8 => format!(r#"{{"id": {id}, "collision": {bottom}, "outline": {bottom}, "light": 0}}"#),
                9 | 10 => format!(r#"{{"id": {id}, "collision": {top}, "outline": {top}, "light": 0}}"#),
                _ => format!(r#"{{"id": {id}, "collision": {full}, "outline": {full}, "light": 0}}"#),
            };
            rows.push(row);
        }
        std::fs::write(
            dir.join("block_shapes.json"),
            format!(r#"{{"version": 1, "states": [{}]}}"#, rows.join(",")),
        )
        .unwrap();
        haste_data::GameData::load_from(&reports, Some(&dir.join("block_shapes.json"))).unwrap()
    })
}

/// Whether the class dump resolved (the cache's oracle dir): the covered-class tests
/// have nothing to predict without it.
fn dump_present() -> bool {
    crate::player::interact::block_classes::placement_class("minecraft:oak_log").is_some()
}

/// A rig over the fixture: stone floor, the `minecraft:replaceable` tag synced with
/// water in it, `oak_*` items in hand.
fn rig(held: u32) -> Rig {
    let mut r = Rig::new(GameMode::Survival);
    // The fixture's own data and tags (the shared ones know none of these blocks).
    r.p.int.set_block_tags(BlockTags::new([(
        haste_core::Identifier::minecraft("replaceable"),
        vec![3],
    )]));
    r.p.int.hotbar[0] = item(held);
    r
}

fn env(r: &Rig) -> InteractEnv<'_> {
    InteractEnv {
        data: fixture(),
        entities: &r.entities,
        entity_types: &r.types,
        border: None,
        warm_blocks: None,
        ui: None,
    }
}

/// `placement` with the fixture's environment.
fn placement_of(
    r: &Rig,
    h: BlockHit,
    held: u32,
    yaw: f32,
) -> Option<(BlockPos, Option<BlockStateId>)> {
    let stack = ItemStackData {
        count: 1,
        item: Some(RegistryIndex(held)),
        components_raw: Default::default(),
    };
    r.p.placement(h, &stack, &r.w, &env(r), yaw)
        .map(|p| (p.pos, p.state))
}

/// A hit on the top face of the block at `pos`, clicked at height `click_y` within it
/// (0..=1).
fn hit_on_top(pos: BlockPos, click_y: f64) -> BlockHit {
    BlockHit {
        pos,
        face: Direction::Up,
        location: DVec3::new(
            pos.x as f64 + 0.5,
            pos.y as f64 + click_y,
            pos.z as f64 + 0.5,
        ),
        distance: 1.0,
        inside: false,
    }
}

/// A hit on the north side face of the block at `pos`, clicked at height `click_y`.
fn hit_on_side(pos: BlockPos, click_y: f64) -> BlockHit {
    BlockHit {
        pos,
        face: Direction::North,
        location: DVec3::new(
            pos.x as f64 + 0.5,
            pos.y as f64 + click_y,
            pos.z as f64 + 0.999,
        ),
        distance: 1.0,
        inside: false,
    }
}

/// A hit on the underside of the block at `pos`.
fn hit_below(pos: BlockPos) -> BlockHit {
    BlockHit {
        pos,
        face: Direction::Down,
        location: DVec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5),
        distance: 1.0,
        inside: false,
    }
}

/// The floor block the crosshair would hit two cells in front of the rig's player
/// (whose box never reaches the placement cell above it).
const FLOOR_AHEAD: BlockPos = BlockPos::new(0, -1, 2);

/// `Block.getStateForPlacement`: a plain block places its default state.
#[test]
fn a_plain_block_places_its_default_state() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let r = rig(DIRT_ITEM);
    let placed = placement_of(&r, hit_on_top(FLOOR_AHEAD, 1.0), DIRT_ITEM, 0.0);
    assert_eq!(
        placed,
        Some((BlockPos::new(0, 0, 2), Some(BlockStateId(2)))),
        "stone is not replaceable: the neighbour above"
    );
}

/// `RotatedPillarBlock.getStateForPlacement`: the axis is the clicked face's axis.
#[test]
fn a_log_takes_the_clicked_faces_axis() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = rig(OAK_LOG_ITEM);
    // A floating block to click the side of (the floor below it is not replaceable
    // downwards): its north face points at the air cell in front of the player.
    r.w.set(0, 2, 3, STONE);
    let b = &fixture().blocks;
    let up = placement_of(&r, hit_on_top(FLOOR_AHEAD, 1.0), OAK_LOG_ITEM, 0.0)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(b.property_value(up, "axis"), Some("y"));
    let side = placement_of(
        &r,
        hit_on_side(BlockPos::new(0, 2, 3), 0.5),
        OAK_LOG_ITEM,
        0.0,
    )
    .unwrap()
    .1
    .unwrap();
    assert_eq!(
        b.property_value(side, "axis"),
        Some("z"),
        "a north/south face gives the z axis"
    );
}

/// `SlabBlock.getStateForPlacement`: bottom on an up face with the click in the lower
/// half, top on a down face, top on a side face with the click in the upper half, and
/// the `waterlogged` property follows a water source at the position.
#[test]
fn a_slab_places_bottom_or_top_by_the_click() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = rig(OAK_SLAB_ITEM);
    // A floating stone block to click from below.
    r.w.set(0, 2, 3, STONE);
    let b = &fixture().blocks;
    let on = |h: BlockHit| placement_of(&r, h, OAK_SLAB_ITEM, 0.0).unwrap().1.unwrap();
    let low = on(hit_on_top(FLOOR_AHEAD, 0.3));
    assert_eq!(b.property_value(low, "type"), Some("bottom"));
    let under = on(hit_below(BlockPos::new(0, 2, 3)));
    assert_eq!(b.property_value(under, "type"), Some("top"));
    let side_low = on(hit_on_side(BlockPos::new(0, 2, 3), 0.3));
    assert_eq!(b.property_value(side_low, "type"), Some("bottom"));
    let side_high = on(hit_on_side(BlockPos::new(0, 2, 3), 0.7));
    assert_eq!(b.property_value(side_high, "type"), Some("top"));
    assert_eq!(
        b.property_value(side_high, "waterlogged"),
        Some("false"),
        "no water at the placement position"
    );
}

/// `SlabBlock.getStateForPlacement`'s double: clicking into a same-kind slab (its top
/// half, with the same item) places into the slab's own cell and doubles it, dry.
#[test]
fn a_slab_clicked_into_its_own_half_doubles() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = rig(OAK_SLAB_ITEM);
    let b = &fixture().blocks;
    // A bottom oak_slab on the floor; the crosshair hits its top face.
    r.w.set(FLOOR_AHEAD.x, FLOOR_AHEAD.y, FLOOR_AHEAD.z, BlockStateId(7));
    let (pos, state) = placement_of(&r, hit_on_top(FLOOR_AHEAD, 1.0), OAK_SLAB_ITEM, 0.0).unwrap();
    assert_eq!(pos, FLOOR_AHEAD, "the replaceable slab's own cell");
    let state = state.unwrap();
    assert_eq!(b.property_value(state, "type"), Some("double"));
    assert_eq!(
        b.property_value(state, "waterlogged"),
        Some("false"),
        "the double displaces the fluid"
    );
    // Clicking its underside with the same item does not replace it (a bottom slab
    // only takes a click from above): the cell below takes the slab, top half (the
    // clicked face is DOWN). A floating slab, so the cell below is air.
    let floating = BlockPos::new(0, 2, 3);
    r.w.set(floating.x, floating.y, floating.z, BlockStateId(7));
    let (pos, state) = placement_of(&r, hit_below(floating), OAK_SLAB_ITEM, 0.0).unwrap();
    assert_eq!(pos, BlockPos::new(0, 1, 3));
    assert_eq!(b.property_value(state.unwrap(), "type"), Some("top"));
}

/// `SlabBlock.getStateForPlacement`'s `waterlogged`: placing into a water source leaves
/// the slab waterlogged.
#[test]
fn a_slab_placed_into_water_is_waterlogged() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = rig(OAK_SLAB_ITEM);
    let b = &fixture().blocks;
    // The cell above the clicked floor block is water (replaceable): the slab goes
    // there, waterlogged.
    let target = BlockPos::new(0, 0, 2);
    r.w.set(target.x, target.y, target.z, WATER);
    let (pos, state) = placement_of(&r, hit_on_top(FLOOR_AHEAD, 1.0), OAK_SLAB_ITEM, 0.0).unwrap();
    assert_eq!(pos, target);
    assert_eq!(
        b.property_value(state.unwrap(), "waterlogged"),
        Some("true")
    );
}

/// `StairBlock.getStateForPlacement`: `facing` from the player's horizontal direction,
/// `half` by the click, `waterlogged` from the fluid at the position.
#[test]
fn stairs_take_the_players_facing_and_the_clicks_half() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = rig(OAK_STAIRS_ITEM);
    r.w.set(0, 2, 3, STONE);
    let b = &fixture().blocks;
    let under = BlockPos::new(0, 2, 3);
    let place = |h: BlockHit, yaw: f32| {
        placement_of(&r, h, OAK_STAIRS_ITEM, yaw)
            .unwrap()
            .1
            .unwrap()
    };
    // Yaw 0 faces south (`Direction.fromYRot(0)`); the click on the floor's top face
    // with the cursor in the lower half is a bottom stair.
    let south = place(hit_on_top(FLOOR_AHEAD, 0.3), 0.0);
    assert_eq!(b.property_value(south, "facing"), Some("south"));
    assert_eq!(b.property_value(south, "half"), Some("bottom"));
    assert_eq!(b.property_value(south, "waterlogged"), Some("false"));
    // Yaw 180 faces north; the underside click is a top stair.
    let north = place(hit_below(under), 180.0);
    assert_eq!(b.property_value(north, "facing"), Some("north"));
    assert_eq!(b.property_value(north, "half"), Some("top"));
    // Yaw 90 faces west, 270 east.
    assert_eq!(
        b.property_value(place(hit_on_top(FLOOR_AHEAD, 0.3), 90.0), "facing"),
        Some("west")
    );
    assert_eq!(
        b.property_value(place(hit_on_top(FLOOR_AHEAD, 0.3), 270.0), "facing"),
        Some("east")
    );
}

/// A block class whose `getStateForPlacement` is not ported (a door's is its own):
/// nothing is predicted, but the placement itself still happens (the position and the
/// obstruction check answer as before).
#[test]
fn an_uncovered_class_places_without_a_prediction() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let r = rig(OAK_DOOR_ITEM);
    let (pos, state) = placement_of(&r, hit_on_top(FLOOR_AHEAD, 1.0), OAK_DOOR_ITEM, 0.0).unwrap();
    assert_eq!(pos, BlockPos::new(0, 0, 2));
    assert_eq!(state, None, "the door's placement state is the server's");
}

/// The obstruction check runs against the placement state (`canPlace` reads the state
/// `getStateForPlacement` returned): a bottom slab fits under an entity that a full
/// cube (the block's default state) would collide with.
#[test]
fn the_obstruction_check_uses_the_placement_state() {
    if !dump_present() {
        eprintln!("no block_classes.json in the oracle dir: placement is not predicted");
        return;
    }
    let mut r = rig(OAK_SLAB_ITEM);
    // An entity floating in the upper half of the placement cell (0, 0, 2): its box
    // starts at y 0.6, above a bottom slab's 0.5 but inside a full cube's 1.0.
    r.add_entity(
        EntityId(9),
        Aabb::entity(DVec3::new(0.5, 0.6, 2.5), 0.6, 0.3),
        RegistryIndex(0),
    );
    let slab = placement_of(&r, hit_on_top(FLOOR_AHEAD, 1.0), OAK_SLAB_ITEM, 0.0);
    assert_eq!(
        slab,
        Some((BlockPos::new(0, 0, 2), Some(BlockStateId(8)))),
        "the dry bottom slab is the default state",
    );
    // The same cell with a full-cube block (dirt): the entity obstructs it.
    r.p.int.hotbar[0] = item(DIRT_ITEM);
    let dirt = placement_of(&r, hit_on_top(FLOOR_AHEAD, 1.0), DIRT_ITEM, 0.0);
    assert_eq!(dirt, None, "the full cube reaches the entity's box");
}
