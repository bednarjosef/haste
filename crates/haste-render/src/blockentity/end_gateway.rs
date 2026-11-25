// The end gateway (vanilla `TheEndGatewayRenderer`): the visible-face rule of its starfield cube
// and its spawning/cooldown beam, drawn with the beacon's beam shape (`BeaconRenderer
// .submitBeaconBeam`).

use haste_core::{BlockPos, BlockStateId, Direction, Identifier};

use super::beacon::emit_beam;
use super::BlockEntityInstance;
use crate::entity::EntityVertex;

/// The end gateway beam (vanilla `EndGatewayRenderState`): the spawn/cooldown scale, the beam
/// height in blocks and the ARGB colour; the animation time rides per frame on the instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatewayBeam {
    /// Vanilla `state.scale` after its `sin(scale * PI)`.
    pub scale: f32,
    /// Vanilla `state.height`: `floor(scale * beamDistance)`, 0 hides the beam.
    pub height: i32,
    /// Magenta while spawning, purple while cooling down (`DyeColor.getTextureDiffuseColor`).
    pub color: u32,
}

/// The end gateway's visible-face rule (vanilla `TheEndGatewayBlockEntity.shouldRenderFace` ->
/// `Block.shouldRenderFace`): a face shows when the neighbour does not occlude it — an opaque
/// full cube hides it, unless it is the gateway block itself (portals draw against their own
/// type). `is_opaque` is the table's full-opaque check.
pub fn end_gateway_faces(
    get_state: impl Fn(BlockPos) -> BlockStateId,
    block_of: impl Fn(BlockStateId) -> haste_core::Identifier,
    is_opaque: impl Fn(BlockStateId) -> bool,
    state: BlockStateId,
    pos: BlockPos,
) -> u8 {
    let own = block_of(state);
    let mut faces = 0u8;
    for d in Direction::ALL {
        let neighbour = get_state(pos.offset(d));
        let same = block_of(neighbour) == own;
        if !same && is_opaque(neighbour) {
            continue;
        }
        faces |= 1 << d as u8;
    }
    faces
}

/// The end gateway's beam (vanilla `TheEndGatewayRenderer.submit`): the same beam shape at
/// radii 0.15/0.175, from `-height` to `+height`, scaled by the spawn/cooldown percent.
pub(crate) fn emit_gateway_beam(
    inner: &mut Vec<EntityVertex>,
    glow: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    let Some(beam) = inst.gateway_beam else {
        return;
    };
    if beam.height <= 0 {
        return;
    }
    let texture = Identifier::minecraft("entity/end_portal/end_gateway_beam");
    emit_beam(
        inner,
        glow,
        inst.pos,
        camera,
        &texture,
        rect_of,
        beam.scale,
        inst.beam_time,
        -beam.height,
        beam.height * 2,
        beam.color,
        1.0,
        0.15,
        0.175,
    );
}
