//! Fetching 26.3 jars/assets/libraries from Mojang's manifest, running the data generator, config, launching the embedded vanilla server, launcher flow. Owner: WS7.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.

pub mod config;
pub mod download;
pub mod install;
pub mod mojang;
pub mod server;
