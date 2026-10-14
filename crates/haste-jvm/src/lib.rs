//! Embedded JVM host, tier-3 vanilla class loading, tier-2 proxies, batched GUI pipeline. Owner: jvm-host.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//!
//! * [`JvmHost`] starts the one JVM of the process (invocation API, libjvm from the configured JDK),
//!   compiles Haste's Java side if needed (`build-java.sh`), generates the tier-2 proxies
//!   (`proxies.conf` -> `cache/proxies/26.3/`) and installs `HasteClassLoader`, which serves
//!   allow-listed vanilla classes from the player's client jar one at a time (the jar is never on
//!   the class path).
//! * [`GuiPipeline`] drives real vanilla screens (`PauseScreen`, `ChatScreen`, `InventoryScreen`):
//!   input goes in through the 26.3 event signatures and `render()` returns the whole frame as a
//!   [`haste_core::gui::GuiFrame`] from one JNI call.
//! * [`GameState`] is the Rust-owned state the proxies read through natives (inventory, chat
//!   history, player name) and the outbox screens write to (sent chat messages).

mod error;
mod gui;
mod host;
pub mod keys;
mod loader;
mod natives;
mod proxies;

pub use error::JvmError;
pub use gui::{GuiPipeline, ScreenKind};
pub use host::{JvmConfig, JvmHost, StartTimings};
pub use loader::{ClassLog, ClassSource, ALLOW_CLASSES, ALLOW_PACKAGES, DENY};
pub use proxies::{
    decode_stack, empty_stack, encode_stack, ChatMessage, DebugInfo, GameState, PlayerVitals,
    TargetedState, INVENTORY_SIZE,
};
