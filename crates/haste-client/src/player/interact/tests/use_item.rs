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
