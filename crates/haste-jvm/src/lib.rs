//! Embedded JVM host, tier-3 vanilla class loading, tier-2 proxies, batched GUI pipeline. Owner: jvm-host.
//!
//! See docs/INTERFACES.md for the contract this crate implements and docs/AGENTS.md for the rules.
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

mod crate_dir;
mod error;
mod gui;
mod host;
// Declared here with `#[path]` (not in `proxies.rs`, which is at its recorded size cap): the
// item cooldowns mirror belongs with the proxies' player state.
#[path = "proxies/item_cooldowns.rs"]
pub mod item_cooldowns;
pub mod keys;
mod launch_options;
mod loader;
mod natives;
mod proxies;
pub mod sound_manager;

pub use error::JvmError;
pub use gui::{GuiPipeline, ScreenKind};
pub use host::{JvmConfig, JvmHost, StartTimings};
pub use item_cooldowns::{set_item_cooldowns, ItemCooldownHud, ItemCooldownsHud};
pub use loader::{ClassLog, ClassSource, ALLOW_CLASSES, ALLOW_PACKAGES, DENY, WARM_CLASSES};
pub use proxies::{
    decode_stack, empty_stack, encode_stack, AttackStrengthHud, ChatMessage, CreateWorldRequest,
    DebugInfo, GameState, MountHud, PlayerVitals, TargetedState, INVENTORY_SIZE,
};
