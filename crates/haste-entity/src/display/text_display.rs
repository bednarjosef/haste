//! `Display.TextDisplay`'s synched data and its `TextRenderState` (text, line width, the
//! opacity and background interpolators, the style flags).

use std::sync::Arc;

use haste_core::display::TextDisplayDecoration;
use haste_core::entity::MetadataValue;
use haste_core::Component;

use super::{byte, int, SUBCLASS_FIRST};
use crate::metadata::Metadata;

/// `TextDisplay.DATA_TEXT_ID` .. `DATA_STYLE_FLAGS_ID`.
pub const TEXT: u8 = SUBCLASS_FIRST;
pub const LINE_WIDTH: u8 = 24;
pub const BACKGROUND_COLOR: u8 = 25;
pub const TEXT_OPACITY: u8 = 26;
pub const STYLE_FLAGS: u8 = 27;

/// `TextDisplay.INITIAL_LINE_WIDTH`.
pub const INITIAL_LINE_WIDTH: i32 = 200;
/// `TextDisplay.INITIAL_BACKGROUND` (`0x40000000`: black at a quarter alpha).
pub const INITIAL_BACKGROUND: u32 = 0x4000_0000;
/// `TextDisplay.INITIAL_TEXT_OPACITY` (-1: fully opaque).
pub const INITIAL_TEXT_OPACITY: i8 = -1;

/// `TextDisplay.TextRenderState`: the opacity (`LinearIntInterpolator`) and background
/// (`ColorInterpolator`) as (previous, current) pairs, a constant when both are equal.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRenderState {
    pub text: Arc<Component>,
    pub line_width: i32,
    pub opacity: (i32, i32),
    pub background: (u32, u32),
    pub flags: u8,
}

impl TextRenderState {
    /// `createFreshTextRenderState`, or `createInterpolatedTextRenderState` from `previous`
    /// evaluated at `progress` (the text, width and flags never interpolate).
    pub fn build(m: &Metadata, previous: Option<&TextRenderState>, progress: f32) -> Self {
        let text = match m.get(TEXT).map(|e| &e.value) {
            Some(MetadataValue::Component(c)) => c.clone(),
            _ => Component::default(),
        };
        let opacity = i32::from(byte(m, TEXT_OPACITY, INITIAL_TEXT_OPACITY));
        let background = int(m, BACKGROUND_COLOR, INITIAL_BACKGROUND as i32) as u32;
        let (opacity_from, background_from) = match previous {
            Some(p) => (p.opacity_at(progress), p.background_at(progress)),
            None => (opacity, background),
        };
        TextRenderState {
            text: Arc::new(text),
            line_width: int(m, LINE_WIDTH, INITIAL_LINE_WIDTH),
            opacity: (opacity_from, opacity),
            background: (background_from, background),
            flags: byte(m, STYLE_FLAGS, 0) as u8,
        }
    }

    /// `textOpacity().get(progress)`: `Mth.lerpInt`.
    pub fn opacity_at(&self, progress: f32) -> i32 {
        lerp_int(progress, self.opacity.0, self.opacity.1)
    }

    /// `backgroundColor().get(progress)`: `ARGB.srgbLerp`, each channel by `Mth.lerpInt`.
    pub fn background_at(&self, progress: f32) -> u32 {
        let (from, to) = self.background;
        let channel = |shift: u32| {
            let c = lerp_int(
                progress,
                ((from >> shift) & 0xFF) as i32,
                ((to >> shift) & 0xFF) as i32,
            );
            ((c as u32) & 0xFF) << shift
        };
        channel(24) | channel(16) | channel(8) | channel(0)
    }

    /// The frame's text state with the interpolators evaluated.
    pub fn at(&self, progress: f32) -> TextDisplayDecoration {
        TextDisplayDecoration {
            text: self.text.clone(),
            line_width: self.line_width,
            // `submitInner` casts the interpolated opacity to a byte.
            opacity: self.opacity_at(progress) as i8,
            background: self.background_at(progress),
            flags: self.flags,
        }
    }
}

/// `Mth.lerpInt`: `p0 + floor(alpha * (p1 - p0))`.
pub fn lerp_int(alpha: f32, p0: i32, p1: i32) -> i32 {
    p0 + (alpha * (p1 - p0) as f32).floor() as i32
}
