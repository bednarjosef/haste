//! Tests of the queue between the sound system and the Java sound event listeners
//! (`crate::sound_manager`).

use super::*;

fn chicken(x: f64) -> PlayedSound {
    PlayedSound {
        event: Arc::from("minecraft:entity.chicken.ambient"),
        subtitle: Arc::from("subtitles.entity.chicken.ambient"),
        source: 6,
        pos: [x, 64.0, -3.5],
        range: 16.0,
        volume: 1.0,
        pitch: 1.25,
        looping: false,
        relative: false,
    }
}

fn listener() -> ListenerTransform {
    ListenerTransform {
        position: [1.0, 65.62, 2.0],
        forward: [0.0, 0.0, 1.0],
        up: [0.0, 1.0, 0.0],
    }
}

#[test]
fn nothing_is_queued_while_java_does_not_listen() {
    let mut q = SoundEventQueue::default();
    q.publish(listener(), [chicken(0.0)]);
    let (mut names, mut values) = (Vec::new(), Vec::new());
    // The listener is kept even so: it is what the overlay reads once it listens.
    assert_eq!(q.take(&mut names, &mut values), listener().to_array());
    assert!(names.is_empty() && values.is_empty());
}

#[test]
fn a_drain_encodes_the_sounds_oldest_first_and_empties_the_queue() {
    let mut q = SoundEventQueue::default();
    q.set_listening(true);
    let mut relative = chicken(0.0);
    relative.relative = true;
    relative.looping = true;
    relative.range = f32::INFINITY;
    q.publish(listener(), [chicken(10.5), relative]);
    let (mut names, mut values) = (Vec::new(), Vec::new());
    let l = q.take(&mut names, &mut values);
    assert_eq!(l, [1.0, 65.62, 2.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
    assert_eq!(names.len(), 4);
    assert_eq!(&*names[0], "minecraft:entity.chicken.ambient");
    assert_eq!(&*names[1], "subtitles.entity.chicken.ambient");
    assert_eq!(values.len(), 2 * VALUES_PER_SOUND);
    assert_eq!(
        &values[..VALUES_PER_SOUND],
        &[10.5, 64.0, -3.5, 16.0, 1.0, 1.25, 6.0, 0.0, 0.0]
    );
    assert_eq!(values[VALUES_PER_SOUND + 3], f64::INFINITY);
    assert_eq!(values[VALUES_PER_SOUND + 7], 1.0);
    assert_eq!(values[VALUES_PER_SOUND + 8], 1.0);
    // Drained: the next frame gets nothing new (and the buffers are reused, cleared).
    q.take(&mut names, &mut values);
    assert!(names.is_empty() && values.is_empty());
}

#[test]
fn the_queue_keeps_the_newest_sounds_when_frames_stop() {
    let mut q = SoundEventQueue::default();
    q.set_listening(true);
    q.publish(listener(), (0..MAX_QUEUED + 3).map(|i| chicken(i as f64)));
    let (mut names, mut values) = (Vec::new(), Vec::new());
    q.take(&mut names, &mut values);
    assert_eq!(values.len(), MAX_QUEUED * VALUES_PER_SOUND);
    // The three oldest went first.
    assert_eq!(values[0], 3.0);
}

#[test]
fn stopping_to_listen_forgets_the_queue() {
    let mut q = SoundEventQueue::default();
    q.set_listening(true);
    q.publish(listener(), [chicken(0.0)]);
    q.set_listening(false);
    q.set_listening(true);
    let (mut names, mut values) = (Vec::new(), Vec::new());
    q.take(&mut names, &mut values);
    assert!(names.is_empty());
}

#[test]
fn the_initial_listener_is_vanillas() {
    assert_eq!(
        SoundEventQueue::default().take(&mut Vec::new(), &mut Vec::new()),
        [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0]
    );
}
