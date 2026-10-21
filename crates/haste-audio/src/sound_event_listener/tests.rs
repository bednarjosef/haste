//! Tests of the sound event listeners' record (`crate::sound_event_listener`).

use super::*;

fn events() -> SoundEvents {
    SoundEvents::parse(
        br#"{
            "entity.chicken.ambient": {"subtitle": "subtitles.entity.chicken.ambient", "sounds": ["mob/chicken/say1"]},
            "block.stone.step": {"sounds": ["step/stone1"]}
        }"#,
    )
    .unwrap()
}

fn variant(volume: f32, attenuation_distance: u32) -> SoundVariant {
    SoundVariant {
        id: haste_core::Identifier::parse("minecraft:mob/chicken/say1").unwrap(),
        path: "minecraft/sounds/mob/chicken/say1.ogg".into(),
        volume,
        pitch: 1.0,
        weight: 1,
        stream: false,
        attenuation_distance,
        preload: false,
    }
}

fn chicken() -> Arc<str> {
    Arc::from("minecraft:entity.chicken.ambient")
}

#[test]
fn nothing_is_recorded_while_nobody_listens() {
    let mut listeners = SoundEventListeners::default();
    let pos = Some(DVec3::new(1.0, 2.0, 3.0));
    let v = variant(1.0, 16);
    listeners.notify(
        &events(),
        &chicken(),
        &v,
        SoundSource::Neutral,
        pos,
        1.0,
        1.0,
        false,
    );
    assert!(!listeners.is_listening());
    assert_eq!(listeners.drain().count(), 0);
}

#[test]
fn a_positioned_sound_records_its_subtitle_position_and_range() {
    let mut listeners = SoundEventListeners::default();
    listeners.set_listening(true);
    let pos = Some(DVec3::new(1.0, 2.0, 3.0));
    let v = variant(1.0, 16);
    listeners.notify(
        &events(),
        &chicken(),
        &v,
        SoundSource::Neutral,
        pos,
        0.5,
        1.2,
        false,
    );
    let played: Vec<PlayedSound> = listeners.drain().collect();
    assert_eq!(
        played,
        vec![PlayedSound {
            event: chicken(),
            subtitle: Arc::from("subtitles.entity.chicken.ambient"),
            source: SoundSource::Neutral,
            pos: DVec3::new(1.0, 2.0, 3.0),
            // `Sound.getAttenuationDistance(0.5)`: a quiet sound still carries its full
            // 16 blocks (`max(volume, 1)`).
            range: 16.0,
            volume: 0.5,
            pitch: 1.2,
            looping: false,
            relative: false,
        }]
    );
    // Drained: the next drain is empty.
    assert_eq!(listeners.drain().count(), 0);
}

#[test]
fn a_loud_sound_carries_further() {
    // The instance volume (play volume times the variant's) above 1 scales the range.
    let mut listeners = SoundEventListeners::default();
    listeners.set_listening(true);
    let v = variant(2.0, 16);
    let pos = Some(DVec3::ZERO);
    listeners.notify(
        &events(),
        &chicken(),
        &v,
        SoundSource::Neutral,
        pos,
        1.5,
        1.0,
        true,
    );
    let played = listeners.drain().next().unwrap();
    assert_eq!(played.range, 48.0);
    assert!(played.looping);
    assert_eq!(attenuation_distance(0.2, 16), 16.0);
    assert_eq!(attenuation_distance(3.0, 8), 24.0);
}

#[test]
fn a_relative_sound_is_heard_everywhere_at_the_origin() {
    let mut listeners = SoundEventListeners::default();
    listeners.set_listening(true);
    let v = variant(1.0, 16);
    listeners.notify(
        &events(),
        &chicken(),
        &v,
        SoundSource::Ui,
        None,
        1.0,
        1.0,
        false,
    );
    let played = listeners.drain().next().unwrap();
    assert!(played.relative);
    assert_eq!(played.pos, DVec3::ZERO);
    assert_eq!(played.range, f32::INFINITY);
    assert_eq!(range(true, 16.0), f32::INFINITY);
    assert_eq!(range(false, 16.0), 16.0);
}

#[test]
fn sounds_without_a_subtitle_or_unknown_events_are_not_recorded() {
    let mut listeners = SoundEventListeners::default();
    listeners.set_listening(true);
    let v = variant(1.0, 16);
    let pos = Some(DVec3::ZERO);
    let step: Arc<str> = Arc::from("minecraft:block.stone.step");
    listeners.notify(
        &events(),
        &step,
        &v,
        SoundSource::Block,
        pos,
        1.0,
        1.0,
        false,
    );
    let unknown: Arc<str> = Arc::from("mod:unknown");
    listeners.notify(
        &events(),
        &unknown,
        &v,
        SoundSource::Block,
        pos,
        1.0,
        1.0,
        false,
    );
    assert_eq!(listeners.drain().count(), 0);
}

#[test]
fn an_event_without_a_namespace_finds_its_subtitle() {
    let mut listeners = SoundEventListeners::default();
    listeners.set_listening(true);
    let v = variant(1.0, 16);
    let bare: Arc<str> = Arc::from("entity.chicken.ambient");
    listeners.notify(
        &events(),
        &bare,
        &v,
        SoundSource::Neutral,
        None,
        1.0,
        1.0,
        false,
    );
    let played = listeners.drain().next().unwrap();
    assert_eq!(&*played.subtitle, "subtitles.entity.chicken.ambient");
    assert_eq!(&*played.event, "entity.chicken.ambient");
}

#[test]
fn stopping_to_listen_forgets_the_undrained_sounds() {
    let mut listeners = SoundEventListeners::default();
    listeners.set_listening(true);
    let v = variant(1.0, 16);
    listeners.notify(
        &events(),
        &chicken(),
        &v,
        SoundSource::Neutral,
        None,
        1.0,
        1.0,
        false,
    );
    listeners.set_listening(false);
    listeners.set_listening(true);
    assert_eq!(listeners.drain().count(), 0);
}
