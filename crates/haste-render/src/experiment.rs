//! In-process A/B switch for measurements. Code under test reads [`on`] to pick its variant,
//! and the offscreen bench flips it every `HASTE_AB_BLOCK` frames, so both variants run
//! through the same GPU clock states: under the power-saver profile two separate runs of
//! one binary differ by up to 30 %, more than most changes being measured. Only for
//! stateless variants (a shader, a draw order); anything that carries state from frame to
//! frame (the GPU occlusion cull's last-frame visibility) must be compared in separate runs.

use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// Whether the variant under test is on this frame.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Selects the variant for the next frames (the bench's switch).
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}
