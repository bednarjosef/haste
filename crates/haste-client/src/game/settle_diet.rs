//! The settle diet: returning freed memory to the OS once the world goes quiet.
//!
//! Joining floods the client with short-lived data — at render distance 32 the server
//! streams ~120 MiB of chunk packets, decoded, meshed and dropped — and the allocators
//! keep those freed pages instead of handing them back: glibc's arenas only shrink their
//! tops, and the JVM's serial-GC heap stays grown after its join spike. So while the
//! player is idle, a background thread runs a diet: `malloc_trim(0)` walks glibc's
//! arenas and frees what they can spare, and once per session one full GC on the HUD's
//! JVM uncommits the heap's join spike (the JVM starts with a tight
//! `-XX:MaxHeapFreeRatio` so that GC shrinks it). The diet runs at most once a minute,
//! never on the render thread, and only after the world has been quiet — the one
//! full-GC pause lands while standing still, not while streaming chunks.
//!
//! Started with the game thread (see `run.rs`). "Quiet" is watched through
//! [`Shared::chunks_loaded`], the chunk-arrival counter the game thread publishes once a
//! second: it moves on every chunk the server sends (join flood, flying around), which
//! is exactly the work whose freed memory the diet returns; the meshing backlog always
//! drains within seconds of the last arrival (the bench settles on 1.5 s of it).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::Shared;

/// How long without a chunk arrival before a diet, and the minimum time between diets.
const QUIET: Duration = Duration::from_secs(15);
const INTERVAL: Duration = Duration::from_secs(60);
/// The diet thread's poll period.
const POLL: Duration = Duration::from_secs(5);

/// Starts the session's diet thread (called once per session from the game thread,
/// `game::run`): it exits when the connection closes.
pub(super) fn start(shared: Arc<Shared>) {
    std::thread::Builder::new()
        .name("haste-settle-diet".into())
        .spawn(move || diet_thread(shared))
        .expect("spawning the settle-diet thread");
}

/// Whether a diet is due: the world has been quiet for [`QUIET`] and the last diet (if
/// any) is older than [`INTERVAL`]. Pure over the inputs so the timing can be tested.
fn diet_due(quiet: Duration, since_last_diet: Option<Duration>) -> bool {
    quiet >= QUIET && since_last_diet.is_none_or(|t| t >= INTERVAL)
}

fn diet_thread(shared: Arc<Shared>) {
    let mut last_chunks = shared.chunks_loaded.load(Ordering::Relaxed);
    let mut last_arrival = Instant::now();
    let mut last_diet: Option<Instant> = None;
    let mut gc_done = false;
    loop {
        std::thread::sleep(POLL);
        if shared.is_closed() {
            return;
        }
        let chunks = shared.chunks_loaded.load(Ordering::Relaxed);
        if chunks != last_chunks {
            last_chunks = chunks;
            last_arrival = Instant::now();
        }
        if !diet_due(last_arrival.elapsed(), last_diet.map(|t| t.elapsed())) {
            continue;
        }
        last_diet = Some(Instant::now());
        // glibc: free the arenas' tops (the chunk-packet flood's remains).
        let t = Instant::now();
        let trimmed = unsafe { libc::malloc_trim(0) };
        tracing::debug!(
            "settle diet: malloc_trim returned {trimmed} in {:?}",
            t.elapsed()
        );
        // The HUD's JVM, once per session: a full GC, which with the JVM's
        // `-XX:MaxHeapFreeRatio` uncommits the join spike.
        if !gc_done {
            gc_done = true;
            if let Some(ui) = shared.ui.lock().clone() {
                if let Err(e) = ui.request_gc() {
                    tracing::debug!("settle diet: HUD gc skipped: {e:#}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diet_waits_for_quiet_and_interval() {
        assert!(!diet_due(Duration::from_secs(14), None));
        assert!(diet_due(Duration::from_secs(15), None));
        // the first diet after a quiet spell is due, later ones wait for the interval
        assert!(!diet_due(
            Duration::from_secs(60),
            Some(Duration::from_secs(59))
        ));
        assert!(diet_due(
            Duration::from_secs(60),
            Some(Duration::from_secs(60))
        ));
    }
}
