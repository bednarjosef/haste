//! The slow-frame log (hitch diagnosis): the phase timings `draw` reports for frames whose
//! whole interval is slow, so a stall outside the renderer's own numbers can be attributed.

use super::*;

/// A frame whose whole interval (the previous frame's end to this one's) exceeds this is
/// logged with its phase breakdown.
const HITCH_MS: f32 = 20.0;

/// The slow-frame log: `draw` reports every frame over [`HITCH_MS`] with the time each phase
/// took, so a stall outside the renderer's own numbers (a JVM GC pause in the GUI frame, a
/// lock the game thread holds, the event loop between frames) can be attributed to a phase.
/// Only the game path reports; menu frames are not interesting.
pub(crate) struct HitchTrace {
    /// When the last reported frame's `draw` ended.
    last_draw_end: Option<Instant>,
}

impl HitchTrace {
    pub(crate) fn new() -> Self {
        Self {
            last_draw_end: None,
        }
    }

    /// The time since the last frame's `draw` ended (the event loop and the pre-draw part of
    /// `about_to_wait`), in ms; 0 for the first traced frame.
    pub(super) fn frame_started(&mut self) -> f32 {
        let now = Instant::now();
        self.last_draw_end
            .map_or(0.0, |last| (now - last).as_secs_f32() * 1e3)
    }

    /// Reports the frame when its whole interval (gap plus every phase) was slow. `p`'s
    /// phases are in draw order; `lock` is the view mutex wait inside `view`.
    pub(super) fn frame_ended(&mut self, gap_ms: f32, p: FramePhases) {
        let now = Instant::now();
        let total_ms = (now - self.last_draw_end.unwrap_or(now)).as_secs_f32() * 1e3;
        self.last_draw_end = Some(now);
        if total_ms <= HITCH_MS {
            return;
        }
        tracing::warn!(
            "hitch: frame {total_ms:.1} ms: gap {gap_ms:.2} lock {:.2} view {:.2} feed {:.2} \
             entities {:.2} gui {:.2} pre {:.2} render {:.2} post {:.2}",
            p.view_lock,
            p.view,
            p.feed,
            p.entities,
            p.gui,
            p.pre,
            p.render,
            p.post,
        );
    }
}

/// One frame's phase times, milliseconds: `view` is the game view (lock, options, camera),
/// `entities` the feed's entity pass, `gui` the HUD frame (the JVM call), `pre` the world
/// sampling and particles, `render` the renderer call, `post` the title bar and capture.
#[allow(clippy::too_many_arguments)]
pub(super) struct FramePhases {
    pub(super) view_lock: f32,
    pub(super) view: f32,
    pub(super) feed: f32,
    pub(super) entities: f32,
    pub(super) gui: f32,
    pub(super) pre: f32,
    pub(super) render: f32,
    pub(super) post: f32,
}
