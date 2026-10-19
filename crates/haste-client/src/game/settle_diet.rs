//! The settle diet: returning freed memory to the OS once the world goes quiet.
//!
//! Joining floods the client with short-lived data — at render distance 32 the server
//! streams ~120 MiB of chunk packets, decoded, meshed and dropped — and glibc's arenas
//! keep those freed pages instead of handing them back: they only shrink their tops. So
//! while the player is idle, a background thread runs a diet: `malloc_trim(0)` walks the
//! arenas and frees what they can spare. The diet runs at most once a minute, never on
//! the render thread, and only after the world has been quiet. (The JVM's heap is
//! returned by its own G1 periodic GC, see `haste-jvm`'s launch options.)
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
        // glibc: free the arenas' tops (the chunk-packet flood's remains). It runs on
        // this thread, never the game or render one; the duration is logged so a slow
        // walk shows up (it takes the arenas' locks, so a concurrently allocating
        // thread can briefly wait on one — never during play, the world is quiet).
        trim();
    }
}

/// glibc's `malloc_trim`; other allocators return freed memory on their own.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim() {
    let t = Instant::now();
    let trimmed = unsafe { libc::malloc_trim(0) };
    tracing::info!(
        "settle diet: malloc_trim returned {trimmed} in {:?}",
        t.elapsed()
    );
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim() {}

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
