//! The local player's footsteps, ported from vanilla's movement-emission chain:
//! `Player.getMovementEmission` (no emission while flying or sneaking on the ground),
//! `Entity.applyMovementEmissionAndPlaySound` (the walked distance accumulates in
//! `moveDist`; a step fires when it passes `nextStep`, which only advances on a played
//! step) and `Entity.vibrationAndSoundEffectsFromBlock` (on the ground or climbing,
//! not swimming), with `Player.playStepSound` picking the block. The block comes from
//! `mainSupportingBlockPos` (the collision under the feet) and the effect position
//! (`getOnPosLegacy`, feet y minus 0.2), like vanilla.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId};
use haste_physics::ClimbableBlocks;

use super::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// `SoundSource.PLAYERS`, the category of the player's own sounds.
const CATEGORY_PLAYERS: i32 = 7;
/// How much of the walked distance counts (vanilla `applyMovementEmissionAndPlaySound`:
/// the clipped horizontal movement times 0.6, the full length while climbing).
const MOVE_DIST_SCALE: f32 = 0.6;

/// One block the step rule looks at, with the facts the rule needs: its sound type
/// from the oracle dump and whether vanilla counts it climbable (`isStateClimbable`:
/// `#climbable` or powder snow).
pub struct StepBlock<'a> {
    pub pos: BlockPos,
    pub state: BlockStateId,
    /// `None` when the dump does not know the block (mods): plays nothing.
    pub sound: Option<&'a haste_data::BlockSoundType>,
    pub climbable: bool,
}

/// What one tick's step rule reads besides the blocks. The player tick fills this
/// from its state; the rules stay here.
pub struct StepCtx<'a> {
    /// The collision-clipped movement of this tick.
    pub movement: DVec3,
    pub on_ground: bool,
    /// Creative flight (vanilla `abilities.flying`).
    pub flying: bool,
    /// Shift held (vanilla `isDiscrete`).
    pub discrete: bool,
    /// The crouch pose (vanilla `isCrouching`).
    pub crouching: bool,
    /// Sprint-swimming (vanilla `isSwimming`, shared flag 4).
    pub swimming: bool,
    /// In water (vanilla `isInWater`).
    pub in_water: bool,
    /// The main supporting block (vanilla `mainSupportingBlockPos` -> `getOnPos()`).
    pub supporting: StepBlock<'a>,
    /// The effect block (`getOnPosLegacy`, feet y - 0.2): the one the step sound uses.
    pub effect: StepBlock<'a>,
}

/// The local player's footstep state: vanilla `Entity.moveDist` and `nextStep`.
pub struct StepSounds {
    /// Accumulated walked distance (vanilla `moveDist`).
    move_dist: f32,
    /// Distance at which the next step plays (vanilla `nextStep`, `(int)moveDist + 1`).
    next_step: f32,
    /// `ClientLevel.random`: seeds each sound instance like `playLocalSound` does.
    random: LegacyRandom,
}

impl StepSounds {
    pub fn new() -> Self {
        Self {
            move_dist: 0.0,
            next_step: 1.0,
            random: LegacyRandom::new(super::own_block::unseeded()),
        }
    }

    /// Resets the accumulator (teleport, respawn, flight start).
    pub fn reset(&mut self) {
        self.move_dist = 0.0;
        self.next_step = 1.0;
    }

    /// The step distance, for tests.
    #[cfg(test)]
    pub fn move_dist(&self) -> f32 {
        self.move_dist
    }

    /// The next step's distance, for tests.
    #[cfg(test)]
    pub fn next_step(&self) -> f32 {
        self.next_step
    }

    /// One tick's movement, after physics: accumulates the walked distance and plays a
    /// step when vanilla's rule fires. `Entity.applyMovementEmissionAndPlaySound`:
    /// the distance only accumulates with emission (never while flying or sneaking on
    /// the ground), and a step needs a non-air supporting block below.
    pub fn after_move(&mut self, ctx: &StepCtx<'_>, requests: &mut Vec<SoundRequest>) {
        // Player.getMovementEmission: NONE while flying or on the ground with shift
        // held — no sounds, and moveDist does not accumulate either.
        if ctx.flying || (ctx.on_ground && ctx.discrete) {
            return;
        }
        // Climbing uses the full movement length; walking the horizontal part.
        let horiz = (ctx.movement.x * ctx.movement.x + ctx.movement.z * ctx.movement.z).sqrt();
        let moved = if ctx.supporting.climbable {
            ctx.movement.length()
        } else {
            horiz
        };
        self.move_dist += (moved as f32) * MOVE_DIST_SCALE;
        if self.move_dist > self.next_step && ctx.supporting.state != BlockStateId::AIR {
            // The effect block (y - 0.2) carries the sound; when the supporting block
            // differs, only its vibration counts (vanilla's second call has
            // shouldSound = false).
            let only_effect = ctx.supporting.pos == ctx.effect.pos;
            let mut produced = self.block_effects(ctx, &ctx.effect, true, requests);
            if !only_effect {
                produced |= self.block_effects(ctx, &ctx.supporting, false, requests);
            }
            if produced {
                // nextStep advances only on a played step (or the water case below).
                self.next_step = self.move_dist as i32 as f32 + 1.0;
            } else if ctx.in_water {
                // In water the blocks produce nothing; the accumulator still advances
                // and the swim sound replaces the step (added with the water rules).
                self.next_step = self.move_dist as i32 as f32 + 1.0;
            }
        }
    }

    /// `Entity.vibrationAndSoundEffectsFromBlock`: the block must not be air, the
    /// player on the ground (or climbing this block, or crouching without vertical
    /// movement) and not swimming. Returns whether side effects were produced, so
    /// `nextStep` only advances on a played step.
    fn block_effects(
        &mut self,
        ctx: &StepCtx<'_>,
        block: &StepBlock<'_>,
        should_sound: bool,
        requests: &mut Vec<SoundRequest>,
    ) -> bool {
        if block.state == BlockStateId::AIR {
            return false;
        }
        let plays = (ctx.on_ground || block.climbable || (ctx.crouching && ctx.movement.y == 0.0))
            && !ctx.swimming;
        if plays && should_sound {
            self.walking_step(block, requests);
        }
        plays
    }

    /// `Player.playStepSound` (the plain part): the block's step sound at
    /// `volume * 0.15`, pitch unchanged, PLAYER source.
    fn walking_step(&mut self, block: &StepBlock<'_>, requests: &mut Vec<SoundRequest>) {
        let Some(t) = block.sound else {
            return; // not in the dump (mods): no sound, no guess
        };
        if t.step_sound == "minecraft:intentionally_empty" {
            return; // vanilla's "no sound" event
        }
        requests.push(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Inline(t.step_sound.clone()),
            category: CATEGORY_PLAYERS,
            pos: SoundPos::Fixed(block.pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5)),
            volume: t.volume * 0.15,
            pitch: t.pitch,
            seed: self.random.next_long(),
        }));
    }
}

impl Default for StepSounds {
    fn default() -> Self {
        Self::new()
    }
}

/// The position vanilla's `getOnPos(offset)` resolves to without a supporting block:
/// the cell of the feet moved down by `offset`.
fn on_pos(pos: DVec3, offset: f64) -> BlockPos {
    BlockPos::new(
        pos.x.floor() as i32,
        (pos.y - offset).floor() as i32,
        pos.z.floor() as i32,
    )
}

/// Resolves the two blocks of [`StepCtx`] at `pos`: the supporting block from the
/// collision under the box (vanilla `findSupportingBlock` through
/// `mainSupportingBlockPos`; the cell at feet y minus 1e-5 when airborne or nothing
/// supports the box) and the effect block at y minus 0.2, which keeps the supporting
/// cell's x/z on the ground and stays the supporting cell itself on fences, walls and
/// fence gates (vanilla `getOnPos(0.2F)`).
pub fn resolve_step_blocks<'a>(
    pos: DVec3,
    aabb: &Aabb,
    on_ground: bool,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    data: &'a haste_data::GameData,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> (StepBlock<'a>, StepBlock<'a>) {
    let climb = crate::player::climbable::SyncedClimbables::new(tags, Some(&data.blocks));
    let block = |pos: BlockPos| StepBlock {
        pos,
        state: world.block_state(pos),
        sound: data.block_sound_type(world.block_state(pos)),
        climbable: climb.is_climbable(world.block_state(pos))
            || data.is_powder_snow(world.block_state(pos)),
    };
    let supporting_pos = if on_ground {
        haste_physics::collision::supporting_block(aabb, world, shapes)
            .unwrap_or_else(|| on_pos(pos, 1.0e-5))
    } else {
        on_pos(pos, 1.0e-5)
    };
    let supporting = block(supporting_pos);
    // getOnPos(0.2): the fence family keeps the supporting cell (its top is tall
    // enough that y - 0.2 lands inside it).
    let keeps_supporting_y = supporting.state != BlockStateId::AIR
        && (in_tag(tags, data, supporting.state, "fences")
            || in_tag(tags, data, supporting.state, "walls")
            || is_fence_gate(data, supporting.state));
    let effect_pos = if on_ground && keeps_supporting_y {
        supporting_pos
    } else if on_ground {
        BlockPos::new(
            supporting_pos.x,
            (pos.y - 0.2).floor() as i32,
            supporting_pos.z,
        )
    } else {
        on_pos(pos, 0.2)
    };
    (supporting, block(effect_pos))
}

/// `#minecraft:<tag>` membership of `state`'s block through the synced tags.
fn in_tag(
    tags: Option<&haste_physics::mining::BlockTags>,
    data: &haste_data::GameData,
    state: BlockStateId,
    tag: &str,
) -> bool {
    tags.is_some_and(|t| {
        data.blocks
            .block_registry_index_of_state(state)
            .is_some_and(|i| t.contains(&haste_core::Identifier::minecraft(tag), i.0))
    })
}

/// `FenceGateBlock` by registry name (vanilla checks the class; there is no
/// fence-gate tag): `minecraft:*_fence_gate`.
fn is_fence_gate(data: &haste_data::GameData, state: BlockStateId) -> bool {
    let name = data.blocks.block_of(state);
    name.namespace() == "minecraft" && name.path().ends_with("_fence_gate")
}
