//! The sound engine core: which sounds are playing and how loud they are, with no audio
//! output at all.
//!
//! This is the fully testable tracking half of the engine; the rodio output stage is built
//! on top of it later and only consumes `gains()`.
use glam::DVec3;

use crate::sounds::SoundVariant;
use crate::spatial::{attenuation, clamp_pitch, pan, stereo_gains};

/// The vanilla sound categories a sound can belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundSource {
    Master,
    Music,
    Record,
    Weather,
    Block,
    Hostile,
    Neutral,
    Player,
    Ambient,
    Voice,
    Ui,
}

/// All sound-source volumes; the effective volume of a source is `master * source`.
#[derive(Clone, Debug)]
pub struct SourceVolumes {
    master: f32,
    sources: [f32; 10],
}

impl Default for SourceVolumes {
    fn default() -> Self {
        Self {
            master: 1.0,
            sources: [1.0; 10],
        }
    }
}

impl SourceVolumes {
    fn index(source: SoundSource) -> Option<usize> {
        match source {
            SoundSource::Master => None,
            SoundSource::Music => Some(0),
            SoundSource::Record => Some(1),
            SoundSource::Weather => Some(2),
            SoundSource::Block => Some(3),
            SoundSource::Hostile => Some(4),
            SoundSource::Neutral => Some(5),
            SoundSource::Player => Some(6),
            SoundSource::Ambient => Some(7),
            SoundSource::Voice => Some(8),
            SoundSource::Ui => Some(9),
        }
    }

    /// The volume of one source (`Master` gives the master volume as-is).
    pub fn get(&self, source: SoundSource) -> f32 {
        match Self::index(source) {
            Some(i) => self.sources[i],
            None => self.master,
        }
    }

    /// Set the volume of one source; `Master` scales every other source with it.
    pub fn set(&mut self, source: SoundSource, volume: f32) {
        match Self::index(source) {
            Some(i) => self.sources[i] = volume,
            None => self.master = volume,
        }
    }

    /// Effective gain multiplier of a source: `master * source`.
    pub fn effective(&self, source: SoundSource) -> f32 {
        self.master * self.get(source)
    }
}

/// One sound currently tracked by the engine.
#[derive(Clone, Debug)]
struct PlayingSound {
    /// Path of the Ogg inside the assets, used to identify the sound for `stop`.
    path: String,
    source: SoundSource,
    pos: Option<DVec3>,
    /// Overall volume of the sound: the play volume times the variant's volume.
    volume: f32,
    /// Final pitch multiplier: the play pitch times the variant's pitch, clamped to
    /// 0.5..=2.0.
    pitch: f32,
    attenuation_distance: u32,
    /// (left, right) gains as of the last `tick()`.
    gains: (f32, f32),
}

/// A sound engine that tracks playing sounds and their stereo gains but produces no
/// audio output whatsoever.
///
/// Gains are recomputed on `tick()` from the current listener state and source volumes,
/// so moving the listener or changing a volume takes effect on the next tick.
#[derive(Debug)]
pub struct SilentEngine {
    sounds: Vec<PlayingSound>,
    volumes: SourceVolumes,
    listener_pos: DVec3,
    listener_yaw: f32,
}

impl Default for SilentEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SilentEngine {
    pub fn new() -> Self {
        Self {
            sounds: Vec::new(),
            volumes: SourceVolumes::default(),
            listener_pos: DVec3::ZERO,
            listener_yaw: 0.0,
        }
    }

    /// Start tracking a sound. The variant's `volume`/`pitch` from sounds.json multiply
    /// into the play `volume`/`pitch`; the pitch is clamped here so every consumer sees
    /// valid input. Sounds with a position are attenuated and panned on `tick()`; sounds
    /// without one play at full gain, centred.
    pub fn play(
        &mut self,
        variant: &SoundVariant,
        source: SoundSource,
        pos: Option<DVec3>,
        volume: f32,
        pitch: f32,
    ) {
        self.sounds.push(PlayingSound {
            path: variant.path.clone(),
            source,
            pos,
            volume: volume * variant.volume,
            pitch: clamp_pitch(pitch * variant.pitch),
            attenuation_distance: variant.attenuation_distance,
            gains: (0.0, 0.0),
        });
    }

    /// Set the listener (camera) position and yaw for attenuation and panning.
    /// Minecraft yaw: 0 = facing +Z (south), increasing clockwise seen from above.
    pub fn set_listener(&mut self, pos: DVec3, yaw_deg: f32) {
        self.listener_pos = pos;
        self.listener_yaw = yaw_deg;
    }

    /// Set the volume of one sound source (or the master volume).
    pub fn set_volume(&mut self, source: SoundSource, volume: f32) {
        self.volumes.set(source, volume);
    }

    /// Stop playing sounds. `None` matches everything on that axis; a sound is stopped
    /// when both the source filter and the path filter match it.
    pub fn stop(&mut self, source: Option<SoundSource>, path: Option<&str>) {
        self.sounds.retain(|s| {
            let source_matches = source.is_none_or(|src| s.source == src);
            let path_matches = path.is_none_or(|p| s.path == p);
            !(source_matches && path_matches)
        });
    }

    /// Recompute the (left, right) gains of every playing sound from the current
    /// listener and source volumes.
    pub fn tick(&mut self) {
        for sound in &mut self.sounds {
            let effective = self.volumes.effective(sound.source);
            let (gain, pan) = match sound.pos {
                Some(pos) => (
                    attenuation(
                        (pos - self.listener_pos).length(),
                        sound.volume,
                        sound.attenuation_distance,
                    ) * effective,
                    pan(self.listener_pos, self.listener_yaw, pos),
                ),
                None => (sound.volume.min(1.0) * effective, 0.0),
            };
            sound.gains = stereo_gains(gain, pan);
        }
    }

    /// Number of sounds currently tracked as playing.
    pub fn playing(&self) -> usize {
        self.sounds.len()
    }

    /// The current (left, right) gains of all playing sounds, in play order.
    pub fn gains(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.sounds.iter().map(|s| s.gains)
    }

    /// The final (variant-adjusted, clamped) pitch of all playing sounds, in play order.
    pub fn pitches(&self) -> impl Iterator<Item = f32> + '_ {
        self.sounds.iter().map(|s| s.pitch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sounds::SoundVariant;
    use haste_core::Identifier;

    fn variant(path: &str, attenuation_distance: u32) -> SoundVariant {
        variant_with(path, attenuation_distance, 1.0, 1.0)
    }

    fn variant_with(
        path: &str,
        attenuation_distance: u32,
        volume: f32,
        pitch: f32,
    ) -> SoundVariant {
        SoundVariant {
            id: Identifier::new("minecraft", path.trim_end_matches(".ogg")),
            path: path.to_string(),
            volume,
            pitch,
            weight: 1,
            stream: false,
            attenuation_distance,
            preload: false,
        }
    }

    /// Sum of all gains, as a cheap "everything is silent" check.
    fn total_gains(engine: &SilentEngine) -> (f32, f32) {
        engine
            .gains()
            .fold((0.0, 0.0), |(l, r), (gl, gr)| (l + gl, r + gr))
    }

    #[test]
    fn non_positional_sound_is_full_gain_and_centred() {
        let mut engine = SilentEngine::new();
        engine.play(
            &variant("sounds/dig/stone1.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
        );
        engine.tick();
        assert_eq!(engine.playing(), 1);
        // Full gain, centred: each channel gets gain / sqrt(2).
        let (l, r) = engine.gains().next().unwrap();
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert_eq!(l, r);
    }

    #[test]
    fn master_volume_zero_silences_everything() {
        let mut engine = SilentEngine::new();
        engine.set_listener(DVec3::new(0.0, 0.0, 0.0), 0.0);
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            Some(DVec3::new(2.0, 0.0, 0.0)),
            1.0,
            1.0,
        );
        engine.play(
            &variant("sounds/b.ogg", 16),
            SoundSource::Ui,
            None,
            1.0,
            1.0,
        );
        engine.set_volume(SoundSource::Master, 0.0);
        engine.tick();
        assert_eq!(engine.playing(), 2);
        let (l, r) = total_gains(&engine);
        assert_eq!((l, r), (0.0, 0.0));
    }

    #[test]
    fn per_source_volume_scales_only_that_source() {
        let mut engine = SilentEngine::new();
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
        );
        engine.play(
            &variant("sounds/b.ogg", 16),
            SoundSource::Ui,
            None,
            1.0,
            1.0,
        );
        engine.set_volume(SoundSource::Block, 0.25);
        engine.tick();
        let gains: Vec<(f32, f32)> = engine.gains().collect();
        let norm = |(l, r): (f32, f32)| (l * l + r * r).sqrt();
        assert!((norm(gains[0]) - 0.25).abs() < 1e-5);
        assert!((norm(gains[1]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn stop_by_source() {
        let mut engine = SilentEngine::new();
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
        );
        engine.play(
            &variant("sounds/b.ogg", 16),
            SoundSource::Ui,
            None,
            1.0,
            1.0,
        );
        engine.play(
            &variant("sounds/c.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
        );
        engine.stop(Some(SoundSource::Block), None);
        assert_eq!(engine.playing(), 1);
        engine.tick();
        // Only the Ui sound survives.
        assert!((total_gains(&engine).0).abs() > 0.0);
    }

    #[test]
    fn stop_by_path() {
        let mut engine = SilentEngine::new();
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
        );
        engine.play(
            &variant("sounds/b.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
        );
        engine.stop(None, Some("sounds/a.ogg"));
        assert_eq!(engine.playing(), 1);
        // The remaining sound is b, not a.
        engine.stop(None, Some("sounds/b.ogg"));
        assert_eq!(engine.playing(), 0);
    }

    #[test]
    fn stop_all() {
        let mut engine = SilentEngine::new();
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            None,
            1.0,
            1.0,
        );
        engine.play(
            &variant("sounds/b.ogg", 16),
            SoundSource::Music,
            None,
            1.0,
            1.0,
        );
        engine.stop(None, None);
        assert_eq!(engine.playing(), 0);
    }

    #[test]
    fn positional_sound_out_of_range_after_listener_moves_is_silent() {
        let mut engine = SilentEngine::new();
        let sound = variant("sounds/dig/stone1.ogg", 16);
        engine.set_listener(DVec3::new(0.0, 0.0, 0.0), 0.0);
        engine.play(
            &sound,
            SoundSource::Block,
            Some(DVec3::new(4.0, 0.0, 0.0)),
            1.0,
            1.0,
        );
        engine.tick();
        let (l, r) = engine.gains().next().unwrap();
        assert!((l * l + r * r).sqrt() > 0.7, "in range should be loud");

        // Walk away: the same sound is now out of range.
        engine.set_listener(DVec3::new(1000.0, 0.0, 0.0), 0.0);
        engine.tick();
        assert_eq!(engine.playing(), 1);
        let (l, r) = engine.gains().next().unwrap();
        assert_eq!((l, r), (0.0, 0.0));
    }

    #[test]
    fn listener_move_scales_gain_without_replay() {
        let mut engine = SilentEngine::new();
        let sound = variant("sounds/a.ogg", 16);
        engine.set_listener(DVec3::ZERO, 0.0);
        engine.play(
            &sound,
            SoundSource::Block,
            Some(DVec3::new(8.0, 0.0, 0.0)),
            1.0,
            1.0,
        );
        engine.tick();
        let (l, r) = engine.gains().next().unwrap();
        assert!(
            (l * l + r * r).sqrt() - 0.5 < 1e-5,
            "half range = half gain"
        );
    }

    #[test]
    fn volume_two_sound_is_audible_at_distance_24() {
        // volume 2.0 doubles the audible range to 32, so 24 blocks is still in range.
        let mut engine = SilentEngine::new();
        engine.set_listener(DVec3::ZERO, 0.0);
        engine.play(
            &variant("sounds/a.ogg", 16),
            SoundSource::Block,
            Some(DVec3::new(24.0, 0.0, 0.0)),
            2.0,
            1.0,
        );
        engine.tick();
        let (l, r) = engine.gains().next().unwrap();
        // gain = min(2, 1) * (1 - 24/32) = 0.25, split over both channels.
        let norm = (l * l + r * r).sqrt();
        assert!((norm - 0.25).abs() < 1e-5, "norm {norm}");
        assert!(l <= 1.0 && r <= 1.0);
    }

    #[test]
    fn variant_volume_scales_the_gain() {
        // A variant with volume 0.5 played at volume 1.0 has half the gain of a
        // volume-1.0 variant at the same spot.
        let mut loud = SilentEngine::new();
        loud.set_listener(DVec3::ZERO, 0.0);
        loud.play(
            &variant_with("sounds/a.ogg", 16, 1.0, 1.0),
            SoundSource::Block,
            Some(DVec3::new(4.0, 0.0, 0.0)),
            1.0,
            1.0,
        );
        loud.tick();
        let mut quiet = SilentEngine::new();
        quiet.set_listener(DVec3::ZERO, 0.0);
        quiet.play(
            &variant_with("sounds/a.ogg", 16, 0.5, 1.0),
            SoundSource::Block,
            Some(DVec3::new(4.0, 0.0, 0.0)),
            1.0,
            1.0,
        );
        quiet.tick();
        let norm = |e: &SilentEngine| {
            let (l, r) = e.gains().next().unwrap();
            (l * l + r * r).sqrt()
        };
        assert!((norm(&loud) - 0.75).abs() < 1e-5);
        assert!((norm(&quiet) - 0.375).abs() < 1e-5);
    }

    #[test]
    fn variant_pitch_multiplies_and_clamps() {
        // Variant pitch 1.5 played at pitch 1.5: 2.25 clamps to 2.0.
        let mut engine = SilentEngine::new();
        engine.play(
            &variant_with("sounds/a.ogg", 16, 1.0, 1.5),
            SoundSource::Block,
            None,
            1.0,
            1.5,
        );
        assert_eq!(engine.pitches().next().unwrap(), 2.0);
    }
}
