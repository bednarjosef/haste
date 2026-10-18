//! Using an item: the use ticking down and releasing (`LivingEntity.updateUsingItem`), and a
//! consumable's sound while it is used (`Consumable.emitParticlesAndSounds`).

use super::*;

/// While a consumable is used the client plays its own sound every vanilla interval
/// (`Consumable.shouldEmitParticlesAndSounds` / `emitParticlesAndSounds` via
/// `LivingEntity.updateUsingItem`): six eat sounds over one bread's 32-tick use.
#[test]
fn a_consumable_plays_its_sound_every_vanilla_interval() {
    use crate::sound::{SoundRef, SoundRequest};
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    r.p.food = 10; // hungry, so `can_consume` allows eating
    r.p.int.hotbar[0] = item(4); // apple: eat, 32 ticks
    r.w = TestWorld::new();
    let use_ = InteractInput {
        use_item: true,
        ..Default::default()
    };
    r.tick(use_); // the press starts the use
    for _ in 0..31 {
        r.tick(use_);
    }
    let reqs: Vec<SoundRequest> = r.p.take_own_sound_requests().collect();
    let events: Vec<String> = reqs
        .iter()
        .map(|r| match r {
            SoundRequest::Play(p) => match &p.sound {
                SoundRef::Inline(name) => name.clone(),
                other => panic!("unexpected sound {other:?}"),
            },
            other => panic!("unexpected request {other:?}"),
        })
        .collect();
    assert_eq!(
        events,
        vec!["minecraft:entity.generic.eat"; 6],
        "remaining 24, 20, 16, 12, 8, 4"
    );
    // The volume and pitch are the vanilla ranges.
    for r in &reqs {
        let SoundRequest::Play(p) = r else { panic!() };
        assert!(p.volume == 0.5 || p.volume == 1.0, "volume {}", p.volume);
        assert!((0.8..=1.2).contains(&p.pitch), "pitch {}", p.pitch);
        assert_eq!(p.category, 7, "SoundSource.PLAYERS");
    }
}

/// While a consumable is used its item particles spawn with the sound
/// (`Consumable.hasConsumeParticles` -> `LivingEntity.spawnItemParticles(stack, 5)` from
/// `onUseTick`): five crumbs per emit, six emits over one bread use. The Rig looks
/// straight down (pitch 90), so vanilla's 0.6 offset in front of the mouth points below
/// the eye, and the y part of the position offset becomes the z.
#[test]
fn a_consumable_spawns_its_item_particles_with_the_sound() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    r.p.food = 10;
    r.p.int.hotbar[0] = item(4); // apple: eat, 32 ticks, particles on
    r.w = TestWorld::new();
    let use_ = InteractInput {
        use_item: true,
        ..Default::default()
    };
    r.tick(use_);
    for _ in 0..31 {
        r.tick(use_);
    }
    let crumbs: Vec<_> = r.p.take_own_item_particles().collect();
    assert_eq!(crumbs.len(), 30, "six emits x five crumbs");
    // The rig's world is empty, so the player falls while eating; each emit's five crumbs
    // share that tick's eye height (pitch 90 rotates the 0.6 offset straight down).
    for group in crumbs.chunks(5) {
        let y = group[0].pos.y;
        assert!(
            group.iter().all(|c| (c.pos.y - y).abs() < 1e-6),
            "one eye per emit"
        );
    }
    for c in &crumbs {
        assert_eq!(c.item, RegistryIndex(4), "the used stack's item");
        // Pitch 90 rotates the offsets: the y offset (-0.9..-0.3) lands on z (the rig's
        // player sits at z 0.5), x stays horizontal within +-0.15.
        assert!((-0.9..=-0.3).contains(&(c.pos.z - 0.5)), "z {:?}", c.pos);
        assert!((c.pos.x - 0.5).abs() <= 0.15 + 1e-6, "x {:?}", c.pos);
        // The velocity `d` rotates the same way (its y part becomes z); the +0.05 lift
        // is added after the rotation, on y.
        assert!((c.vel[1] - 0.05).abs() < 1e-6, "vy {:?}", c.vel);
        assert!((0.1..=0.2).contains(&c.vel[2]), "vz {:?}", c.vel);
        assert!(c.vel[0].abs() <= 0.05 + 1e-6, "vx {:?}", c.vel);
    }
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

/// A stack whose component patch adds a `minecraft:consumable` (a command-given or modded
/// item): the patch's duration, animation, sound and particles override the item default
/// (`ItemStack.get(DataComponents.CONSUMABLE)`), even on an item that is not food.
fn patched_stack(item: u32, component_ty: i32, body: &[u8]) -> ItemStackData {
    let mut w = haste_protocol::wire::Writer::new();
    w.len_prefix(1);
    w.len_prefix(0);
    w.var_int(component_ty);
    w.bytes(body);
    ItemStackData {
        count: 1,
        item: Some(RegistryIndex(item)),
        components_raw: bytes::Bytes::from(w.into_inner()),
    }
}

/// A `minecraft:consumable` body as vanilla's `Consumable.STREAM_CODEC` writes it.
fn consumable_body(
    seconds: f32,
    animation: i32,
    sound: &haste_protocol::wire::IdOr<haste_protocol::wire::SoundEvent>,
    particles: bool,
) -> Vec<u8> {
    let mut w = haste_protocol::wire::Writer::new();
    w.f32(seconds);
    w.var_int(animation);
    w.write(sound);
    w.bool(particles);
    w.len_prefix(0); // no on_consume_effects
    w.into_inner()
}

/// The `minecraft:data_component_type` id of `minecraft:consumable` in the test registries
/// (the entries list in `data()`).
const CONSUMABLE: i32 = 4;

/// Holding use on a stick whose patch adds a consumable runs the eat use for the patched
/// duration and animation, with the patched sound and particles
/// (`Consumable.startConsuming`, `ItemStack.onUseTick`).
#[test]
fn a_patched_consumable_overrides_the_item_default() {
    use crate::sound::{SoundRef, SoundRequest};
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    // A stick (not food, no default consumable) that drinks for half a second.
    let sound = haste_protocol::wire::IdOr::Inline(haste_protocol::wire::SoundEvent {
        name: Identifier::minecraft("entity.generic.drink"),
        fixed_range: None,
    });
    let body = consumable_body(0.5, 2, &sound, true);
    r.p.int.hotbar[0] = patched_stack(3, CONSUMABLE, &body); // stick
    r.w = TestWorld::new();
    let use_ = InteractInput {
        use_item: true,
        ..Default::default()
    };
    r.tick(use_); // the press starts the use
    assert!(r.p.is_using_item());
    assert_eq!(r.p.use_duration(), 10, "0.5 s");
    assert_eq!(r.p.use_animation(), UseAnimation::Drink);
    // The per-tick effects carry the patched sound: one emit in a 10-tick use (past
    // 21.875% and every fourth remaining tick: remaining 4).
    for _ in 0..9 {
        r.tick(use_);
    }
    let reqs: Vec<SoundRequest> = r.p.take_own_sound_requests().collect();
    let names: Vec<&String> = reqs
        .iter()
        .map(|req| match req {
            SoundRequest::Play(p) => match &p.sound {
                SoundRef::Inline(name) => name,
                other => panic!("unexpected sound {other:?}"),
            },
            other => panic!("unexpected request {other:?}"),
        })
        .collect();
    assert_eq!(names, ["minecraft:entity.generic.drink"]);
    // `has_consume_particles` from the patch: five crumbs with the one emit.
    let crumbs: Vec<_> = r.p.take_own_item_particles().collect();
    assert_eq!(crumbs.len(), 5);
    assert_eq!(crumbs[0].item, RegistryIndex(3), "the stick's crumbs");
}

/// The stack's patch decides the consumable even against the item's default: a registry
/// sound holder resolves through `minecraft:sound_event`, an inline event keeps its own
/// name, and a stack without a patch falls back to the item default.
#[test]
fn stack_consumable_reads_patch_sound_and_default() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    // A registry sound holder: id 0 is entity.generic.eat in the test registries.
    let registry = haste_protocol::wire::IdOr::Id(0);
    let body = consumable_body(1.6, 1, &registry, true);
    let stick = patched_stack(3, CONSUMABLE, &body);
    let c = r.p.int.stack_consumable(&stick, data()).unwrap();
    assert_eq!(c.sound, Identifier::minecraft("entity.generic.eat"));
    assert_eq!(c.consume_ticks(), 32);
    // An inline event: its own name, whatever the registries know.
    let inline = haste_protocol::wire::IdOr::Inline(haste_protocol::wire::SoundEvent {
        name: Identifier::minecraft("mob.goat.cheer"),
        fixed_range: Some(64.0),
    });
    let body = consumable_body(1.6, 1, &inline, false);
    let stick = patched_stack(3, CONSUMABLE, &body);
    let c = r.p.int.stack_consumable(&stick, data()).unwrap();
    assert_eq!(c.sound, Identifier::minecraft("mob.goat.cheer"));
    assert!(!c.has_consume_particles);
    // No patch: the item's default (an apple eats).
    let c = r.p.int.stack_consumable(&item(4), data()).unwrap();
    assert_eq!(c, haste_data::Consumable::default_food());
}

/// A patch that removes `minecraft:consumable` makes the stack not consumable, even though
/// the item's default has one (an apple): the use never starts.
#[test]
fn a_removed_consumable_stops_the_use() {
    let mut r = Rig::new(GameMode::Survival);
    r.p.int.prepare(data());
    r.p.int.set_item_components(item_components());
    r.p.food = 10; // hungry, so eating would be allowed
    let mut w = haste_protocol::wire::Writer::new();
    w.len_prefix(0);
    w.len_prefix(1);
    w.var_int(CONSUMABLE);
    r.p.int.hotbar[0] = ItemStackData {
        count: 1,
        item: Some(RegistryIndex(4)), // apple
        components_raw: bytes::Bytes::from(w.into_inner()),
    };
    r.w = TestWorld::new();
    let out = r.tick(InteractInput {
        use_item: true,
        ..Default::default()
    });
    // The use_item packet went out; the client-side use did not start.
    assert!(matches!(out[0], ServerboundPlay::UseItem(_)));
    assert!(!r.p.is_using_item());
}
