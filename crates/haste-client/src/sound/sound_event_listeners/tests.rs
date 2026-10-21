//! Tests of the hand-over of played sounds to the Java listeners
//! (`crate::sound::sound_event_listeners`).

use std::sync::Arc;

use glam::DVec3;

use super::*;

#[test]
fn a_played_sound_keeps_its_caption_position_and_range_with_vanillas_source_ordinal() {
    let played = PlayedSound {
        event: Arc::from("minecraft:entity.chicken.ambient"),
        subtitle: Arc::from("subtitles.entity.chicken.ambient"),
        source: SoundSource::Neutral,
        pos: DVec3::new(10.5, 64.0, -3.5),
        range: 16.0,
        volume: 1.0,
        pitch: 1.1,
        looping: false,
        relative: false,
    };
    let java = java_sound(played);
    assert_eq!(&*java.subtitle, "subtitles.entity.chicken.ambient");
    assert_eq!(&*java.event, "minecraft:entity.chicken.ambient");
    assert_eq!(java.pos, [10.5, 64.0, -3.5]);
    assert_eq!(java.range, 16.0);
    assert_eq!(java.pitch, 1.1);
    assert!(!java.looping && !java.relative);
    // `SoundSource.NEUTRAL` is ordinal 6 in vanilla's enum.
    assert_eq!(java.source, 6);
    assert_eq!(source_ordinal(SoundSource::Master), 0);
    assert_eq!(source_ordinal(SoundSource::Ui), 10);
}

#[test]
fn the_listener_is_the_sound_systems_camera_transform() {
    // Facing south (yaw 0), level: forward +Z, up +Y; the overlay's `right()` is
    // `forward x up` = -X, as vanilla's camera gives.
    let l = ListenerTransform::from_look(DVec3::new(1.0, 65.62, 2.0), 0.0, 0.0);
    let java = java_listener(&l);
    assert_eq!(java.position, [1.0, 65.62, 2.0]);
    assert!((java.forward[2] - 1.0).abs() < 1e-12 && java.forward[0].abs() < 1e-12);
    assert!((java.up[1] - 1.0).abs() < 1e-12);
}
