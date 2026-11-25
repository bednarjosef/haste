// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See docs/THIRD_PARTY.md.
//! Animated sprites (vanilla `SpriteContents.AnimatedTexture`): the frames and schedule.
use std::borrow::Cow;

use crate::assets::{frame_at, AnimationSchedule};

use super::{AtlasRegion, MipmapStrategy};
/// An animated sprite packed into the atlas: the region its current frame occupies
/// (all frames share it), the frames' RGBA pixels in schedule-index order, and the
/// playback schedule. The renderer re-uploads the region's texels each tick from
/// `frame_pixels`.
#[derive(Clone, Debug)]
pub struct AnimatedSprite {
    /// Texture id (`minecraft:block/water_still`) the animation belongs to.
    pub name: String,
    /// The atlas region all frames are shown in.
    pub region: AtlasRegion,
    /// Frames in schedule-index order; sizes match `region`.
    pub frames: Vec<image::RgbaImage>,
    pub schedule: AnimationSchedule,
    /// Mip treatment for the per-frame mip updates (vanilla `MipmapStrategy`).
    pub mipmap_strategy: MipmapStrategy,
    /// Alpha boost added when matching coverage (vanilla `alpha_cutoff_bias`).
    pub alpha_cutoff_bias: f32,
}

impl AnimatedSprite {
    /// The frame shown before animation starts (vanilla's initial display frame:
    /// the first listed frame, else frame 0). Panics when there are no frames.
    pub fn initial_frame(&self) -> &image::RgbaImage {
        let index = self.schedule.frames.first().map(|f| f.index).unwrap_or(0) as usize;
        self.frames.get(index).unwrap_or(&self.frames[0])
    }

    /// The RGBA pixels of the frame shown at `tick`, blending towards the next
    /// frame when the schedule interpolates. `Cow::Borrowed` is the exact frame
    /// (no interpolation or blend 0); blending allocates.
    pub fn frame_pixels(&self, tick: u64) -> Cow<'_, [u8]> {
        let (index, next, blend) = frame_at(&self.schedule, tick);
        let current = &self.frames[index as usize];
        if !self.schedule.interpolate || blend <= 0.0 {
            return Cow::Borrowed(current.as_raw());
        }
        let next = &self.frames[next as usize];
        Cow::Owned(blend_frames(current, next, blend))
    }
}

/// Linear per-channel blend of two same-size RGBA frames (vanilla `interpolate`;
/// alpha too, so a fading frame fades its coverage as well).
fn blend_frames(current: &image::RgbaImage, next: &image::RgbaImage, blend: f32) -> Vec<u8> {
    let (w, h) = current.dimensions();
    let mut out = Vec::with_capacity(current.as_raw().len());
    for y in 0..h {
        for x in 0..w {
            let a = current.get_pixel(x, y);
            let b = next.get_pixel(x, y);
            let mixed = [
                (f32::from(a.0[0]) * (1.0 - blend) + f32::from(b.0[0]) * blend).round() as u8,
                (f32::from(a.0[1]) * (1.0 - blend) + f32::from(b.0[1]) * blend).round() as u8,
                (f32::from(a.0[2]) * (1.0 - blend) + f32::from(b.0[2]) * blend).round() as u8,
                (f32::from(a.0[3]) * (1.0 - blend) + f32::from(b.0[3]) * blend).round() as u8,
            ];
            out.extend_from_slice(&mixed);
        }
    }
    out
}
