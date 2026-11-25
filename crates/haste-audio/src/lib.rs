//! Sound: `sounds.json` from the assets, Ogg Vorbis decoding, positional playback of the sound
//! events the server sends. Owner: sound.
//!
//! See docs/INTERFACES.md for the contract this crate implements and docs/AGENTS.md for the rules.

pub mod channels;
pub mod engine;
pub mod music;
pub mod output;
pub mod random;
pub mod sound_event_listener;
pub mod sounds;
pub mod spatial;
