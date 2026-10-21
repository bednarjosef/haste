//! The frame's leash ropes (vanilla `LeashFeatureRenderer`'s endpoints): the entity store's
//! lines, with each end's light sampled from the world at the block the rope end sits in
//! (the mob probe of `EntityInstance::sample_light`: `floor(y + 0.5)` for the height).

use glam::DVec3;
use haste_core::view::BlockView;
use haste_core::BlockPos;
use haste_entity::leash::LeashLine as EntityLeash;
use haste_render::entity::LeashLine;

/// Converts `lines` into `out` (cleared first, so the buffer is reused frame to frame).
pub(super) fn convert(world: &dyn BlockView, lines: &[EntityLeash], out: &mut Vec<LeashLine>) {
    out.clear();
    out.extend(lines.iter().map(|line| LeashLine {
        start: line.start,
        end: line.end,
        start_light: light_at(world, line.start),
        end_light: light_at(world, line.end),
    }));
}

/// `(block, sky)` light at the block containing a rope end: the lightmap's u is block light and
/// its v is sky light (the order `EntityRenderer` reads them in).
fn light_at(world: &dyn BlockView, p: DVec3) -> (u8, u8) {
    let pos = BlockPos::new(
        p.x.floor() as i32,
        (p.y + 0.5).floor() as i32,
        p.z.floor() as i32,
    );
    (world.block_light(pos), world.sky_light(pos))
}
