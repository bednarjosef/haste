//! Playtest 2026-10-18, click timing and feedback: the held-button packet counts
//! (the "spam feels uncapped" report), and the use results vanilla answers with
//! (`Minecraft.startUseItem`'s swing rules, `BlockItem.place`'s obstruction).
//! The rig lives in `tests.rs`; only the crosshair-look variant of its tick is
//! here (it is fixed at straight down there).

use haste_core::game::GameMode;
use haste_core::input::{InputState, InteractInput};
use haste_physics::testing::TestWorld;
use haste_protocol::ServerboundPlay;

use super::tests::{actions, data, item, Rig, ATTACK, USE};
use super::{action, InteractEnv};

/// `minecraft:dirt` in the test registry (see `tests::data`): a block item.
const DIRT_ITEM: u32 = 12;

/// One rig tick with the crosshair looking somewhere other than straight down
/// (`Rig::tick` is fixed at pitch 90). The rig's teleport left a look offset
/// (the input's angles plus it are the world look), so the input pitch is
/// `pitch` minus it.
fn tick_looking(r: &mut Rig, pitch: f32, i: InteractInput) -> Vec<ServerboundPlay> {
    r.p.set_interact(i);
    let mut out = Vec::new();
    let env = InteractEnv {
        data: data(),
        entities: &r.entities,
        entity_types: &r.types,
        border: r.border.as_ref(),
        warm_blocks: None,
    };
    let input = InputState {
        pitch: pitch - r.p.look_offset().1,
        ..Default::default()
    };
    r.p.tick_interact(&input, &r.w, &env, None, &mut out);
    for (pos, s) in r.p.take_world_edits().collect::<Vec<_>>() {
        r.w.set(pos.x, pos.y, pos.z, s);
    }
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

/// Bug A: holding the use button is capped at one use every 4 ticks
/// (`Minecraft.startUseItem` sets `rightClickDelay` 4, and `Minecraft.tick`
/// counts it down before the key handling): a 40-tick hold with a block item
/// places at ticks 1, 5, 9, ... 37 — 10 `use_item_on` packets, nothing between.
#[test]
fn held_use_places_one_block_every_four_ticks() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(DIRT_ITEM);
    // Pitch 45 looks at the floor's top face at (0, -1, 2) (the ray from the eye
    // at (0.5, 1.62, 0.5) reaches y = 0 at z = 2.12), so the placement position
    // (0, 0, 2) is beside the player, not the block their own box occupies.
    let mut placed_at = Vec::new();
    for t in 1..=40 {
        let out = tick_looking(&mut r, 45.0, USE);
        assert!(
            out.iter()
                .all(|p| matches!(p, ServerboundPlay::UseItemOn(_))),
            "a placement sends only use_item_on: {out:?}"
        );
        if out
            .iter()
            .any(|p| matches!(p, ServerboundPlay::UseItemOn(_)))
        {
            placed_at.push(t);
        }
    }
    assert_eq!(
        placed_at,
        [1, 5, 9, 13, 17, 21, 25, 29, 33, 37],
        "rightClickDelay 4"
    );
}

/// Bug A: holding the attack button in creative is capped by
/// `MultiPlayerGameMode.destroyDelay` 5 (`continueDestroyBlock` counts it down
/// while the button stays down). The sword cannot break in creative, so the
/// crosshair block stays put and the cadence is the packets alone: the press
/// tick breaks, the delay re-arms, and a break follows each time it runs out.
#[test]
fn held_creative_attack_breaks_are_capped_by_destroy_delay() {
    let mut r = Rig::new(GameMode::Creative);
    r.p.int.hotbar[0] = item(2); // diamond sword: no creative breaks, packets only
    let mut starts_at = Vec::new();
    let mut punches = 0;
    for t in 1..=40 {
        let out = r.tick(ATTACK);
        if actions(&out).iter().any(|a| a.0 == action::START_DESTROY) {
            starts_at.push(t);
        }
        punches += out
            .iter()
            .filter(|p| matches!(p, ServerboundPlay::Punch(_)))
            .count();
    }
    // The press tick decrements the delay twice (startDestroyBlock sets it 5,
    // then the same tick's continueAttack counts it down), so the first break
    // is 5 ticks after the press; every later break sets the delay inside
    // continueDestroyBlock itself, which only counts down once per tick: a
    // 6-tick period from then on.
    assert_eq!(starts_at, [1, 6, 12, 18, 24, 30, 36], "destroyDelay 5");
    // startAttack punches on the press, continueAttack on every held tick that
    // keeps breaking (the press tick runs both).
    assert_eq!(punches, 41);
}

/// Bug B: right-clicking into air with an item whose use is a Pass (a dirt
/// block, redstone dust) sends the `use_item` packet — vanilla's
/// `MultiPlayerGameMode.useItem` sends it before the client-side
/// `itemStack.use` — but swings nothing (`Minecraft.startUseItem` swings only
/// a Success with `SwingSource.PREDICTED`), and the use falls through to the
/// other hand.
#[test]
fn an_unusable_item_in_air_does_not_swing_but_sends_use_item() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    r.p.int.hotbar[0] = item(DIRT_ITEM);
    // A stick in the off hand: the Pass falls through to it (`use_item` hand 1).
    r.p.int.offhand = item(3);
    let out = r.tick(USE);
    assert_eq!(
        out.iter()
            .filter(|p| matches!(p, ServerboundPlay::UseItem(_)))
            .count(),
        2,
        "the main hand's Pass falls through to the off hand: {out:?}"
    );
    let hands: Vec<i32> = out
        .iter()
        .filter_map(|p| match p {
            ServerboundPlay::UseItem(u) => Some(u.hand),
            _ => None,
        })
        .collect();
    assert_eq!(hands, [0, 1]);
    assert!(!r.p.hand.is_swinging(), "a Pass never swings");
    assert!(
        !out.iter().any(|p| matches!(p, ServerboundPlay::Punch(_))),
        "the use path sends no punch"
    );
}

/// The use that succeeds client-side swings and drops the hand
/// (`Minecraft.startUseItem` on a Success): placing a block item is one, and
/// `player.itemUsed(hand)` zeroes the equip progress for the item switch.
/// (Moved from `tests.rs`, where the pickaxe-on-stone swing it asserted was
/// the bug: a Pass swings nothing.)
#[test]
fn a_placing_use_swings_the_hand_and_drops_it() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.hotbar[0] = item(DIRT_ITEM);
    // Pitch 45: the placement position (0, 0, 2) is beside the player.
    let out = tick_looking(&mut r, 45.0, USE);
    assert!(matches!(out[0], ServerboundPlay::UseItemOn(_)), "{out:?}");
    assert!(r.p.hand.is_swinging(), "the use started a swing");
    // `player.itemUsed(hand)` zeroed the equip progress, and the hand started at 0
    // (vanilla's field default), so the pair for this tick is (0, 0).
    assert_eq!(r.p.hand.equip(), (0.0, 0.0));
    // The held item is new this tick, so the shown item still differs from it: the
    // progress stays 0 and the new item is shown. The tick after brings the hand
    // back up towards ((itemSwapTicker + 1) / (20 / attack_speed))^3 = (2 / 5)^3
    // = 0.064, under the 0.1 at which the item is switched.
    let item = r.p.main_hand().item;
    r.p.hand.tick(
        0.0,
        (0.0, 90.0),
        true,
        false,
        crate::hand::HeldStack::of(item),
        4.0,
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
    );
    assert!(
        (r.p.hand.equip().1 - 0.064).abs() < 1e-6,
        "{}",
        r.p.hand.equip().1
    );
}

/// `WritableBookItem.use` is a SUCCESS: the use swings, opens the book editor
/// and stops there — the other hand is never tried.
#[test]
fn a_writable_book_use_swings_and_opens_the_editor() {
    let mut r = Rig::new(GameMode::Survival);
    r.w = TestWorld::new();
    r.p.int.hotbar[0] = item(13); // writable_book
    r.p.int.offhand = item(3); // a stick: the Success must stop before it
    let out = r.tick(USE);
    assert_eq!(
        out.iter()
            .filter(|p| matches!(p, ServerboundPlay::UseItem(_)))
            .count(),
        1,
        "the book's Success stops the use: {out:?}"
    );
    assert!(r.p.hand.is_swinging(), "the use started a swing");
    assert_eq!(r.p.take_used_screen(), Some(super::UsedScreen::ItemGui(0)));
}
