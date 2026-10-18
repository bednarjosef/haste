use std::sync::OnceLock;

use haste_core::game::GameMode;
use haste_core::input::InputState;
use haste_core::{BlockStateId, DVec3, EntityId, Identifier};
use haste_data::GameData;
use haste_physics::mining::BlockTags;
use haste_physics::testing::{states, TestWorld};
use haste_physics::ClimbableBlocks;
use haste_protocol::packets::play::clientbound as cb;
use haste_protocol::ClientboundPlay;

use super::SyncedClimbables;
use crate::player::interact::InteractEnv;
use crate::player::LocalPlayer;

const ME: EntityId = EntityId(7);

/// A small block-state table written to a temp dir, with the state ids chosen so the ladder's
/// north state is 20 — the physics test kit's `states::LADDER` id, whose ladder-slab geometry
/// the player test below relies on. `minecraft:water`'s 18 levels are id padding.
fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("haste-climbable-test-{}", std::process::id()));
        let reports = dir.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        let level_values = (0..=17)
            .map(|l| format!("\"{l}\""))
            .collect::<Vec<_>>()
            .join(",");
        let level_states = (0..=17)
            .map(|l| {
                format!(
                    r#"{{"id": {}, "default": {}, "properties": {{"level": "{l}"}}}}"#,
                    l + 2,
                    l == 0
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let blocks = format!(
            r#"{{
              "minecraft:air": {{"states": [{{"id": 0, "default": true}}]}},
              "minecraft:stone": {{"states": [{{"id": 1, "default": true}}]}},
              "minecraft:water": {{"properties": {{"level": [{level_values}]}}, "states": [{level_states}]}},
              "minecraft:ladder": {{"properties": {{"facing": ["north", "south"]}}, "states": [
                {{"id": 20, "default": true, "properties": {{"facing": "north"}}}},
                {{"id": 21, "properties": {{"facing": "south"}}}}
              ]}},
              "minecraft:oak_trapdoor": {{"properties": {{"facing": ["north", "south"], "open": ["true", "false"]}}, "states": [
                {{"id": 22, "default": true, "properties": {{"facing": "north", "open": "true"}}}},
                {{"id": 23, "properties": {{"facing": "north", "open": "false"}}}},
                {{"id": 24, "properties": {{"facing": "south", "open": "true"}}}},
                {{"id": 25, "properties": {{"facing": "south", "open": "false"}}}}
              ]}},
              "minecraft:scaffolding": {{"states": [{{"id": 26, "default": true}}]}},
              "minecraft:vine": {{"states": [{{"id": 27, "default": true}}]}}
            }}"#
        );
        std::fs::write(reports.join("blocks.json"), blocks).unwrap();
        let entries = [
            "air",
            "stone",
            "water",
            "ladder",
            "oak_trapdoor",
            "scaffolding",
            "vine",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| format!(r#""minecraft:{n}": {{"protocol_id": {i}}}"#))
        .collect::<Vec<_>>()
        .join(",");
        std::fs::write(
            reports.join("registries.json"),
            format!(r#"{{"minecraft:block": {{"entries": {{{entries}}}}}}}"#),
        )
        .unwrap();
        // Stone (the floor and wall) a full cube, the ladder state 20 its 3/16 slab; every
        // other state is empty (nothing the test touches needs geometry).
        let shapes = r#"{"version": 1, "states": [
          {"id": 0, "collision": [], "outline": [], "light": 0, "air": true, "opaque": false},
          {"id": 1, "collision": [[0.0,0.0,0.0,1.0,1.0,1.0]], "outline": [[0.0,0.0,0.0,1.0,1.0,1.0]], "light": 0, "air": false, "opaque": true},
          {"id": 20, "collision": [[0.0,0.0,0.8125,1.0,1.0,1.0]], "outline": [[0.0,0.0,0.8125,1.0,1.0,1.0]], "light": 0, "air": false, "opaque": false}
        ]}"#;
        std::fs::write(dir.join("block_shapes.json"), shapes).unwrap();
        GameData::load_from(&reports, Some(&dir.join("block_shapes.json"))).unwrap()
    })
}

/// The tag set the server would sync: ladders, scaffolding and vines climbable, the trapdoor in
/// `#minecraft:trapdoors`, the vines glidable-through.
fn tags() -> BlockTags {
    // Registry protocol ids in the fixture: ladder 3, oak_trapdoor 4, scaffolding 5, vine 6.
    BlockTags::new([
        (Identifier::minecraft("climbable"), vec![3, 5, 6]),
        (Identifier::minecraft("trapdoors"), vec![4]),
        (Identifier::minecraft("can_glide_through"), vec![6]),
    ])
}

/// The states the trapdoor rule reads, resolved through the fixture's own state table.
fn fixture_states(data: &GameData) -> (BlockStateId, BlockStateId, BlockStateId, BlockStateId) {
    let state = |block: &str, props: &[(&str, &str)]| {
        data.blocks
            .state(&Identifier::minecraft(block), props)
            .unwrap()
    };
    (
        state("ladder", &[("facing", "north")]),
        state("oak_trapdoor", &[("facing", "north"), ("open", "true")]),
        state("oak_trapdoor", &[("facing", "north"), ("open", "false")]),
        state("oak_trapdoor", &[("facing", "south"), ("open", "true")]),
    )
}

/// The tags decide: without them nothing is climbable (like `NoClimbables`), with them the
/// tag's blocks climb (a modded server can put any block in the tag). Unknown state ids
/// resolve to the placeholder block and do not climb.
#[test]
fn tags_decide_what_climbs() {
    let data = data();
    let (ladder, _, _, _) = fixture_states(data);
    let stone = data
        .blocks
        .default_state(&Identifier::minecraft("stone"))
        .unwrap();
    let tags = tags();
    let climb = SyncedClimbables::new(Some(&tags), Some(&data.blocks));
    assert!(climb.is_climbable(ladder));
    assert!(!climb.is_climbable(stone));
    assert!(!climb.is_climbable(BlockStateId(9999)));
    // No tags yet: nothing climbs.
    let none = SyncedClimbables::new(None, Some(&data.blocks));
    assert!(!none.is_climbable(ladder));
    // No state table: the tags alone cannot resolve a state to its block.
    let no_table = SyncedClimbables::new(Some(&tags), None);
    assert!(!no_table.is_climbable(ladder));
}

/// `#minecraft:can_glide_through` and `minecraft:scaffolding` resolve from the data too.
#[test]
fn glide_through_and_scaffolding_come_from_the_data() {
    let data = data();
    let vine = data
        .blocks
        .default_state(&Identifier::minecraft("vine"))
        .unwrap();
    let scaffolding = data
        .blocks
        .default_state(&Identifier::minecraft("scaffolding"))
        .unwrap();
    let tags = tags();
    let climb = SyncedClimbables::new(Some(&tags), Some(&data.blocks));
    assert!(climb.can_glide_through(vine));
    assert!(!climb.can_glide_through(scaffolding));
    assert!(climb.is_scaffolding(scaffolding));
    assert!(!climb.is_scaffolding(vine));
    // Without tags or table: neither.
    let none = SyncedClimbables::new(None, None);
    assert!(!none.can_glide_through(vine));
    assert!(!none.is_scaffolding(scaffolding));
}

/// Vanilla `LivingEntity.trapdoorUsableAsLadder`: an open trapdoor over a ladder facing the
/// same way. Closed, differently facing or over another block: not a ladder.
#[test]
fn trapdoor_usable_as_ladder_follows_vanilla() {
    let data = data();
    let (ladder, trapdoor_open, trapdoor_closed, trapdoor_south) = fixture_states(data);
    let stone = data
        .blocks
        .default_state(&Identifier::minecraft("stone"))
        .unwrap();
    let tags = tags();
    let climb = SyncedClimbables::new(Some(&tags), Some(&data.blocks));
    assert!(climb.trapdoor_usable_as_ladder(trapdoor_open, ladder));
    assert!(!climb.trapdoor_usable_as_ladder(trapdoor_closed, ladder));
    assert!(!climb.trapdoor_usable_as_ladder(trapdoor_south, ladder));
    assert!(!climb.trapdoor_usable_as_ladder(trapdoor_open, stone));
    // Without the state table the properties cannot be read: not a ladder.
    let none = SyncedClimbables::new(Some(&tags), None);
    assert!(!none.trapdoor_usable_as_ladder(trapdoor_open, ladder));
}

/// Pushing into a ladder on a wall makes the player rise when the server synced the
/// `#minecraft:climbable` tag, and not without it — through the client's own tick (the one the
/// real driver runs, with the interactions environment carrying the game data).
#[test]
fn pushing_into_a_ladder_climbs_only_with_the_synced_tag() {
    let data = data();
    let ladder = data
        .blocks
        .state(&Identifier::minecraft("ladder"), &[("facing", "north")])
        .unwrap();
    // The same rig as the haste-physics climbing tests: a stone floor whose surface is y = 0,
    // a ladder column on a stone wall at z = 1 (the ladder state resolves to the test kit's
    // ladder id, so its slab geometry applies).
    assert_eq!(ladder, states::LADDER);
    let mut world = TestWorld::flat(0, states::STONE);
    for y in 0..=6 {
        world.set(0, y, 0, ladder);
    }
    world.fill((-1, 0, 1), (1, 7, 1), states::STONE);
    let env = InteractEnv {
        data,
        entities: &[],
        entity_types: &[],
        border: None,
    };
    let forward = InputState {
        forward: true,
        ..Default::default()
    };
    let run = |tags: Option<BlockTags>| -> f64 {
        let mut p = LocalPlayer::with_entity(ME, GameMode::Survival);
        if let Some(t) = tags {
            p.int.set_block_tags(t);
        }
        let mut out = Vec::new();
        assert!(p.handle(
            &ClientboundPlay::PlayerPosition(cb::PlayerPosition {
                teleport_id: 1,
                pos: DVec3::new(0.5, 0.0, 0.5),
                velocity: DVec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
            }),
            &mut out
        ));
        p.tick_interact(&InputState::default(), &world, &env, None, &mut out);
        p.tick_interact(&InputState::default(), &world, &env, None, &mut out);
        let y0 = p.state.pos.y;
        for _ in 0..30 {
            p.tick_interact(&forward, &world, &env, None, &mut out);
        }
        p.state.pos.y - y0
    };
    // With the tag: the player climbs the column (about 0.1176 blocks per tick).
    let rose = run(Some(tags()));
    assert!(rose > 2.0, "climbed only {rose} blocks with the tag");
    // Without tags: nothing climbs, the player pushes against the ladder's slab on the floor.
    let idle = run(None);
    assert!(idle.abs() < 1e-9, "rose {idle} blocks without the tag");
}
