//! Item cooldown tests: vanilla `ItemCooldowns`'s percent math and expiry, the `cooldown`
//! packet (`ClientPacketListener.handleItemCooldown`), the group resolution
//! (`ItemCooldowns.getCooldownGroup`) and the use gating of
//! `MultiPlayerGameMode.performUseItemOn`/`useItem`. The rig and fixtures are
//! [`super::tests`]'s.

use std::sync::Arc;

use haste_core::game::GameMode;
use haste_core::input::{InputState, InteractInput};
use haste_core::{BlockPos, BlockStateId, Identifier};
use haste_protocol::packets::play::clientbound::Cooldown;
use haste_protocol::ClientboundPlay;
use haste_protocol::ServerboundPlay;

use super::cooldowns::ItemCooldowns;
use super::tests::{data, item, Rig, USE};
use crate::player::interact::InteractEnv;
use haste_core::view::BlockView;

/// `minecraft:apple`'s registry index in the test registries (see `tests::data`): a food item
/// whose use starts eating.
const APPLE: u32 = 4;
/// `minecraft:shield`'s: an item the test components give a `use_cooldown` group.
const SHIELD: u32 = 9;
/// `minecraft:dirt`'s: a block item, for the placement prediction's gate.
const DIRT_ITEM: u32 = 12;

/// Item default components for the tests (the apple's food and consumable, the shield's
/// cooldown group), so they do not need the data generator cache.
fn components() -> Arc<haste_data::ItemComponents> {
    let mut c = haste_data::ItemComponents::default();
    c.insert_consumable(
        Identifier::minecraft("apple"),
        haste_data::Consumable::default_food(),
    );
    c.insert_food(Identifier::minecraft("apple"), false);
    c.insert_use_cooldown(
        Identifier::minecraft("shield"),
        Identifier::minecraft("shield"),
    );
    Arc::new(c)
}

/// A use of `ticks` on `group`, the way the server's `cooldown` packet brings it
/// (`ClientPacketListener.handleItemCooldown`).
fn cooldown(r: &mut Rig, group: &str, ticks: i32) {
    r.p.handle(
        &ClientboundPlay::Cooldown(Cooldown {
            group: Identifier::minecraft(group),
            ticks,
        }),
        &mut Vec::new(),
    );
}

#[test]
fn percent_and_expiry_follow_vanilla() {
    let mut c = ItemCooldowns::default();
    let pearl = Identifier::minecraft("ender_pearl");
    // A group without a cooldown and an unknown group are both 0.
    assert_eq!(c.percent(&pearl, 0.5), 0.0);
    // Vanilla `addCooldown` at counter 0: start 0, end 20 (an ender pearl's ticks).
    c.add(pearl.clone(), 20);
    assert!((c.percent(&pearl, 0.0) - 1.0).abs() < 1e-6);
    // Four ticks later, mid-tick: remaining (20 - 4.5) / 20.
    for _ in 0..4 {
        c.tick();
    }
    assert!((c.percent(&pearl, 0.5) - 0.775).abs() < 1e-6);
    // `addCooldown` restarts the group's window from now (counter 4).
    c.add(pearl.clone(), 5);
    assert!((c.percent(&pearl, 0.0) - 1.0).abs() < 1e-6);
    // After the fifth tick the counter is 9 = the end: dropped by `tick`
    // (`endTime <= tickCount`), like the two before it kept the window.
    for _ in 0..5 {
        c.tick();
    }
    assert_eq!(c.percent(&pearl, 0.0), 0.0);
    // `removeCooldown` (the packet's zero duration).
    c.add(pearl.clone(), 20);
    c.remove(&pearl);
    assert_eq!(c.percent(&pearl, 0.0), 0.0);
}

#[test]
fn the_cooldown_packet_starts_and_clears_a_group() {
    let mut r = Rig::new(GameMode::Survival);
    let apple = Identifier::minecraft("apple");
    cooldown(&mut r, "apple", 40);
    assert!(r.p.int.cooldowns.percent(&apple, 0.0) > 0.99);
    // The zero duration clears it (`handleItemCooldown`).
    cooldown(&mut r, "apple", 0);
    assert_eq!(r.p.int.cooldowns.percent(&apple, 0.0), 0.0);
}

/// `ItemCooldowns.getCooldownGroup`: the `use_cooldown` component's group when it names one,
/// else the item's registry id (vanilla 26.3 registers every cooldown with seconds only, so
/// the id is the usual group — the shield here stands in for a mod's grouped item).
#[test]
fn the_group_is_the_component_group_or_the_item_id() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.set_item_components(components());
    assert_eq!(
        r.p.int.cooldown_group(&item(SHIELD), data()),
        Some(Identifier::minecraft("shield"))
    );
    assert_eq!(
        r.p.int.cooldown_group(&item(APPLE), data()),
        Some(Identifier::minecraft("apple"))
    );
}

/// `MultiPlayerGameMode.useItem` on a cooldown: the packet is still sent (vanilla returns it
/// from the prediction and `startPrediction` sends it), but `itemStack.use` is skipped — no
/// use starts client side — and the Pass falls through to the off hand.
#[test]
fn a_cooldown_gates_the_clientside_use_but_not_the_packet() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(components());
    r.p.food = 10; // hungry, so the apple can be eaten
    r.p.int.hotbar[0] = item(APPLE);
    r.p.int.offhand = item(APPLE);
    // The press starts the use (`LivingEntity.startUsingItem`).
    r.tick(USE);
    assert!(r.p.is_using_item());
    cooldown(&mut r, "apple", 40);
    r.p.int.set_using_item(false);
    // The use repeat's delay (rightClickDelay 4) runs out on the fourth tick after the
    // press; still on cooldown, the use is a client-side Pass but the packet goes out for
    // both hands.
    for _ in 0..3 {
        r.tick(USE);
    }
    let out = r.tick(USE);
    let hands: Vec<i32> = out
        .iter()
        .filter_map(|p| match p {
            ServerboundPlay::UseItem(u) => Some(u.hand),
            _ => None,
        })
        .collect();
    assert_eq!(hands, [0, 1], "both hands' packets went out: {out:?}");
    assert!(!r.p.is_using_item(), "no use started under the cooldown");
    assert!(!r.p.hand.is_swinging(), "a Pass swings nothing");
    // Once the cooldown is over (40 ticks), the same use eats again.
    for _ in 0..40 {
        r.tick(USE);
    }
    cooldown(&mut r, "apple", 0);
    for _ in 0..3 {
        r.tick(USE);
    }
    r.tick(USE);
    assert!(r.p.is_using_item(), "the use restarted after the cooldown");
}

/// `performUseItemOn`'s item half: with the held block item on cooldown the placement
/// prediction is skipped (no swing, no local block) while the `use_item_on` packet still goes
/// out; without the cooldown the placement happens as usual.
#[test]
fn a_cooldown_gates_the_placement_prediction_but_not_the_packet() {
    // Without a cooldown: the placement predicts, swings the hand and edits the world.
    let mut plain = Rig::new(GameMode::Survival);
    plain.p.int.hotbar[0] = item(DIRT_ITEM);
    let out = tick_looking(&mut plain, 45.0, USE);
    assert!(out
        .iter()
        .any(|p| matches!(p, ServerboundPlay::UseItemOn(_))));
    assert!(plain.p.hand.is_swinging());
    assert_ne!(
        plain.w.block_state(BlockPos::new(0, 0, 2)),
        BlockStateId::AIR
    );
    // With the cooldown: the packet goes out, nothing happens client-side.
    let mut gated = Rig::new(GameMode::Survival);
    gated.p.int.hotbar[0] = item(DIRT_ITEM);
    cooldown(&mut gated, "dirt", 40);
    let out = tick_looking(&mut gated, 45.0, USE);
    assert!(
        out.iter()
            .any(|p| matches!(p, ServerboundPlay::UseItemOn(_))),
        "the packet went out inside startPrediction: {out:?}"
    );
    assert!(!gated.p.hand.is_swinging(), "a Pass swings nothing");
    assert_eq!(
        gated.w.block_state(BlockPos::new(0, 0, 2)),
        BlockStateId::AIR,
        "no placement was predicted"
    );
}

/// One rig tick with the crosshair looking somewhere other than straight down (`Rig::tick`
/// is fixed at pitch 90), like `click_results::tick_looking`.
fn tick_looking(r: &mut Rig, pitch: f32, i: InteractInput) -> Vec<ServerboundPlay> {
    r.p.set_interact(i);
    let mut out = Vec::new();
    let env = InteractEnv {
        data: data(),
        entities: &r.entities,
        entity_types: &r.types,
        border: r.border.as_ref(),
        warm_blocks: None,
        ui: None,
    };
    let input = InputState {
        pitch: pitch - r.p.look_offset().1,
        ..Default::default()
    };
    r.p.tick_interact(&input, &r.w, &env, None, &mut out);
    for (pos, s) in r.p.take_world_edits() {
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
