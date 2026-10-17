// The end gateway's per-frame render data (vanilla `TheEndGatewayRenderer.extractRenderState`):
// the starfield cube's visible faces and the spawning/cooldown beam.

use haste_protocol::haste_core::BlockPos;
use haste_world::World;

use super::WorldFeed;

impl WorldFeed {
    /// The end gateway's visible faces and beam at `pos`: the faces from the neighbouring
    /// blocks (vanilla `Block.shouldRenderFace`: an opaque neighbour hides the face unless it
    /// is the gateway block itself), the beam from the block entity's NBT age and cooldown
    /// (vanilla `TheEndGatewayRenderer.extractRenderState`: spawning under age 200 in magenta
    /// to the build limit, cooling down for 40 ticks in purple over 50 blocks).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn gateway_data(
        &mut self,
        pos: BlockPos,
        world: &World,
        blocks: &haste_data::BlockStates,
        data: &haste_data::GameData,
        be: &haste_core::chunk::BlockEntityData,
        world_age: i64,
        partial: f32,
    ) -> (u8, Option<haste_render::GatewayBeam>) {
        use haste_core::view::{BlockPhysics as _, BlockView as _};
        let mut faces = 0u8;
        let state = world.block_state(pos);
        let own = blocks.block_of(state).clone();
        for d in haste_core::Direction::ALL {
            let neighbour = world.block_state(pos.offset(d));
            let same = *blocks.block_of(neighbour) == own;
            let opaque = data.is_opaque_full(neighbour);
            if !same && opaque {
                continue;
            }
            faces |= 1 << d as u8;
        }
        // The gateway's age ticks on (vanilla `beamAnimationTick` runs on both sides) but the
        // server only sends the NBT once, so track the age ourselves from the payload's first
        // sight: age = age at sight + ticks since.
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        let entry = *self
            .gateway_ages
            .entry(pos)
            .or_insert_with(|| (key.0, key.1, world_age, parse_gateway_age(&be.nbt)));
        let (k, len, seen_at, age0) = entry;
        let age = if (k, len) == key {
            age0 + (world_age - seen_at).max(0)
        } else {
            // A new payload: restart from its own age.
            self.gateway_ages
                .insert(pos, (key.0, key.1, world_age, parse_gateway_age(&be.nbt)));
            parse_gateway_age(&be.nbt)
        };
        let beam = parse_gateway_beam(age, partial);
        (faces, beam)
    }
}

/// The end gateway's beam (vanilla `TheEndGatewayRenderer.extractRenderState`): spawning
/// under age 200 in magenta to the build limit; the cooldown phase arrives as a block event
/// (not in the NBT), so it is not drawn yet. `None` hides the beam.
fn parse_gateway_beam(age: i64, partial: f32) -> Option<haste_render::GatewayBeam> {
    if age >= 200 {
        return None;
    }
    // Vanilla `getSpawnPercent`: `clamp((age + partial) / 200, 0, 1)`, then `sin(scale * PI)`;
    // the beam distance is the build limit (`level.getMaxY()`), the height its floor. The
    // overworld's build limit is 320.
    let scale = ((age as f32 + partial) / 200.0).clamp(0.0, 1.0);
    let scale = (scale * std::f32::consts::PI).sin();
    let height = (scale * 320.0).floor() as i32;
    Some(haste_render::GatewayBeam {
        scale,
        height,
        // Vanilla `DyeColor.MAGENTA.getTextureDiffuseColor()` while spawning.
        color: haste_render::blockentity::DYE_DIFFUSE_COLOR[2],
    })
}

/// The gateway's `Age` from its NBT (vanilla `TheEndGatewayBlockEntity.loadAdditional`,
/// default 0).
fn parse_gateway_age(nbt: &[u8]) -> i64 {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper)
        .ok()
        .and_then(|doc| doc.root_tag.get_long("Age"))
        .unwrap_or(0)
}
