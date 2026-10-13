//! Sound: `sounds.json` from the assets, Ogg Vorbis decoding, positional playback of the sound
//! events the server sends. Owner: sound.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

pub mod engine;
pub mod music;
pub mod sounds;
pub mod spatial;
