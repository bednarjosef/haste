use std::sync::{Arc, OnceLock};

use haste_core::input::InputState;
use haste_core::RegistryIndex;
use haste_physics::mining::{BlockTags, Tool, ToolTable};
use haste_physics::testing::TestWorld;

use super::*;

const ME: EntityId = EntityId(7);
const STONE: BlockStateId = BlockStateId(1);
const DIRT: BlockStateId = BlockStateId(2);
const BEDROCK: BlockStateId = BlockStateId(3);
const GRASS: BlockStateId = BlockStateId(4);

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
                    "minecraft:enchantment": {{"entries": {{{}}}}}}}"#,
                entries(&names),
                entries(&["air", "diamond_pickaxe", "diamond_sword", "stick"]),
                entries(&["speed", "haste", "mining_fatigue", "conduit_power"]),
                entries(&["tool", "enchantments"]),
                entries(&["efficiency", "sharpness"]),
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

struct Rig {
    p: LocalPlayer,
    w: TestWorld,
    entities: Vec<(EntityId, Aabb)>,
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
        };
        r.tick(InteractInput::default());
        r.tick(InteractInput::default());
        r.tick(InteractInput::default());
        r
    }

    fn tick(&mut self, i: InteractInput) -> Vec<ServerboundPlay> {
        self.p.set_interact(i);
        let mut out = Vec::new();
        let env = InteractEnv {
            data: data(),
            entities: &self.entities,
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
    for t in 1..=12 {
        let out = r.tick(ATTACK);
        let a = actions(&out);
        assert!(a.iter().all(|a| a.0 == action::START_DESTROY));
        if !a.is_empty() {
            broke_at.push(t);
        }
    }
    // The press tick breaks and its own continue step already counts the delay down once.
    assert_eq!(broke_at, [1, 6, 12]);
    let mut s = Rig::new(GameMode::Creative);
    s.p.int.hotbar[0] = item(2);
    for _ in 0..10 {
        assert!(actions(&s.tick(ATTACK)).is_empty());
    }
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
    assert!(matches!(out[1], ServerboundPlay::Punch(_)));
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
fn attack_entity_pick_and_hotbar() {
    let mut r = Rig::new(GameMode::Survival);
    // A mob standing on the block we look at, between us and it.
    r.entities.push((
        EntityId(42),
        Aabb::entity(DVec3::new(0.5, 0.0, 0.5), 0.6, 0.5),
    ));
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
    };
    let p = r.p.mining_params(STONE, &env);
    assert_eq!(
        (p.tool_speed, p.tool_correct, p.haste_level),
        (8.0, Some(true), 2)
    );
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

#[test]
fn drop_item_key_sends_status_5_once_per_press() {    let mut r = Rig::new(GameMode::Survival);
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
        r.p.hand.tick(0.0, (0.0, 90.0), true, false, item, 4.0);
        progress.push(r.p.hand.attack().1);
    }
    for (got, want) in progress.iter().zip([
        0.0,
        1.0 / 6.0,
        2.0 / 6.0,
        3.0 / 6.0,
        0.0,
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
    r.p.hand.tick(0.0, (0.0, 90.0), true, false, item, 4.0);
    assert_eq!(r.p.hand.equip(), (0.0, 0.0));
    assert_eq!(r.p.hand.shown_item(), item);
    r.p.hand.tick(0.0, (0.0, 90.0), true, false, item, 4.0);
    assert!(
        (r.p.hand.equip().1 - 0.064).abs() < 1e-6,
        "{}",
        r.p.hand.equip().1
    );
}
