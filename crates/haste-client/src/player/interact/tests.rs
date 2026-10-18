use std::sync::{Arc, OnceLock};

use haste_core::input::InputState;
use haste_core::{Identifier, RegistryIndex};
use haste_physics::mining::{BlockTags, Tool, ToolTable};
use haste_physics::testing::TestWorld;

use super::pick::is_pickable;
use super::use_item::UseAnimation;
use super::*;
use haste_core::game::GameMode;
use haste_core::view::BlockView;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ClientboundPlay;
use haste_protocol::ServerboundPlay;

const ME: EntityId = EntityId(7);
const STONE: BlockStateId = BlockStateId(1);
const DIRT: BlockStateId = BlockStateId(2);
const BEDROCK: BlockStateId = BlockStateId(3);
const GRASS: BlockStateId = BlockStateId(4);

// `minecraft:entity_type` registry indexes in the test GameData (registries.json below).
const ITEM_ENTITY: RegistryIndex = RegistryIndex(0);
const EXPERIENCE_ORB: RegistryIndex = RegistryIndex(1);
const ARROW: RegistryIndex = RegistryIndex(2);
const SPECTRAL_ARROW: RegistryIndex = RegistryIndex(3);
const ZOMBIE: RegistryIndex = RegistryIndex(5);
const PLAYER: RegistryIndex = RegistryIndex(6);
const ENDER_DRAGON: RegistryIndex = RegistryIndex(7);
const VILLAGER: RegistryIndex = RegistryIndex(12);

/// A small GameData written to a temp dir: air, stone (1.5, needs a tool), dirt (0.5),
/// bedrock (-1), short grass (0, no collision), obsidian (50, needs a tool).
fn data() -> &'static haste_data::GameData {
    static D: OnceLock<haste_data::GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("haste-interact-test-{}", std::process::id()));
        let reports = dir.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        let names = ["air", "stone", "dirt", "bedrock", "short_grass", "obsidian"];
        let blocks: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, n)| format!(r#""minecraft:{n}": {{"states": [{{"id": {i}, "default": true}}]}}"#))
            .collect();
        std::fs::write(reports.join("blocks.json"), format!("{{{}}}", blocks.join(","))).unwrap();
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
                    "minecraft:mob_effect": {{"entries": {{{}}}}},
                    "minecraft:data_component_type": {{"entries": {{{}}}}},
                    "minecraft:enchantment": {{"entries": {{{}}}}},
                    "minecraft:entity_type": {{"entries": {{{}}}}}}}"#,
                entries(&names),
                entries(&[
                    "air",
                    "diamond_pickaxe",
                    "diamond_sword",
                    "stick",
                    "apple",
                    "bow",
                    "crossbow",
                    "trident",
                    "spyglass",
                    "shield",
                    "potion",
                ]),
                entries(&[
                    "speed",
                    "haste",
                    "mining_fatigue",
                    "conduit_power",
                    "nausea",
                    "darkness",
                ]),
                entries(&["tool", "enchantments"]),
                entries(&["efficiency", "sharpness"]),
                entries(&[
                    "item",
                    "experience_orb",
                    "arrow",
                    "spectral_arrow",
                    "trident",
                    "zombie",
                    "player",
                    "ender_dragon",
                    "interaction",
                    "fireball",
                    "wind_charge",
                    "shulker_bullet",
                    "villager",
                ]),
            ),
        )
        .unwrap();
        let full = "[[0.0,0.0,0.0,1.0,1.0,1.0]]";
        let shape = |i: usize, c: &str, o: &str| {
            format!(r#"{{"id": {i}, "collision": {c}, "outline": {o}, "light": 0, "air": {}, "opaque": false}}"#, i == 0)
        };
        let states = [
            shape(0, "[]", "[]"),
            shape(1, full, full),
            shape(2, full, full),
            shape(3, full, full),
            shape(4, "[]", "[[0.125,0.0,0.125,0.875,0.8125,0.875]]"),
            shape(5, full, full),
        ];
        std::fs::write(
            dir.join("block_shapes.json"),
            format!(r#"{{"version": 1, "states": [{}]}}"#, states.join(",")),
        )
        .unwrap();
        let props = |n: &str, d: f32, c: bool| {
            format!(r#""minecraft:{n}": {{"destroy_time": {d:?}, "requires_correct_tool_for_drops": {c}}}"#)
        };
        std::fs::write(
            dir.join("block_properties.json"),
            format!(
                r#"{{"version": 1, "blocks": {{{}}}}}"#,
                [
                    props("air", 0.0, false),
                    props("stone", 1.5, true),
                    props("dirt", 0.5, false),
                    props("bedrock", -1.0, false),
                    props("short_grass", 0.0, false),
                    props("obsidian", 50.0, true),
                ]
                .join(",")
            ),
        )
        .unwrap();
        haste_data::GameData::load_from(&reports, Some(&dir.join("block_shapes.json"))).unwrap()
    })
}

fn tools() -> Arc<ToolTable> {
    use haste_physics::mining::{BlockSet, ToolRule};
    let tag = |t: &str| BlockSet::Tag(Identifier::minecraft(t));
    let mut t = ToolTable::default();
    t.insert(
        Identifier::minecraft("diamond_pickaxe"),
        Tool {
            rules: vec![
                ToolRule {
                    blocks: tag("incorrect_for_diamond_tool"),
                    speed: None,
                    correct_for_drops: Some(false),
                },
                ToolRule {
                    blocks: tag("mineable/pickaxe"),
                    speed: Some(8.0),
                    correct_for_drops: Some(true),
                },
            ],
            default_mining_speed: 1.0,
            damage_per_block: 1,
            can_destroy_blocks_in_creative: true,
        },
    );
    t.insert(
        Identifier::minecraft("diamond_sword"),
        Tool {
            rules: vec![ToolRule {
                blocks: BlockSet::Block(Identifier::minecraft("cobweb")),
                speed: Some(15.0),
                correct_for_drops: None,
            }],
            default_mining_speed: 1.0,
            damage_per_block: 2,
            can_destroy_blocks_in_creative: false,
        },
    );
    Arc::new(t)
}

fn item(i: u32) -> ItemStackData {
    ItemStackData {
        count: 1,
        item: Some(RegistryIndex(i)),
        components_raw: Default::default(),
    }
}

/// Item default components for the tests, so they do not need the data generator cache.
fn item_components() -> Arc<haste_data::ItemComponents> {
    use haste_data::{AttributeModifier, Consumable, ItemComponents};
    let speed = |amount: f64| AttributeModifier {
        attribute: Identifier::minecraft("attack_speed"),
        amount,
        operation: 0,
        slot: "mainhand".into(),
    };
    let mut c = ItemComponents::default();
    c.insert(Identifier::minecraft("diamond_sword"), vec![speed(-2.4)]);
    c.insert(Identifier::minecraft("diamond_pickaxe"), vec![speed(-2.8)]);
    c.insert_consumable(
        Identifier::minecraft("apple"),
        Consumable {
            consume_seconds: 1.6,
            animation: 1,
        },
    );
    c.insert_food(Identifier::minecraft("apple"), false);
    c.insert_consumable(
        Identifier::minecraft("potion"),
        Consumable {
            consume_seconds: 1.6,
            animation: 2,
        },
    );
    Arc::new(c)
}

struct Rig {
    p: LocalPlayer,
    w: TestWorld,
    entities: Vec<(EntityId, Aabb)>,
    types: Vec<RegistryIndex>,
}

impl Rig {
    /// Player at (0.5, 0, 0.5) on a stone floor (top y = 0), looking straight down at (0, -1, 0).
    fn new(mode: GameMode) -> Rig {
        let mut p = LocalPlayer::with_entity(ME, mode);
        p.int.set_tools(tools());
        p.int.set_block_tags(BlockTags::new([(
            Identifier::minecraft("mineable/pickaxe"),
            vec![1, 5],
        )]));
        let mut out = Vec::new();
        p.handle(
            &ClientboundPlay::PlayerPosition(
                haste_protocol::packets::play::clientbound::PlayerPosition {
                    teleport_id: 1,
                    pos: DVec3::new(0.5, 0.0, 0.5),
                    velocity: DVec3::ZERO,
                    yaw: 0.0,
                    pitch: 90.0,
                    flags: 0,
                },
            ),
            &mut out,
        );
        let mut r = Rig {
            p,
            w: TestWorld::flat(0, STONE),
            entities: Vec::new(),
            types: Vec::new(),
        };
        r.tick(InteractInput::default());
        r.tick(InteractInput::default());
        r.tick(InteractInput::default());
        r
    }

    /// An entity the crosshair could hit, with its `minecraft:entity_type` registry index.
    fn add_entity(&mut self, id: EntityId, aabb: Aabb, type_index: RegistryIndex) {
        self.entities.push((id, aabb));
        self.types.push(type_index);
    }

    fn tick(&mut self, i: InteractInput) -> Vec<ServerboundPlay> {
        self.p.set_interact(i);
        let mut out = Vec::new();
        let env = InteractEnv {
            data: data(),
            entities: &self.entities,
            entity_types: &self.types,
            warm_blocks: None,
        };
        let input = InputState {
            pitch: 90.0,
            ..Default::default()
        };
        self.p.tick_interact(&input, &self.w, &env, None, &mut out);
        for (pos, s) in self.p.take_world_edits().collect::<Vec<_>>() {
            self.w.set(pos.x, pos.y, pos.z, s);
        }
        // Only the interaction packets.
        out.retain(|p| {
            !matches!(
                p,
                ServerboundPlay::PlayerInput(_)
                    | ServerboundPlay::ClientTickEnd(_)
                    | ServerboundPlay::MovePlayerPos(_)
                    | ServerboundPlay::MovePlayerPosRot(_)
                    | ServerboundPlay::MovePlayerRot(_)
                    | ServerboundPlay::MovePlayerStatusOnly(_)
            )
        });
        out
    }
}

const ATTACK: InteractInput = InteractInput {
    attack: true,
    use_item: false,
    pick_block: false,
    drop_item: false,
    drop_stack: false,
    swap_offhand: false,
    hotbar: None,
    attack_clicks: 0,
    use_clicks: 0,
};

fn actions(out: &[ServerboundPlay]) -> Vec<(i32, i32)> {
    out.iter()
        .filter_map(|p| match p {
            ServerboundPlay::PlayerAction(a) => Some((a.status, a.sequence)),
            _ => None,
        })
        .collect()
}

/// Ticks of held attack until the finish packet, counting the press tick as 1.
fn ticks_to_finish(r: &mut Rig) -> u32 {
    let first = r.tick(ATTACK);
    assert_eq!(actions(&first)[0].0, action::START_DESTROY);
    for t in 2..1000 {
        let out = r.tick(ATTACK);
        if actions(&out).iter().any(|a| a.0 == action::STOP_DESTROY) {
            return t;
        }
    }
    panic!("never finished");
}

#[test]
fn target_is_the_block_below_with_its_top_face() {
    let r = Rig::new(GameMode::Survival);
    assert_eq!(r.p.target(), Some((BlockPos::new(0, -1, 0), Direction::Up)));
}

#[test]
fn survival_stone_by_hand_takes_151_ticks_then_breaks() {
    let mut r = Rig::new(GameMode::Survival);
    // wiki "Breaking": stone by hand 7.5 s = 150 ticks; adding 1/150 in f32 per tick (as the
    // progress is a float) reaches 1.0 only on the 151st tick.
    assert_eq!(ticks_to_finish(&mut r), 151);
    assert_eq!(r.w.block_state(BlockPos::new(0, -1, 0)), BlockStateId::AIR);
    assert!(r.p.break_progress().is_none());
}

#[test]
fn diamond_pickaxe_breaks_stone_in_6_ticks_and_sequences_count_up() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(1);
    let first = r.tick(ATTACK);
    assert_eq!(actions(&first), [(action::START_DESTROY, 1)]);
    assert!(matches!(first.last(), Some(ServerboundPlay::Punch(_))));
    let mut stages = Vec::new();
    for _ in 2..=5 {
        let out = r.tick(ATTACK);
        assert!(actions(&out).is_empty());
        stages.push(r.p.break_progress().unwrap().1);
    }
    assert_eq!(stages, [3, 5, 7, 8]); // 8 / 1.5 / 30 = 0.178 per tick
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::STOP_DESTROY, 2)]);
}

#[test]
fn instant_blocks_break_on_start_alone() {
    let mut r = Rig::new(GameMode::Survival);
    r.w.set(0, 0, 0, GRASS);
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 1)]);
    assert_eq!(r.w.block_state(BlockPos::new(0, 0, 0)), BlockStateId::AIR);
    // Next tick the stone below is the new target: a new start.
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 2)]);
}

#[test]
fn releasing_or_unbreakable() {
    let mut r = Rig::new(GameMode::Survival);
    r.w.set(0, -1, 0, DIRT);
    r.tick(ATTACK);
    r.tick(ATTACK);
    assert!(r.p.break_progress().is_some());
    let out = r.tick(InteractInput::default());
    assert_eq!(actions(&out), [(action::ABORT_DESTROY, 0)]);
    assert!(r.p.break_progress().is_none());
    // Bedrock: start is sent, nothing ever finishes.
    r.w.set(0, -1, 0, BEDROCK);
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 2)]);
    for _ in 0..50 {
        assert!(actions(&r.tick(ATTACK)).is_empty());
    }
}

#[test]
fn creative_breaks_at_once_then_every_six_ticks_and_not_with_a_sword() {
    let mut r = Rig::new(GameMode::Creative);
    let mut broke_at = Vec::new();
    for t in 1..=13 {
        let out = r.tick(ATTACK);
        let a = actions(&out);
        assert!(a.iter().all(|a| a.0 == action::START_DESTROY));
        if !a.is_empty() {
            broke_at.push(t);
        }
    }
    // The press tick breaks and (endAttack) skips continueAttack; the 5-tick destroyDelay then
    // counts down once per held tick: breaks on ticks 1, 7, 13.
    assert_eq!(broke_at, [1, 7, 13]);
    // A sword cannot break blocks in creative (Tool.canDestroyBlocksInCreative false). Vanilla
    // still sends START_DESTROY_BLOCK every 5 ticks: destroyBlock no-ops (Item.canDestroyBlock
    // fails) but startDestroyBlock/continueDestroyBlock return the packet regardless, so the
    // block is never air and no STOP is ever sent.
    let mut s = Rig::new(GameMode::Creative);
    s.p.int.hotbar[0] = item(2);
    let pos = BlockPos::new(0, -1, 0);
    let mut starts = Vec::new();
    for t in 1..=12 {
        let out = s.tick(ATTACK);
        let a = actions(&out);
        assert!(a.iter().all(|x| x.0 == action::START_DESTROY), "{out:?}");
        if !a.is_empty() {
            starts.push(t);
        }
        assert_eq!(s.w.block_state(pos), STONE);
    }
    assert_eq!(starts, [1, 6, 12]);
}

#[test]
fn creative_single_tick_click_destroys_even_while_destroy_delay_counts_down() {
    // The bug: a quick click broke nothing because start_destroy refused while destroy_delay
    // (set by the previous break) was still counting down. Vanilla startDestroyBlock has no
    // destroyDelay check — a fresh click always breaks at once and re-arms the 5-tick delay
    // (which only gates continueDestroyBlock while the button is held).
    let mut r = Rig::new(GameMode::Creative);
    let pos = BlockPos::new(0, -1, 0);
    // First click breaks the block and leaves destroy_delay = 5 counting down.
    let first = r.tick(ATTACK);
    assert_eq!(actions(&first), [(action::START_DESTROY, 1)]);
    assert_eq!(r.w.block_state(pos), BlockStateId::AIR);
    // The server confirms the break, so the spot is air on both sides.
    assert!(r.p.defer_block_update(pos, BlockStateId::AIR));
    let mut out = Vec::new();
    r.p.handle(
        &ClientboundPlay::BlockChangedAck(
            haste_protocol::packets::play::clientbound::BlockChangedAck { sequence: 1 },
        ),
        &mut out,
    );
    // Release for a few ticks: the delay never reaches 0 (it only counts down while the
    // button is held, in continueDestroyBlock).
    for _ in 0..3 {
        assert!(r.tick(InteractInput::default()).is_empty());
    }
    // A fresh single-tick click on a new block at the same spot must break it at once and
    // send START_DESTROY exactly once, despite the running delay.
    r.w.set(0, -1, 0, STONE);
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 2)]);
    assert_eq!(r.w.block_state(pos), BlockStateId::AIR);
    // And even on the very next tick after a break (delay still 5): click, release, click.
    assert!(r.p.defer_block_update(pos, BlockStateId::AIR));
    r.p.handle(
        &ClientboundPlay::BlockChangedAck(
            haste_protocol::packets::play::clientbound::BlockChangedAck { sequence: 2 },
        ),
        &mut Vec::new(),
    );
    // Apply the ack's world edit now instead of on the next tick, so the new stone below is
    // not overwritten by it.
    for (p, s) in r.p.take_world_edits().collect::<Vec<_>>() {
        r.w.set(p.x, p.y, p.z, s);
    }
    r.w.set(0, -1, 0, STONE);
    // Release (the fresh press needs its edge), then click again one tick later: the delay
    // is still at 5 — it only counts down on held ticks.
    assert!(r.tick(InteractInput::default()).is_empty());
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 3)]);
    assert_eq!(r.w.block_state(pos), BlockStateId::AIR);
}

#[test]
fn creative_keeps_swinging_on_a_miss_without_miss_time() {
    let mut r = Rig::new(GameMode::Creative);
    r.w = TestWorld::new();
    // hasMissTime() is false in creative: every fresh click on nothing is a miss and a swing
    // (no 10-tick freeze like survival). Holding the button on a miss swings only on the press
    // tick (continueAttack does nothing without a block target), so each click is press+release.
    for _ in 0..5 {
        assert!(matches!(r.tick(ATTACK)[0], ServerboundPlay::Punch(_)));
        assert!(r.tick(InteractInput::default()).is_empty());
    }
}

#[test]
fn a_click_on_nothing_misses_and_freezes_the_held_attack_for_ten_ticks() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    let out = r.tick(ATTACK);
    assert!(matches!(out[0], ServerboundPlay::Punch(_))); // the miss swing
                                                          // Held: continueAttack is gated on missTime <= 0 (Minecraft.continueAttack) — nothing
                                                          // happens for ten ticks, not even a stop.
    for _ in 0..10 {
        assert!(r.tick(ATTACK).is_empty());
    }
}

#[test]
fn releasing_the_attack_button_clears_miss_time() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    assert!(matches!(r.tick(ATTACK)[0], ServerboundPlay::Punch(_)));
    // Released: continueAttack(false) resets missTime (and would stop a break).
    assert!(r.tick(InteractInput::default()).is_empty());
    let out = r.tick(ATTACK);
    assert!(matches!(out[0], ServerboundPlay::Punch(_))); // a fresh miss, not blocked
}

#[test]
fn spam_clicking_at_20_cps_on_air_swings_every_tick() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    // One press+release per tick (20 cps): each fresh press is a miss and swings; the release
    // clears missTime (Minecraft.continueAttack(false)) so the next click is not swallowed by
    // the 10-tick freeze.
    for _ in 0..5 {
        assert!(matches!(r.tick(ATTACK)[0], ServerboundPlay::Punch(_)));
        assert!(r.tick(InteractInput::default()).is_empty());
    }
}

#[test]
fn several_queued_attack_clicks_in_one_tick_each_count() {
    let mut r = Rig::new(GameMode::Survival);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        ZOMBIE,
    );
    // KeyMapping.consumeClick: two clicks queued in one tick run startAttack twice.
    r.p.queue_attack_click();
    r.p.queue_attack_click();
    let out = r.tick(ATTACK);
    let attacks = out
        .iter()
        .filter(|p| matches!(p, ServerboundPlay::Attack(_)))
        .count();
    assert_eq!(attacks, 2, "{out:?}");
}

#[test]
fn three_clicks_in_one_tick_run_start_attack_three_times() {
    let mut r = Rig::new(GameMode::Survival);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        ZOMBIE,
    );
    // Three presses between ticks (faster than the 20 Hz tick): each consumeClick in
    // `while (keyAttack.consumeClick()) instantAttack |= startAttack()` runs startAttack.
    r.p.queue_attack_click();
    r.p.queue_attack_click();
    r.p.queue_attack_click();
    let out = r.tick(ATTACK);
    let attacks = out
        .iter()
        .filter(|p| matches!(p, ServerboundPlay::Attack(_)))
        .count();
    assert_eq!(attacks, 3, "{out:?}");
}

#[test]
fn several_queued_use_clicks_in_one_tick_each_count() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(3);
    // Two right clicks in one tick: startUseItem twice (the consumeClick loop does not wait for
    // rightClickDelay).
    r.p.queue_use_click();
    r.p.queue_use_click();
    let out = r.tick(InteractInput {
        use_item: true,
        ..Default::default()
    });
    let uses = out
        .iter()
        .filter(|p| matches!(p, ServerboundPlay::UseItemOn(_)))
        .count();
    assert_eq!(uses, 2, "{out:?}");
}

#[test]
fn a_miss_delays_the_start_of_a_break_by_ten_ticks() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    assert!(matches!(r.tick(ATTACK)[0], ServerboundPlay::Punch(_)));
    r.w = TestWorld::flat(0, STONE);
    // Holding attack onto the stone: the break starts only when missTime runs out (tick 11) —
    // the freeze, then continueDestroyBlock's startDestroyBlock. No abort, no restart.
    for t in 2..=11 {
        let out = r.tick(ATTACK);
        if actions(&out).iter().any(|a| a.0 == action::START_DESTROY) {
            assert_eq!(t, 11);
            return;
        }
    }
    panic!("never started");
}

#[test]
fn server_updates_wait_for_the_ack() {
    let mut r = Rig::new(GameMode::Creative);
    r.tick(ATTACK);
    let pos = BlockPos::new(0, -1, 0);
    assert_eq!(r.w.block_state(pos), BlockStateId::AIR);
    // A stale server update for the block arrives before the ack: held back.
    assert!(r.p.defer_block_update(pos, STONE));
    assert!(r.p.take_world_edits().next().is_none());
    let mut out = Vec::new();
    r.p.handle(
        &ClientboundPlay::BlockChangedAck(
            haste_protocol::packets::play::clientbound::BlockChangedAck { sequence: 1 },
        ),
        &mut out,
    );
    assert_eq!(r.p.take_world_edits().collect::<Vec<_>>(), [(pos, STONE)]);
    // After the ack, updates go straight to the world.
    assert!(!r.p.defer_block_update(pos, BlockStateId::AIR));
}

#[test]
fn a_silently_rejected_break_is_reverted_on_the_ack() {
    let mut r = Rig::new(GameMode::Creative);
    r.tick(ATTACK);
    let pos = BlockPos::new(0, -1, 0);
    assert_eq!(r.w.block_state(pos), BlockStateId::AIR);
    let mut out = Vec::new();
    r.p.handle(
        &ClientboundPlay::BlockChangedAck(
            haste_protocol::packets::play::clientbound::BlockChangedAck { sequence: 1 },
        ),
        &mut out,
    );
    // No block update came: the server kept its stone.
    assert_eq!(r.p.take_world_edits().collect::<Vec<_>>(), [(pos, STONE)]);
}

#[test]
fn use_on_block_with_cursor_and_repeat_delay() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(3);
    let use_ = InteractInput {
        use_item: true,
        ..Default::default()
    };
    let out = r.tick(use_);
    match &out[0] {
        ServerboundPlay::UseItemOn(u) => {
            assert_eq!((u.pos, u.face, u.sequence), (BlockPos::new(0, -1, 0), 1, 1));
            assert!((u.cursor_y - 1.0).abs() < 1e-6 && (u.cursor_x - 0.5).abs() < 1e-6);
            assert!(!u.inside_block);
        }
        p => panic!("{p:?}"),
    }
    // A stick on stone is a Pass: no punch, `useItem` with the same hand follows.
    assert!(matches!(&out[1], ServerboundPlay::UseItem(u) if u.hand == 0));
    let mut used_at = vec![1];
    for t in 2..=10 {
        if r.tick(use_)
            .iter()
            .any(|p| matches!(p, ServerboundPlay::UseItemOn(_)))
        {
            used_at.push(t);
        }
    }
    assert_eq!(used_at, [1, 5, 9]);
    // Looking up at nothing with an item: use_item.
    r.w = TestWorld::new();
    let out = r.tick(InteractInput::default());
    assert!(out.is_empty());
    let out = r.tick(use_);
    assert!(matches!(out[0], ServerboundPlay::UseItem(_)));
}

#[test]
fn use_with_an_empty_main_hand_uses_the_off_hand() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    r.p.int.offhand = item(3);
    let out = r.tick(InteractInput {
        use_item: true,
        ..Default::default()
    });
    match &out[0] {
        ServerboundPlay::UseItem(u) => assert_eq!(u.hand, 1),
        p => panic!("{p:?}"),
    }
}

#[test]
fn use_does_nothing_while_a_block_is_being_destroyed() {
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 1)]);
    let use_ = InteractInput {
        use_item: true,
        ..Default::default()
    };
    // The right click lands while the stone is breaking: startUseItem is gated on
    // !gameMode.isDestroying(), nothing goes out (the abort below is the attack releasing).
    let out = r.tick(use_);
    let used = out.iter().any(|p| {
        matches!(p, ServerboundPlay::UseItemOn(_)) || matches!(p, ServerboundPlay::UseItem(_))
    });
    assert!(!used, "{out:?}");
    // After the attack is released the same click works.
    r.tick(InteractInput::default());
    let out = r.tick(use_);
    assert!(matches!(out[0], ServerboundPlay::UseItemOn(_)));
}

#[test]
fn while_an_item_is_in_use_clicks_do_nothing_and_releasing_sends_release_use_item() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    let use_ = InteractInput {
        use_item: true,
        ..Default::default()
    };
    r.p.int.set_using_item(true);
    // Holding use: no new use, no release, and the attack click is consumed and discarded.
    assert!(r.tick(use_).is_empty());
    assert!(r
        .tick(InteractInput {
            attack: true,
            ..use_
        })
        .is_empty());
    // Releasing the button: MultiPlayerGameMode.releaseUsingItem.
    assert_eq!(
        actions(&r.tick(InteractInput::default())),
        [(action::RELEASE_USE_ITEM, 0)]
    );
}

#[test]
fn attack_entity_pick_and_hotbar() {
    let mut r = Rig::new(GameMode::Survival);
    // A mob standing on the block we look at, between us and it.
    r.entities.push((
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
    ));
    r.types.push(ZOMBIE);
    // Our eye is inside that box? no: the box ends at y 0.5, our eye is at 1.62.
    let out = r.tick(ATTACK);
    assert!(matches!(
        out[0],
        ServerboundPlay::Attack(sb::Attack { entity_id: 42 })
    ));
    assert!(matches!(out[1], ServerboundPlay::Punch(_)));
    // Held attack on an entity does not attack again.
    assert!(r.tick(ATTACK).is_empty());
    r.entities.clear();
    r.types.clear();
    let out = r.tick(InteractInput {
        pick_block: true,
        hotbar: Some(3),
        ..Default::default()
    });
    assert!(matches!(
        out[0],
        ServerboundPlay::SetCarriedItem(sb::SetCarriedItem { slot: 3 })
    ));
    assert!(matches!(
        out[1],
        ServerboundPlay::PickItemFromBlock(sb::PickItemFromBlock {
            include_data: false,
            ..
        })
    ));
    assert_eq!(r.p.held_slot, 3);
    // Missing: a lone punch.
    r.w = TestWorld::new();
    let out = r.tick(ATTACK);
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], ServerboundPlay::Punch(_)));
}

#[test]
fn inventory_and_effects_from_packets() {
    let mut r = Rig::new(GameMode::Survival);
    let mut out = Vec::new();
    use haste_protocol::packets::play::clientbound as cb;
    r.p.handle(
        &ClientboundPlay::ContainerSetSlot(cb::ContainerSetSlot {
            window_id: 0,
            state_id: 1,
            slot: 36,
            item: item(1),
        }),
        &mut out,
    );
    assert_eq!(r.p.main_hand().item, Some(RegistryIndex(1)));
    r.p.int.prepare(data());
    r.p.handle(
        &ClientboundPlay::UpdateMobEffect(cb::UpdateMobEffect {
            entity_id: ME.0,
            effect: 1,
            amplifier: 1,
            duration: 100,
            flags: 0,
        }),
        &mut out,
    );
    let env = InteractEnv {
        data: data(),
        entities: &[],
        entity_types: &[],
        warm_blocks: None,
    };
    let p = r.p.mining_params(STONE, &env);
    assert_eq!(
        (p.tool_speed, p.tool_correct, p.haste_level),
        (8.0, Some(true), 2)
    );
}

/// A slot the Java side changed (creative inventory pick, menu click): the server never
/// echoes those back, so the game thread applies the mirror's change queue through the
/// same path as `set_player_inventory` — the first-person hand reads the result.
#[test]
fn inventory_slot_from_the_java_mirror_reaches_the_hand() {
    let mut r = Rig::new(GameMode::Creative);
    // Vanilla `Inventory` numbering: hotbar 0, armor 36..=39 (feet..head), off hand 40.
    r.p.set_inventory_slot(0, &item(2)); // diamond_sword
    assert_eq!(r.p.main_hand().item, Some(RegistryIndex(2)));
    // The selected slot's stack change swaps the hand's item (the reported bug).
    r.p.set_inventory_slot(0, &ItemStackData::default());
    assert_eq!(
        r.p.main_hand().item,
        None,
        "a creative pickup clears the slot"
    );
    r.p.set_inventory_slot(2, &item(3)); // stick
    assert_eq!(r.p.int.hotbar[2].item, Some(RegistryIndex(3)));
    r.p.held_slot = 2;
    assert_eq!(r.p.main_hand().item, Some(RegistryIndex(3)));
    // Armor (36 feet, 37 legs, 38 chest, 39 head) and the off hand keep their slots.
    r.p.set_inventory_slot(38, &item(1));
    assert_eq!(r.p.int.armor[1].item, Some(RegistryIndex(1)));
    r.p.set_inventory_slot(40, &item(4));
    assert_eq!(r.p.int.offhand.item, Some(RegistryIndex(4)));
    // Main inventory (9..=35) is held nowhere yet: accepted, nothing panics.
    r.p.set_inventory_slot(20, &item(1));
}

/// The held stack's own `minecraft:tool` component and Efficiency enchantment drive the mining
/// speed (`ItemStack.getDestroySpeed` / `Player.getDestroySpeed`): a stack with a speed-8 tool
/// rule on `mineable/pickaxe` and Efficiency V (level^2 + 1 = 26) gives 8 + 26 = 34.
#[test]
fn held_stack_components_drive_mining_speed() {
    use haste_protocol::wire::{IdSet, Writer};
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    let mut tool = Writer::new();
    tool.var_int(1);
    tool.write(&IdSet::Tag(Identifier::minecraft("mineable/pickaxe")));
    tool.option(&Some(8.0f32), |w, v| w.f32(*v));
    tool.option(&Some(true), |w, v| w.bool(*v));
    tool.f32(1.0);
    tool.var_int(1);
    tool.bool(true);
    let mut patch = Writer::new();
    patch.len_prefix(2);
    patch.len_prefix(0);
    patch.var_int(0); // minecraft:tool
    patch.bytes(&tool.buf);
    patch.var_int(1); // minecraft:enchantments
    patch.var_int(1); // one enchantment
    patch.var_int(0); // minecraft:efficiency
    patch.var_int(5); // level V
    r.p.int.hotbar[0] = ItemStackData {
        count: 1,
        item: Some(RegistryIndex(1)),
        components_raw: patch.into_inner().into(),
    };
    r.tick(InteractInput::default());
    let env = InteractEnv {
        data: data(),
        entities: &[],
        entity_types: &[],
        warm_blocks: None,
    };
    let p = r.p.mining_params(STONE, &env);
    assert_eq!((p.tool_speed, p.tool_correct), (8.0, Some(true)));
    assert_eq!(p.mining_efficiency, 26.0);
    // (8 + 26) / 1.5 / 30 = 0.755 per tick: 2 ticks (wiki: Efficiency V diamond pickaxe).
    assert_eq!(p.ticks_to_break(), Some(2));
}

/// The held item's `minecraft:attack_speed` comes from its attribute modifiers
/// (`Player.getCurrentItemAttackStrengthDelay`): base 4.0, diamond sword -2.4 -> 1.6, diamond
/// pickaxe -2.8 -> 1.2, an item without the modifier keeps the base.
#[test]
fn held_item_attack_speed_comes_from_its_attribute_modifiers() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    assert!((r.p.held_attack_speed() - 4.0).abs() < 1e-6, "bare hand");
    r.p.int.hotbar[0] = item(2); // diamond_sword
    r.tick(InteractInput::default());
    assert!(
        (r.p.held_attack_speed() - 1.6).abs() < 1e-6,
        "diamond sword {}",
        r.p.held_attack_speed()
    );
    r.p.int.hotbar[0] = item(1); // diamond_pickaxe
    r.tick(InteractInput::default());
    assert!(
        (r.p.held_attack_speed() - 1.2).abs() < 1e-6,
        "diamond pickaxe {}",
        r.p.held_attack_speed()
    );
    r.p.int.hotbar[0] = item(3); // stick: no attack_speed modifier
    r.tick(InteractInput::default());
    assert!((r.p.held_attack_speed() - 4.0).abs() < 1e-6, "stick");
}

/// `Item.getUseDuration` / `Item.getUseAnimation`: the consumable component for food and drink,
/// and the item classes that override them (bow/crossbow/trident/spyglass/shield).
#[test]
fn use_duration_and_animation_come_from_the_item() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    let env = InteractEnv {
        data: data(),
        entities: &[],
        entity_types: &[],
        warm_blocks: None,
    };
    let d = |i: u32| r.p.int.use_data(&item(i), env.data);
    // Bow/crossbow/trident/shield: 72000 ticks; spyglass 1200.
    assert_eq!(d(5), Some((72000, UseAnimation::Bow)));
    assert_eq!(d(6), Some((72000, UseAnimation::Crossbow)));
    assert_eq!(d(7), Some((72000, UseAnimation::Trident)));
    assert_eq!(d(9), Some((72000, UseAnimation::Block)));
    assert_eq!(d(8), Some((1200, UseAnimation::Spyglass)));
    // Apple: `consume_seconds` 1.6 -> 32 ticks, `eat`; potion drinks.
    assert_eq!(d(4), Some((32, UseAnimation::Eat)));
    assert_eq!(d(10), Some((32, UseAnimation::Drink)));
    // A tool is not used over time.
    assert_eq!(d(1), None);
    assert_eq!(d(3), None);
}

/// Starting a use sets `useItemRemainingTicks` and stops the 4-tick repeat; releasing the key
/// sends `release_use_item` (`Minecraft.handleKeybinds` / `MultiPlayerGameMode.releaseUsingItem`).
#[test]
fn using_an_item_ticks_down_and_releases_on_key_up() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    r.p.int.hotbar[0] = item(5); // bow
    r.w = TestWorld::new(); // look at nothing: the use_item path
    let use_ = InteractInput {
        use_item: true,
        ..Default::default()
    };
    let out = r.tick(use_);
    assert!(matches!(out[0], ServerboundPlay::UseItem(_)));
    assert!(r.p.is_using_item());
    assert_eq!(r.p.use_duration(), 72000);
    assert_eq!(r.p.use_animation(), UseAnimation::Bow);
    // The start tick already ticked the remaining time down (`LivingEntity.tick`).
    assert_eq!(r.p.use_item_remaining(), 71999);
    // Held: the use does not repeat and only the timer advances.
    let out = r.tick(use_);
    assert!(out.is_empty());
    assert_eq!(r.p.use_item_remaining(), 71998);
    assert_eq!(r.p.ticks_using_item(), 2);
    // Key up: one release, the use ends.
    let out = r.tick(InteractInput::default());
    assert_eq!(actions(&out), [(action::RELEASE_USE_ITEM, 0)]);
    assert!(!r.p.is_using_item());
    // A new press starts a fresh use.
    let out = r.tick(use_);
    assert!(matches!(out[0], ServerboundPlay::UseItem(_)));
    assert_eq!(r.p.use_item_remaining(), 71999);
}

#[test]
fn drop_item_key_sends_status_5_once_per_press() {
    let mut r = Rig::new(GameMode::Survival);
    let q = InteractInput {
        drop_item: true,
        ..Default::default()
    };
    let out = r.tick(q);
    assert_eq!(actions(&out), [(action::DROP_ITEM, 0)]);
    // Held, and on the tick after release: nothing more.
    assert!(r.tick(q).is_empty());
    assert!(r.tick(InteractInput::default()).is_empty());
    let out = r.tick(q);
    assert_eq!(actions(&out), [(action::DROP_ITEM, 0)]);
}

#[test]
fn drop_stack_key_sends_status_4() {
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(InteractInput {
        drop_stack: true,
        ..Default::default()
    });
    assert_eq!(actions(&out), [(action::DROP_ALL_ITEMS, 0)]);
    assert!(r
        .tick(InteractInput {
            drop_stack: true,
            ..Default::default()
        })
        .is_empty());
}

#[test]
fn swap_offhand_key_sends_status_7_with_zeroed_pos() {
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(InteractInput {
        swap_offhand: true,
        ..Default::default()
    });
    // Vanilla zeroes pos, face and sequence for these actions; the server updates the slots.
    let sw: Vec<&sb::PlayerAction> = out
        .iter()
        .filter_map(|p| match p {
            ServerboundPlay::PlayerAction(a) => Some(a),
            _ => None,
        })
        .collect();
    assert_eq!(
        sw,
        [&sb::PlayerAction {
            status: action::SWAP_ITEM_WITH_OFFHAND,
            pos: BlockPos::new(0, 0, 0),
            face: 0,
            sequence: 0,
        }]
    );
    assert!(r
        .tick(InteractInput {
            swap_offhand: true,
            ..Default::default()
        })
        .is_empty());
}

/// The `player_action` statuses this module sends, pinned to the declaration
/// order of `ServerboundPlayerActionPacket$Action` in the 26.3 client jar
/// (CHANGE_DESTROY_DIRECTION was inserted at 1, moving every later value).
#[test]
fn player_action_status_values_match_the_jar() {
    assert_eq!(action::START_DESTROY, 0);
    assert_eq!(action::CHANGE_DESTROY_DIRECTION, 1);
    assert_eq!(action::ABORT_DESTROY, 2);
    assert_eq!(action::STOP_DESTROY, 3);
    assert_eq!(action::DROP_ALL_ITEMS, 4);
    assert_eq!(action::DROP_ITEM, 5);
    assert_eq!(action::RELEASE_USE_ITEM, 6);
    assert_eq!(action::SWAP_ITEM_WITH_OFFHAND, 7);
    assert_eq!(action::STAB, 8);
}

#[test]
fn is_pickable_matches_vanilla_classes() {
    // EntitySelector.CAN_BE_PICKED = Entity::isPickable, per entity class, by type name.
    let pickable = [
        "zombie",
        "creeper",
        "sheep",
        "player",
        "item_frame",
        "glow_item_frame",
        "painting",
        "leash_knot",
        "end_crystal",
        "tnt",
        "falling_block",
        "armor_stand",
        "boat",
        "minecart",
        "chest_minecart",
        "interaction",
        "shulker_bullet",
        "fireball",
        "wind_charge",
        "breeze_wind_charge",
        "guardian",
    ];
    for p in pickable {
        assert!(
            is_pickable(&Identifier::minecraft(p)),
            "{p} should be pickable"
        );
    }
    let not_pickable = [
        "item",
        "experience_orb",
        "arrow",
        "spectral_arrow",
        "trident",
        "ender_dragon",
        "marker",
        "area_effect_cloud",
        "lightning_bolt",
        "eye_of_ender",
        "evoker_fangs",
        "ominous_item_spawner",
        "block_display",
        "item_display",
        "text_display",
        "small_fireball",
        "dragon_fireball",
        "wither_skull",
        "egg",
        "snowball",
        "ender_pearl",
        "splash_potion",
        "lingering_potion",
        "experience_bottle",
        "firework_rocket",
        "llama_spit",
        "fishing_bobber",
    ];
    for p in not_pickable {
        assert!(
            !is_pickable(&Identifier::minecraft(p)),
            "{p} should not be pickable"
        );
    }
    // Unknown (mod) types are pickable, like vanilla's default.
    assert!(is_pickable(&Identifier::new("mymod", "golem")));
    assert!(is_pickable(&Identifier::minecraft("something_new")));
}

/// The kick `multiplayer.disconnect.invalid_entity_attacked` in reverse: an item entity, an
/// experience orb, an arrow, a spectral arrow and an ender dragon in the crosshair are never
/// attack targets — the click falls through to a miss (or the block behind).
#[test]
fn non_pickable_entities_in_the_crosshair_are_never_attacked() {
    for (idx, name) in [
        (ITEM_ENTITY, "item"),
        (EXPERIENCE_ORB, "experience_orb"),
        (ARROW, "arrow"),
        (SPECTRAL_ARROW, "spectral_arrow"),
        (ENDER_DRAGON, "ender_dragon"),
    ] {
        let mut r = Rig::new(GameMode::Survival);
        r.w = TestWorld::new();
        r.add_entity(
            EntityId(42),
            Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
            idx,
        );
        let out = r.tick(ATTACK);
        assert!(
            !out.iter().any(|p| matches!(p, ServerboundPlay::Attack(_))),
            "{name} was attacked"
        );
        assert!(
            matches!(out[0], ServerboundPlay::Punch(_)),
            "{name}: {out:?}"
        );
    }
}

#[test]
fn a_non_pickable_entity_does_not_hide_the_block_behind_it() {
    let mut r = Rig::new(GameMode::Survival);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        ITEM_ENTITY,
    );
    // The stone floor is the target, not the item entity: a start, not an attack.
    let out = r.tick(ATTACK);
    assert!(!out.iter().any(|p| matches!(p, ServerboundPlay::Attack(_))));
    assert_eq!(actions(&out), [(action::START_DESTROY, 1)]);
}

#[test]
fn the_player_itself_is_never_an_attack_target() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    // Our own box around the eye: getEntities(except = us) and the pick filter agree it is
    // never a target.
    r.add_entity(
        ME,
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 1.8),
        PLAYER,
    );
    let out = r.tick(ATTACK);
    assert!(!out.iter().any(|p| matches!(p, ServerboundPlay::Attack(_))));
    assert!(matches!(out[0], ServerboundPlay::Punch(_)));
}

#[test]
fn an_entity_wins_only_when_it_is_strictly_closer_than_the_block() {
    // Box top at y 0.5: 1.12 < 1.62 (the floor's top) — the entity is the target.
    let mut r = Rig::new(GameMode::Survival);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        ZOMBIE,
    );
    let out = r.tick(ATTACK);
    assert!(matches!(
        out[0],
        ServerboundPlay::Attack(sb::Attack { entity_id: 42 })
    ));
    // Box top exactly at the floor's top (y 0, the same 1.62): not strictly closer.
    let mut r = Rig::new(GameMode::Survival);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, -0.5, 0.5), 0.6, 0.5),
        ZOMBIE,
    );
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 1)]);
}

/// `MultiPlayerGameMode.interact` sends the hit location relative to the entity (the server's
/// `interactAt` reads it, e.g. an armor stand's slot from its Y) and the raw shift key
/// (`player.isShiftKeyDown`), not the crouch state.
#[test]
fn the_interact_offset_is_relative_to_the_entity_and_carries_the_shift_key() {
    let mut r = Rig::new(GameMode::Survival);
    // A tall box standing on the floor below our eye: the crosshair hits its top half.
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 1.9),
        ZOMBIE,
    );
    // Not sneaking: shift not held.
    let out = r.tick(USE);
    match &out[0] {
        ServerboundPlay::Interact(i) => {
            assert_eq!(i.entity_id, 42);
            // Feet at (0.5, 0, 0.5); the hit is on the box's centre axis, straight down.
            assert!(
                (i.target_offset.y - 1.62).abs() < 1e-6,
                "{:?}",
                i.target_offset
            );
            assert!((i.target_offset.x - 0.0).abs() < 1e-6);
            assert!((i.target_offset.z - 0.0).abs() < 1e-6);
            assert!(!i.sneaking);
        }
        p => panic!("{p:?}"),
    }
}

/// `Minecraft.startUseItem`'s ENTITY case: a Pass from `Player.interactOn` (food on a zombie)
/// goes on to `useItem` with the same hand, and an empty main hand to the off hand; a pair
/// whose client-side interact consumes (a villager) stops at the entity.
#[test]
fn an_entity_use_falls_through_to_use_item_and_the_off_hand() {
    // Food in the main hand on a zombie: interact, use_item, no off hand (it holds nothing).
    let mut r = hungry(GameMode::Survival);
    r.p.int.hotbar[0] = item(4); // apple
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        ZOMBIE,
    );
    let out = r.tick(USE);
    assert!(
        matches!(&out[0], ServerboundPlay::Interact(i) if i.entity_id == 42 && i.hand == 0),
        "{out:?}"
    );
    assert!(
        matches!(&out[1], ServerboundPlay::UseItem(u) if u.hand == 0),
        "{out:?}"
    );
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(r.p.is_using_item());

    // An empty main hand: its use_item is skipped (`!heldItem.isEmpty()`), the entity
    // interact of the off hand follows, then its apple's use_item.
    let mut r = hungry(GameMode::Survival);
    r.p.int.offhand = item(4);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        ZOMBIE,
    );
    let out = r.tick(USE);
    assert!(
        matches!(&out[0], ServerboundPlay::Interact(i) if i.hand == 0),
        "{out:?}"
    );
    assert!(
        matches!(&out[1], ServerboundPlay::Interact(i) if i.hand == 1),
        "{out:?}"
    );
    assert!(
        matches!(&out[2], ServerboundPlay::UseItem(u) if u.hand == 1),
        "{out:?}"
    );
    assert_eq!(out.len(), 3, "{out:?}");
    assert!(r.p.is_using_item());

    // A pair whose client-side interact consumes (a villager's trade): nothing follows the
    // interact, and the held food never starts a use.
    let mut r = hungry(GameMode::Survival);
    r.p.int.hotbar[0] = item(4);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        VILLAGER,
    );
    let out = r.tick(USE);
    assert!(
        matches!(&out[0], ServerboundPlay::Interact(i) if i.entity_id == 42),
        "{out:?}"
    );
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(!r.p.is_using_item(), "the use stopped at the entity");
}

/// Vanilla's `Minecraft.continueAttack`: while the block is breaking the arm swings every tick,
/// but `LivingEntity.SwingState.startIfAble` refuses a restart less than half way through the
/// 6 tick swing, so the swing runs 1/6, 2/6, 3/6, then starts over.
#[test]
fn a_held_attack_swings_the_hand_every_tick() {
    let mut r = Rig::new(GameMode::Survival);
    assert!(!r.p.hand.is_swinging(), "no swing before the attack");
    // The press tick starts the swing (`Minecraft.startAttack`).
    r.tick(ATTACK);
    assert!(r.p.hand.is_swinging(), "the attack started a swing");
    let mut progress = Vec::new();
    for _ in 0..8 {
        r.tick(ATTACK);
        let item = r.p.main_hand().item;
        r.p.hand.tick(
            0.0,
            (0.0, 90.0),
            true,
            false,
            crate::hand::HeldStack::of(item),
            4.0,
            false,
        );
        progress.push(r.p.hand.attack().1);
    }
    // The restart tick's pair falls from 3/6 to 0, so `SwingState.getAnimation`'s wrap carries
    // the drawn value past 1 (the rest pose); the raw progress restarts at 0.
    for (got, want) in progress.iter().zip([
        0.0,
        1.0 / 6.0,
        2.0 / 6.0,
        3.0 / 6.0,
        1.0,
        1.0 / 6.0,
        2.0 / 6.0,
        3.0 / 6.0,
    ]) {
        assert!((got - want).abs() < 1e-6, "{got} != {want}");
    }
    assert!(r.p.hand.is_swinging(), "still mining, still swinging");
}

/// A right click with an item in hand swings it (`Minecraft.startUseItem` swings the result of
/// `InteractionResult.SUCCESS`) and puts the hand down for the item switch.
#[test]
fn a_use_swings_the_hand_and_drops_it() {
    let mut r = Rig::new(GameMode::Survival);
    let place = InteractInput {
        use_item: true,
        ..InteractInput::default()
    };
    // The hotbar's first slot holds a block to place.
    r.p.int.hotbar[0] = item(1);
    r.tick(place);
    assert!(r.p.hand.is_swinging(), "the use started a swing");
    // `player.itemUsed(hand)` zeroed the equip progress, and the hand started at 0 (vanilla's
    // field default), so the pair for this tick is (0, 0).
    assert_eq!(r.p.hand.equip(), (0.0, 0.0));
    // The held item is new this tick, so the shown item still differs from it: the progress
    // stays 0 and the new item is shown. The tick after brings the hand back up towards
    // ((itemSwapTicker + 1) / (20 / attack_speed))^3 = (2 / 5)^3 = 0.064, under the 0.1 at
    // which the item is switched.
    let item = r.p.main_hand().item;
    r.p.hand.tick(
        0.0,
        (0.0, 90.0),
        true,
        false,
        crate::hand::HeldStack::of(item),
        4.0,
        false,
    );
    assert_eq!(r.p.hand.equip(), (0.0, 0.0));
    assert_eq!(r.p.hand.shown_item(), item);
    r.p.hand.tick(
        0.0,
        (0.0, 90.0),
        true,
        false,
        crate::hand::HeldStack::of(item),
        4.0,
        false,
    );
    assert!(
        (r.p.hand.equip().1 - 0.064).abs() < 1e-6,
        "{}",
        r.p.hand.equip().1
    );
}

const USE: InteractInput = InteractInput {
    attack: false,
    use_item: true,
    pick_block: false,
    drop_item: false,
    drop_stack: false,
    swap_offhand: false,
    hotbar: None,
    attack_clicks: 0,
    use_clicks: 0,
};

/// A hungry survival player with the test item components loaded.
fn hungry(mode: GameMode) -> Rig {
    let mut r = Rig::new(mode);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    r.p.food = 10;
    r
}

fn living_flags(b: i8) -> ClientboundPlay {
    use haste_core::entity::{MetadataEntry, MetadataValue};
    ClientboundPlay::SetEntityData(haste_protocol::packets::play::clientbound::SetEntityData {
        entity_id: ME.0,
        metadata: vec![MetadataEntry {
            index: 8,
            value: MetadataValue::Byte(b),
        }],
    })
}

/// `LivingEntity.isUsingItem` is the synced `DATA_LIVING_ENTITY_FLAGS` bit 0x01: the server
/// clearing it ends the eat, and a held button starts the next bite (`Minecraft.handleKeybinds`)
/// without a `release_use_item`.
#[test]
fn eating_ends_when_the_server_clears_the_using_flag_and_continues_while_held() {
    let mut r = hungry(GameMode::Survival);
    r.p.int.hotbar[0] = item(4); // apple
    r.w = TestWorld::new();
    let out = r.tick(USE);
    assert!(matches!(out[0], ServerboundPlay::UseItem(_)));
    assert!(r.p.is_using_item());
    let mut out = Vec::new();
    // The server confirms: nothing changes.
    r.p.handle(&living_flags(0x01), &mut out);
    assert!(r.p.is_using_item());
    assert_eq!(r.p.use_duration(), 32);
    for _ in 0..5 {
        assert!(r.tick(USE).is_empty());
    }
    // The consume completed: the flag drops, the use ends, the next tick starts a new one.
    r.p.handle(&living_flags(0x00), &mut out);
    assert!(!r.p.is_using_item());
    let out = r.tick(USE);
    assert!(actions(&out).is_empty(), "{out:?}");
    assert!(matches!(out[0], ServerboundPlay::UseItem(_)));
    assert!(r.p.is_using_item());
}

/// A use the server started (`onSyncedDataUpdated` with no local `useItem`) takes the held
/// item's duration.
#[test]
fn a_server_started_use_takes_the_held_items_duration() {
    let mut r = hungry(GameMode::Survival);
    r.p.int.hotbar[0] = item(4);
    r.w = TestWorld::new();
    let mut out = Vec::new();
    r.p.handle(&living_flags(0x01), &mut out);
    assert!(r.p.is_using_item());
    // Held button: the use goes on with the apple's duration (the button up would release
    // it, as vanilla's handleKeybinds does for any use).
    assert!(r.tick(USE).is_empty());
    assert_eq!(r.p.use_duration(), 32);
    assert_eq!(r.p.use_animation(), UseAnimation::Eat);
    assert!(r.p.is_using_item());
}

/// `Minecraft.startUseItem`: a Pass from `useItemOn` (food on stone) goes on to `useItem` with
/// the same hand, and an empty main hand on to the off hand. No `punch` on a use.
#[test]
fn use_on_a_block_falls_through_to_use_item_and_the_off_hand() {
    let mut r = hungry(GameMode::Survival);
    r.p.int.hotbar[0] = item(4);
    let out = r.tick(USE);
    assert!(
        matches!(&out[0], ServerboundPlay::UseItemOn(u) if u.hand == 0),
        "{out:?}"
    );
    assert!(
        matches!(&out[1], ServerboundPlay::UseItem(u) if u.hand == 0),
        "{out:?}"
    );
    assert_eq!(out.len(), 2);
    assert!(r.p.is_using_item());

    let mut r = hungry(GameMode::Survival);
    r.p.int.offhand = item(4);
    let out = r.tick(USE);
    assert!(
        matches!(&out[0], ServerboundPlay::UseItemOn(u) if u.hand == 0),
        "{out:?}"
    );
    assert!(
        matches!(&out[1], ServerboundPlay::UseItemOn(u) if u.hand == 1),
        "{out:?}"
    );
    assert!(
        matches!(&out[2], ServerboundPlay::UseItem(u) if u.hand == 1),
        "{out:?}"
    );
    assert_eq!(out.len(), 3);
}

/// Right click on a plain block with nothing in hand: one `use_item_on` per hand, no swing
/// (vanilla swings only on a Success with a PREDICTED swing source).
#[test]
fn an_empty_hand_on_a_plain_block_does_not_swing() {
    let mut r = Rig::new(GameMode::Survival);
    let out = r.tick(USE);
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(out
        .iter()
        .all(|p| matches!(p, ServerboundPlay::UseItemOn(_))));
    assert!(!r.p.hand.is_swinging());
}

/// `MultiPlayerGameMode.attack` ends with `resetAttackStrengthTicker`; `Minecraft.startAttack`'s
/// MISS case resets it too (in every game mode).
#[test]
fn an_entity_hit_and_a_miss_reset_the_attack_strength() {
    let mut r = Rig::new(GameMode::Survival);
    r.add_entity(
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
        ZOMBIE,
    );
    for _ in 0..40 {
        r.p.hand.tick(
            0.0,
            (0.0, 90.0),
            true,
            false,
            crate::hand::HeldStack::of(None),
            4.0,
            false,
        );
    }
    assert!((r.p.hand.attack_strength_scale(0.5) - 1.0).abs() < 1e-6);
    let out = r.tick(ATTACK);
    assert!(matches!(out[0], ServerboundPlay::Attack(_)));
    assert!(r.p.hand.attack_strength_scale(0.5) < 0.2);

    let mut r = Rig::new(GameMode::Creative);
    r.w = TestWorld::new();
    for _ in 0..40 {
        r.p.hand.tick(
            0.0,
            (0.0, 90.0),
            true,
            false,
            crate::hand::HeldStack::of(None),
            4.0,
            false,
        );
    }
    assert!((r.p.hand.attack_strength_scale(0.5) - 1.0).abs() < 1e-6);
    assert!(matches!(r.tick(ATTACK)[0], ServerboundPlay::Punch(_)));
    assert!(r.p.hand.attack_strength_scale(0.5) < 0.2);
}

/// `Consumable.startConsuming` -> `Player.canEat`: food with a full hunger bar sends the packet
/// but does not start the use.
#[test]
fn food_does_not_start_eating_with_a_full_hunger_bar() {
    let mut r = hungry(GameMode::Survival);
    r.p.food = 20;
    r.p.int.hotbar[0] = item(4);
    r.w = TestWorld::new();
    let out = r.tick(USE);
    assert!(matches!(out[0], ServerboundPlay::UseItem(_)));
    assert!(!r.p.is_using_item());
    // A potion is not food: it always starts.
    r.p.int.hotbar[0] = item(10);
    for _ in 0..4 {
        r.tick(InteractInput::default());
    }
    r.tick(USE);
    assert!(r.p.is_using_item());
    // Invulnerable (creative) eats with a full bar.
    let mut r = hungry(GameMode::Creative);
    r.p.food = 20;
    r.p.abilities.invulnerable = true;
    r.p.int.hotbar[0] = item(4);
    r.w = TestWorld::new();
    r.tick(USE);
    assert!(r.p.is_using_item());
}

/// `MultiPlayerGameMode.sameDestroyTarget` compares the held item and its components, not the
/// count: picking up more of it keeps the break going.
#[test]
fn more_of_the_held_item_does_not_restart_the_break() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(3);
    let out = r.tick(ATTACK);
    assert_eq!(actions(&out), [(action::START_DESTROY, 1)]);
    r.p.int.hotbar[0].count = 5;
    let out = r.tick(ATTACK);
    assert!(actions(&out).is_empty(), "{out:?}");
    // A different item does restart it.
    r.p.int.hotbar[0] = item(1);
    let out = r.tick(ATTACK);
    assert_eq!(
        actions(&out).iter().map(|a| a.0).collect::<Vec<_>>(),
        [action::ABORT_DESTROY, action::START_DESTROY]
    );
}
