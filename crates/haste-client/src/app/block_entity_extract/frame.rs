use std::collections::HashMap;
use std::sync::Arc;

use haste_protocol::haste_core::BlockPos;

use super::gateway::gateway_beam_at;

/// One tick's extracted block entity render data: the drawable instances (with the tick-end
/// values, as vanilla's render states after a tick) plus what the render thread needs to fold
/// the partial tick back in. Published by the game thread each tick, cloned (an `Arc`) by the
/// render thread each frame.
pub struct BlockEntityTickFrame {
    /// The instances to draw, in the world walk's order, with per-tick values (`open` and the
    /// spin `TickPair`s are raw tick ends; `anim`, `banner_phase`, `beam_time`, the gateway
    /// beam and the piston progress are at partial 0, to be folded by [`fold_partial`]).
    pub instances: Vec<haste_render::BlockEntityInstance>,
    /// The end gateways' (age, max build y) by position, for the beam's partial fixup (the
    /// beam scale is `sin(clamp((age + partial) / 200) * PI)`, not a plain add).
    pub gateways: HashMap<BlockPos, (i64, i32)>,
}

/// A tick frame with nothing in it (before the game thread's first tick).
impl Default for BlockEntityTickFrame {
    fn default() -> Self {
        Self {
            instances: Vec::new(),
            gateways: HashMap::new(),
        }
    }
}

/// Folds the partial tick into a tick frame's interpolatable values, producing the frame's
/// instances (vanilla's per-frame submit reading the render state): the object-model anims
/// (`BlockEntityAnim::at`), the banner's wave phase (`(floorMod(...) + partial) / 100`), the
/// beams' animation time (`floorMod(gameTime, 40) + partial`), the end gateway's beam scale
/// and the moving piston's progress. Everything else is per-tick data copied unchanged.
pub fn fold_partial(
    tick: &BlockEntityTickFrame,
    partial: f32,
    out: &mut Vec<haste_render::BlockEntityInstance>,
) {
    out.clear();
    out.extend(tick.instances.iter().cloned());
    for inst in out.iter_mut() {
        // The object-model anims (bell, pot, conduit, enchanting table): the raw tick pair
        // lerped by the partial tick, the same `Mth.lerp(partialTicks, oX, X)` the feed did.
        if let Some(anim) = inst.anim {
            inst.anim = Some(anim.at(partial));
        }
        match inst.kind {
            haste_render::BlockEntityKind::Banner { .. } => {
                // Vanilla `BannerRenderer.extractRenderState`'s phase: the position hash plus
                // the world age over 100, plus the partial tick.
                inst.banner_phase += partial / 100.0;
            }
            haste_render::BlockEntityKind::Beacon | haste_render::BlockEntityKind::EndGateway => {
                // Vanilla's beam animation time: `floorMod(gameTime, 40) + partial`.
                inst.beam_time += partial;
                if let (true, Some((age, max_y))) = (
                    matches!(inst.kind, haste_render::BlockEntityKind::EndGateway),
                    tick.gateways.get(&inst.pos).copied(),
                ) {
                    inst.gateway_beam = gateway_beam_at(age, partial, max_y);
                }
            }
            haste_render::BlockEntityKind::MovingPiston => {
                // Vanilla `PistonMovingBlockEntity.getProgress`: the anim pair's ends lerped
                // by the partial tick (a fixed pair, the NBT value without an anim entry,
                // lerps to itself). The frame's `Arc` is shared with the tick frame, so a
                // fresh one is built from the cloned data (pistons are few and live a tick).
                if let Some(p) = inst.piston.clone() {
                    let mut d = (*p).clone();
                    let progress = d.progress;
                    d.progress = haste_render::TickPair {
                        start: progress.start,
                        end: progress.at(partial),
                    };
                    inst.piston = Some(Arc::new(d));
                }
            }
            _ => {}
        }
    }
}
