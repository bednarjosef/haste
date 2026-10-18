use std::collections::HashMap;
use std::sync::Arc;

use haste_protocol::haste_core::BlockPos;
use haste_world::World;

use super::piston::parse_piston_nbt;
use super::BlockEntityExtractor;

impl BlockEntityExtractor {
    /// The end gateway's age at `pos`: its NBT `Age` plus the ticks since the payload was
    /// first seen (the age ticks on client-side, vanilla `beamAnimationTick`, since the
    /// server sends the NBT only once). A new payload restarts from its own age.
    pub(super) fn gateway_age(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        world_age: i64,
    ) -> i64 {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        let entry = *self
            .gateway_ages
            .entry(pos)
            .or_insert_with(|| (key.0, key.1, world_age, parse_gateway_age(&be.nbt)));
        let (k, len, seen_at, age0) = entry;
        if (k, len) == key {
            age0 + (world_age - seen_at).max(0)
        } else {
            // A new payload: restart from its own age.
            let age = parse_gateway_age(&be.nbt);
            self.gateway_ages
                .insert(pos, (key.0, key.1, world_age, age));
            age
        }
    }

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
        let max_y = world.dimension().min_y + world.dimension().height - 1;
        let age = self.gateway_age(pos, be, world_age);
        (faces, gateway_beam_at(age, 0.0, max_y))
    }

    /// The gateway's entry for the tick frame's partial fixup: its age (see [`Self::gateway_age`])
    /// and the world's max build y (the beam's scale target).
    pub(super) fn gateway_frame_entry(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        world: &World,
        world_age: i64,
    ) -> (i64, i32) {
        let max_y = world.dimension().min_y + world.dimension().height - 1;
        let age = self.gateway_age(pos, be, world_age);
        (age, max_y)
    }

    /// The gateway logging state (debugging the draw): `true` when this is the first tick
    /// the position is seen or the beam's liveness changed, and the change to record.
    pub(super) fn gateway_log_state(&mut self, pos: BlockPos, beam_now: bool) -> (bool, bool) {
        let seen = self.logged_gateways.insert(pos);
        let changed = self
            .gateway_beam_live
            .insert(pos, beam_now)
            .is_some_and(|was| was != beam_now);
        (seen, changed)
    }

    /// The gateway's beam-plus-faces output for `extract` (the beam logging above it), as
    /// `(faces, beam)`.
    pub(super) fn gateway_render_data(
        &mut self,
        pos: BlockPos,
        world: &World,
        blocks: &haste_data::BlockStates,
        data: &haste_data::GameData,
        be: &haste_core::chunk::BlockEntityData,
        world_age: i64,
    ) -> (u8, Option<haste_render::GatewayBeam>) {
        self.gateway_data(pos, world, blocks, data, be, world_age)
    }

    /// The moving piston's data at `pos` from the block entity's NBT (vanilla
    /// `PistonMovingBlockEntity.loadAdditional`): the moved block state, facing, progress,
    /// extending and source flags. The progress is the game thread's tick (vanilla advances
    /// it 0.5 per tick to 1.0, then the block entity is gone); this stores the anim's raw
    /// tick pair for the render thread to fold by the partial tick (a fixed pair without an
    /// anim entry lerps to itself).
    pub(super) fn piston_data(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        anim_map: &HashMap<BlockPos, haste_render::BlockEntityAnim>,
    ) -> Option<Arc<haste_render::PistonData>> {
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, data)) = self.piston_cache.get(&pos) {
            if (*k, *len) == key {
                if let Some(data) = data {
                    // The progress ticks on: take the anim's raw tick pair (the render thread
                    // folds the partial tick into its ends).
                    let mut d = (**data).clone();
                    d.progress = anim_map
                        .get(&pos)
                        .map(|a| match a {
                            haste_render::BlockEntityAnim::Piston { progress } => *progress,
                            _ => d.progress,
                        })
                        .unwrap_or(d.progress);
                    return Some(Arc::new(d));
                }
                return None;
            }
        }
        let parsed = Arc::new(parse_piston_nbt(&be.nbt));
        self.piston_cache
            .insert(pos, (key.0, key.1, Some(parsed.clone())));
        Some(parsed)
    }
}

/// The end gateway's beam (vanilla `TheEndGatewayRenderer.extractRenderState`): spawning
/// under age 200 in magenta to the build limit; the cooldown phase arrives as a block event
/// (not in the NBT), so it is not drawn yet. `None` hides the beam.
pub(super) fn gateway_beam_at(
    age: i64,
    partial: f32,
    max_y: i32,
) -> Option<haste_render::GatewayBeam> {
    if age >= 200 {
        return None;
    }
    // Vanilla `getSpawnPercent`: `clamp((age + partial) / 200, 0, 1)`, then `sin(scale * PI)`;
    // the beam distance is the build limit (`level.getMaxY()`), the height its floor.
    let scale = ((age as f32 + partial) / 200.0).clamp(0.0, 1.0);
    let scale = (scale * std::f32::consts::PI).sin();
    let height = (scale as f64 * max_y as f64).floor() as i32;
    Some(haste_render::GatewayBeam {
        scale,
        height,
        // Vanilla `DyeColor.MAGENTA.getTextureDiffuseColor()` while spawning.
        color: haste_render::blockentity::DYE_DIFFUSE_COLOR[2],
    })
}

/// The end gateway's `Age` from its NBT (vanilla `EndGatewayBlockEntity.loadAdditional`,
/// default 0).
pub(super) fn parse_gateway_age(nbt: &[u8]) -> i64 {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper)
        .ok()
        .and_then(|doc| doc.root_tag.get_long("Age"))
        .unwrap_or(0)
}
