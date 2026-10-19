//! Minecart state: the model, the render state's timings and defaults, and the old-behavior
//! rail snapping, each against vanilla's constants (26.3 source,
//! `net.minecraft.client.model.object.cart.MinecartModel`,
//! `net.minecraft.client.renderer.entity.AbstractMinecartRenderer`,
//! `net.minecraft.world.entity.vehicle.minecart.OldMinecartBehavior`).
use haste_core::entity::{MetadataEntry, MetadataValue};
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, DVec3};
use haste_data::BlockStates;

use crate::metadata::Metadata;
use crate::minecart::{self, Variant};

/// Two positions within a billionth of a block (the rail math compounds a few f64 rounds,
/// like vanilla's doubles do).
fn close(a: DVec3, b: DVec3) -> bool {
    (a - b).length_squared() < 1e-12
}

fn metadata(entries: Vec<MetadataEntry>) -> Metadata {
    let mut m = Metadata::default();
    m.apply(&entries);
    m
}

fn entry(index: u8, value: MetadataValue) -> MetadataEntry {
    MetadataEntry { index, value }
}

/// A floor of rails at `y`: every block on that level is the one state `state`, everything
/// else air (the block under a rail is what a real floor is).
struct RailWorld {
    state: BlockStateId,
    y: i32,
}

impl BlockView for RailWorld {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        if pos.y == self.y {
            self.state
        } else {
            BlockStateId(0)
        }
    }
    fn is_chunk_loaded(&self, _pos: haste_core::ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, _pos: BlockPos) -> u8 {
        15
    }
    fn block_light(&self, _pos: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        384
    }
}

/// A blocks.json with one rail block whose `shape` property carries each value, state ids
/// dense from 1 (`shape_values[i]` is state id `i + 1`; id 0 is air).
fn rail_states(shape_values: &[&'static str]) -> BlockStates {
    let states: Vec<String> = shape_values
        .iter()
        .enumerate()
        .map(|(i, shape)| {
            format!(
                r#"{{ "id": {}, "default": {}, "properties": {{ "shape": "{}" }} }}"#,
                i + 1,
                i == 0,
                shape
            )
        })
        .collect();
    let json = format!(
        r#"{{
  "minecraft:air": {{ "states": [ {{ "id": 0, "default": true }} ] }},
  "minecraft:rail": {{
    "properties": {{ "shape": {:?} }},
    "states": [ {} ]
  }}
}}"#,
        shape_values,
        states.join(", ")
    );
    BlockStates::parse(&json, None).unwrap()
}

#[test]
fn model_matches_vanillas_layer() {
    let m = minecart::model();
    assert_eq!(m.tex_width, 64);
    assert_eq!(m.tex_height, 32);
    assert_eq!(
        m.texture,
        haste_core::Identifier::minecraft("entity/minecart/minecart")
    );
    let names: Vec<&str> = m.root.children.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["bottom", "front", "back", "left", "right"]);
    // `MinecartModel.createBodyLayer`: every part pivots at y = 4 model units, the bottom
    // rotated 90 degrees about x, the sides a quarter, half or three-quarter turn about y.
    let by = |name: &str| m.root.children.iter().find(|p| p.name == name).unwrap();
    let bottom = by("bottom");
    assert_eq!(bottom.pivot, glam::Vec3::new(0.0, 4.0, 0.0));
    assert_eq!(bottom.rotation.x, std::f32::consts::FRAC_PI_2);
    assert_eq!(bottom.cubes[0].uv, [0, 10]);
    assert_eq!(bottom.cubes[0].size, glam::Vec3::new(20.0, 16.0, 2.0));
    let front = by("front");
    assert_eq!(front.pivot, glam::Vec3::new(-9.0, 4.0, 0.0));
    assert_eq!(front.rotation.y, 3.0 * std::f32::consts::FRAC_PI_2);
    assert_eq!(front.cubes[0].uv, [0, 0]);
    assert_eq!(front.cubes[0].size, glam::Vec3::new(16.0, 8.0, 2.0));
    assert_eq!(by("back").rotation.y, std::f32::consts::FRAC_PI_2);
    assert_eq!(by("left").rotation.y, std::f32::consts::PI);
    assert_eq!(by("right").rotation, glam::Vec3::ZERO);
    assert_eq!(by("right").pivot, glam::Vec3::new(0.0, 4.0, 7.0));
}

#[test]
fn variant_names_cover_the_seven_types() {
    assert_eq!(
        minecart::variant_for_name("minecraft:minecart"),
        Some(Variant::Plain)
    );
    assert_eq!(
        minecart::variant_for_name("minecraft:chest_minecart"),
        Some(Variant::Chest)
    );
    assert_eq!(
        minecart::variant_for_name("minecraft:spawner_minecart"),
        Some(Variant::Spawner)
    );
    assert_eq!(minecart::variant_for_name("minecraft:pig"), None);
}

#[test]
fn render_state_timings_follow_vanillas_extraction() {
    // hurtTime 10, damage 6, hurtDir -1 (the synched data), read mid-tick at partial 0.5:
    // `hurtTime - partialTicks`, `max(damage - partialTicks, 0)`, the dir as is.
    let m = metadata(vec![
        entry(8, MetadataValue::VarInt(10)),
        entry(9, MetadataValue::VarInt(-1)),
        entry(10, MetadataValue::Float(6.0)),
    ]);
    let d = minecart::decoration(&m, Variant::Plain, 0.5);
    assert_eq!(d.hurt_time, 9.5);
    assert_eq!(d.hurt_dir, -1.0);
    assert_eq!(d.damage_time, 5.5);
    // An unhurt cart: the all-zero hurt dir reads as vanilla's default 1.
    let d = minecart::decoration(&Metadata::default(), Variant::Plain, 0.0);
    assert_eq!(d.hurt_dir, 1.0);
    assert_eq!(d.hurt_time, 0.0);
    assert_eq!(d.damage_time, 0.0);
    // The fuse: `getFuse() - partialTicks + 1` while primed, -1 when not.
    let mut m = metadata(vec![entry(0, MetadataValue::VarInt(0))]);
    m.minecart_fuse = Some(80);
    let d = minecart::decoration(&m, Variant::Tnt, 0.5);
    assert_eq!(d.fuse, 80.5);
    let d = minecart::decoration(&Metadata::default(), Variant::Tnt, 0.5);
    assert_eq!(d.fuse, -1.0);
}

#[test]
fn display_offset_defaults_per_type() {
    // `getDisplayOffset` defaults: 8 for the chest cart, 1 for the hopper cart, 6 otherwise;
    // the synched int (index 12) replaces it.
    let d = minecart::decoration(&Metadata::default(), Variant::Chest, 0.0);
    assert_eq!(d.display_offset, 8);
    let d = minecart::decoration(&Metadata::default(), Variant::Hopper, 0.0);
    assert_eq!(d.display_offset, 1);
    let d = minecart::decoration(&Metadata::default(), Variant::Furnace, 0.0);
    assert_eq!(d.display_offset, 6);
    let m = metadata(vec![entry(12, MetadataValue::VarInt(11))]);
    let d = minecart::decoration(&m, Variant::Plain, 0.0);
    assert_eq!(d.display_offset, 11);
}

#[test]
fn fuse_and_hurt_count_down_like_the_client_tick() {
    let mut m = Metadata::default();
    minecart::prime(&mut m);
    assert_eq!(m.minecart_fuse, Some(minecart::FUSE_TICKS));
    for _ in 0..80 {
        minecart::tick(&mut m);
    }
    assert_eq!(m.minecart_fuse, Some(0));
    // `MinecartTNT.tick` stops at zero; the hurt counters run down by one per tick.
    minecart::tick(&mut m);
    assert_eq!(m.minecart_fuse, Some(0));
    m.minecart_hurt_time = 10;
    m.minecart_damage = 5.5;
    minecart::tick(&mut m);
    assert_eq!(m.minecart_hurt_time, 9);
    assert_eq!(m.minecart_damage, 4.5);
}

/// A cart at x=0.5 on a flat north-south rail in block (0, 64, 0), 0.7 blocks along it: the
/// track's line runs z 0..1 at y 64.0625 (rail top, 1/16 above the floor), so both the
/// snapped position and the ±0.3 endpoints sit on it.
#[test]
fn rail_positions_snap_onto_a_flat_track() {
    let blocks = rail_states(&["north_south", "east_west"]);
    let world = RailWorld {
        state: BlockStateId(1),
        y: 64,
    };
    let (pos, front, back) =
        minecart::rail_positions(&world, &blocks, DVec3::new(0.5, 64.0, 0.7)).unwrap();
    assert!(close(pos, DVec3::new(0.5, 64.0625, 0.7)));
    assert!(close(front, DVec3::new(0.5, 64.0625, 1.0)));
    assert!(close(back, DVec3::new(0.5, 64.0625, 0.4)));
    // A floor of the other shape (state 2, east-west): the cart sits on the same line but
    // snapped along x instead; and a floor that is not a rail at all leaves it unsnapped.
    let world = RailWorld {
        state: BlockStateId(2),
        y: 64,
    };
    let (pos, front, back) =
        minecart::rail_positions(&world, &blocks, DVec3::new(0.7, 64.0, 0.5)).unwrap();
    assert!(close(pos, DVec3::new(0.7, 64.0625, 0.5)));
    assert!(close(front, DVec3::new(1.0, 64.0625, 0.5)));
    assert!(close(back, DVec3::new(0.4, 64.0625, 0.5)));
    let world = RailWorld {
        state: BlockStateId(0),
        y: 64,
    };
    assert!(minecart::rail_positions(&world, &blocks, DVec3::new(0.5, 64.0, 0.7)).is_none());
}

/// An ascending-east rail in block (0, 64, 0): the track climbs from (0.0, 63.5625) west-below
/// to (1.0, 64.0625) east, so the cart at the block's middle sits at y 64.5625 (the line's
/// midpoint plus the +0.5 an ascending track adds, `getPos`'s `yD > 0` branch).
#[test]
fn rail_positions_follow_an_ascending_track() {
    let blocks = rail_states(&["ascending_east", "north_south"]);
    let world = RailWorld {
        state: BlockStateId(1),
        y: 64,
    };
    let (pos, front, back) =
        minecart::rail_positions(&world, &blocks, DVec3::new(0.5, 64.5, 0.5)).unwrap();
    assert!(close(pos, DVec3::new(0.5, 64.5625, 0.5)));
    // Front (+0.3 towards the high end) and back (-0.3) sit on the slope's line.
    assert!((front.x - 0.8).abs() < 1e-9);
    assert!((back.x - 0.2).abs() < 1e-9);
    assert!(front.y > back.y);
    assert!((front.z - 0.5).abs() < 1e-9);
    assert!((back.z - 0.5).abs() < 1e-9);
}

/// A cart standing on a rail one block below its own y (`getCurrentBlockPosOrRailBelow`'s
/// look through the block it stands in) still snaps to that rail.
#[test]
fn rail_positions_look_one_block_down() {
    let blocks = rail_states(&["north_south", "east_west"]);
    let world = RailWorld {
        state: BlockStateId(1),
        y: 64,
    };
    let (pos, _, _) =
        minecart::rail_positions(&world, &blocks, DVec3::new(0.5, 65.0, 0.2)).unwrap();
    assert!(close(pos, DVec3::new(0.5, 64.0625, 0.2)));
}

/// The default display blocks: a chest cart shows `minecraft:chest[facing=north]`, a furnace
/// cart the furnace lit with its fuel flag, a plain cart nothing (`getDefaultDisplayBlockState`).
#[test]
fn default_display_blocks_per_variant() {
    let json = r#"{
  "minecraft:air": { "states": [ { "id": 0, "default": true } ] },
  "minecraft:chest": {
    "properties": { "facing": ["north", "south"], "type": ["single", "left", "right"] },
    "states": [
      { "id": 1, "properties": { "facing": "south", "type": "single" } },
      { "id": 2, "default": true, "properties": { "facing": "north", "type": "single" } },
      { "id": 3, "properties": { "facing": "north", "type": "left" } }
    ]
  },
  "minecraft:furnace": {
    "properties": { "facing": ["north", "south"], "lit": ["false", "true"] },
    "states": [
      { "id": 4, "default": true, "properties": { "facing": "north", "lit": "false" } },
      { "id": 5, "properties": { "facing": "north", "lit": "true" } },
      { "id": 6, "properties": { "facing": "south", "lit": "false" } },
      { "id": 7, "properties": { "facing": "south", "lit": "true" } }
    ]
  }
}"#;
    let blocks = BlockStates::parse(json, None).unwrap();
    let north_chest = blocks
        .default_state(&haste_core::Identifier::minecraft("chest"))
        .and_then(|s| blocks.with_property(s, "facing", "north"))
        .unwrap();
    assert_eq!(north_chest, BlockStateId(2));
    let chest = minecart::default_display_state_in(&blocks, Variant::Chest, false).unwrap();
    assert_eq!(chest, north_chest);
    // The furnace's LIT follows the fuel flag (`MinecartFurnace.getDefaultDisplayBlockState`).
    let furnace = minecart::default_display_state_in(&blocks, Variant::Furnace, true).unwrap();
    assert_eq!(
        blocks.property_value(furnace, "lit"),
        Some("true"),
        "a fueled furnace cart shows the lit furnace"
    );
    assert_eq!(
        minecart::default_display_state_in(&blocks, Variant::Furnace, false).unwrap(),
        BlockStateId(4)
    );
    // A plain cart carries nothing.
    assert_eq!(
        minecart::default_display_state_in(&blocks, Variant::Plain, false),
        None
    );
}
