//! The local player's footsteps, ported from vanilla's movement-emission chain:
//! `Player.getMovementEmission` (no emission while flying or sneaking on the ground),
//! `Entity.applyMovementEmissionAndPlaySound` (the walked distance accumulates in
//! `moveDist`; a step fires when it passes `nextStep`, which only advances on a played
//! step) and `Entity.vibrationAndSoundEffectsFromBlock` (on the ground or climbing,
//! not swimming), with `Player.playStepSound` picking the block: the swim sound and
//! a muffled step in water, combination steps over carpets and snow, and the amethyst
//! chime (`Entity.walkingStepSound`). The block comes from
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
/// `Player.getSwimSound`: `entity.player.swim`.
const SWIM_SOUND: &str = "minecraft:entity.player.swim";
/// `SoundEvents.AMETHYST_BLOCK_CHIME`.
const AMETHYST_CHIME: &str = "minecraft:block.amethyst_block.chime";
/// `Entity.waterSwimSound`: the self-controlled volume modifier (0.35; a passenger
/// controlling us would be 0.4).
const SWIM_VOLUME_MODIFIER: f32 = 0.35;
/// `Entity.playAmethystStepSound`: the chime needs this many ticks since the last.
const AMETHYST_INTERVAL_TICKS: i64 = 20;
/// `Entity.playAmethystStepSound`: the intensity decays by this factor per tick.
const AMETHYST_DECAY: f64 = 0.997;
/// `Entity.playAmethystStepSound`: each chime adds this much intensity (capped at 1).
const AMETHYST_INTENSITY_STEP: f32 = 0.07;

/// One block the step rule looks at, with the facts the rule needs: its sound type
/// from the oracle dump, whether vanilla counts it climbable (`isStateClimbable`:
/// `#climbable` or powder snow) and whether it is in `#crystal_sound_blocks`.
pub struct StepBlock<'a> {
    pub pos: BlockPos,
    pub state: BlockStateId,
    /// `None` when the dump does not know the block (mods): plays nothing.
    pub sound: Option<&'a haste_data::BlockSoundType>,
    pub climbable: bool,
    /// In `#crystal_sound_blocks` (amethyst): steps chime (`shouldPlayAmethystStepSound`).
    pub crystal: bool,
}

/// `Player.getPrimaryStepSoundBlockPos`: the block above the effect pos when that is
/// in `#inside_step_sound_blocks` or `#combination_step_sound_blocks` — snow layers
/// and carpets stand between the feet and the block the effect pos finds.
pub struct PrimaryStep<'a> {
    pub block: StepBlock<'a>,
    /// In `#combination_step_sound_blocks` (carpets): `playCombinationStepSounds`
    /// plays this block's step over a muffled step of the effect block.
    pub combination: bool,
}

/// What one tick's step rule reads besides the blocks. The player tick fills this
/// from its state; the rules stay here.
pub struct StepCtx<'a> {
    /// The player's position (vanilla plays its own sounds at itself).
    pub player_pos: DVec3,
    /// The collision-clipped movement of this tick.
    pub movement: DVec3,
    /// The velocity the move started with (vanilla's `getDeltaMovement` inside
    /// `Entity.move`; the swim sound's volume comes from it).
    pub velocity: DVec3,
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
    /// The primary step-sound block (`Player.getPrimaryStepSoundBlockPos`), when it
    /// differs from the effect block.
    pub primary: Option<PrimaryStep<'a>>,
}

/// The local player's footstep state: vanilla `Entity.moveDist` and `nextStep`, plus
/// the amethyst chime's ramp.
pub struct StepSounds {
    /// Accumulated walked distance (vanilla `moveDist`).
    move_dist: f32,
    /// Distance at which the next step plays (vanilla `nextStep`, `(int)moveDist + 1`).
    next_step: f32,
    /// Our stand-in for `Entity.tickCount`, driving the amethyst interval.
    tick: i64,
    /// `Entity.crystalSoundIntensity`: rises with each chime, decays between them.
    crystal_intensity: f32,
    /// `Entity.lastCrystalSoundPlayTick`.
    last_crystal_tick: i64,
    /// `ClientLevel.random`: seeds each sound instance like `playLocalSound` does.
    random: LegacyRandom,
}

impl StepSounds {
    pub fn new() -> Self {
        Self {
            move_dist: 0.0,
            next_step: 1.0,
            tick: 0,
            crystal_intensity: 0.0,
            last_crystal_tick: 0,
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
        self.tick += 1;
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
                // and the swim sound replaces the step (`waterSwimSound`).
                self.next_step = self.move_dist as i32 as f32 + 1.0;
                self.water_swim_sound(ctx, requests);
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
            self.walking_step(ctx, block, requests);
        }
        plays
    }

    /// `Entity.walkingStepSound` -> `Player.playStepSound`: in water the swim sound
    /// plus a muffled step of the block; otherwise the primary step-sound block's
    /// plain step (carpets play over a muffled step of the block below), or the
    /// block's own. Then the amethyst chime when the block is amethyst.
    fn walking_step(
        &mut self,
        ctx: &StepCtx<'_>,
        block: &StepBlock<'_>,
        requests: &mut Vec<SoundRequest>,
    ) {
        if ctx.in_water {
            // Player.playStepSound's water branch.
            self.water_swim_sound(ctx, requests);
            self.muffled_step(ctx, block, requests);
        } else {
            match &ctx.primary {
                Some(primary) if primary.combination => {
                    // playCombinationStepSounds: the carpet's step, then the block
                    // below it muffled.
                    self.plain_step(ctx, &primary.block, requests);
                    self.muffled_step(ctx, block, requests);
                }
                Some(primary) => self.plain_step(ctx, &primary.block, requests),
                None => self.plain_step(ctx, block, requests),
            }
        }
        // Entity.walkingStepSound's amethyst chime (shouldPlayAmethystStepSound).
        if block.crystal && self.tick >= self.last_crystal_tick + AMETHYST_INTERVAL_TICKS {
            self.amethyst_chime(ctx.player_pos, requests);
        }
    }

    /// `Entity.playStepSound`: the block's step sound at `volume * 0.15`, pitch
    /// unchanged, PLAYER source, played at the player.
    fn plain_step(
        &mut self,
        ctx: &StepCtx<'_>,
        block: &StepBlock<'_>,
        requests: &mut Vec<SoundRequest>,
    ) {
        let Some(t) = block.sound else {
            return; // not in the dump (mods): no sound, no guess
        };
        self.play(
            &t.step_sound,
            ctx.player_pos,
            t.volume * 0.15,
            t.pitch,
            requests,
        );
    }

    /// `Entity.playMuffledStepSound`: the block's step sound at `volume * 0.05`,
    /// `pitch * 0.8`.
    fn muffled_step(
        &mut self,
        ctx: &StepCtx<'_>,
        block: &StepBlock<'_>,
        requests: &mut Vec<SoundRequest>,
    ) {
        let Some(t) = block.sound else {
            return;
        };
        self.play(
            &t.step_sound,
            ctx.player_pos,
            t.volume * 0.05,
            t.pitch * 0.8,
            requests,
        );
    }

    /// `Entity.waterSwimSound` -> `playSwimSound` (`Player.getSwimSound`:
    /// `entity.player.swim`): the volume follows the movement's speed, the pitch
    /// wobbles by `1.0 ± (rand - rand) * 0.4`.
    fn water_swim_sound(&mut self, ctx: &StepCtx<'_>, requests: &mut Vec<SoundRequest>) {
        let v = ctx.velocity;
        let speed = ((v.x * v.x * 0.2 + v.y * v.y + v.z * v.z * 0.2).sqrt() as f32
            * SWIM_VOLUME_MODIFIER)
            .min(1.0);
        let pitch = 1.0 + (self.random.next_float() - self.random.next_float()) * 0.4;
        requests.push(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Inline(SWIM_SOUND.to_owned()),
            category: CATEGORY_PLAYERS,
            pos: SoundPos::Fixed(ctx.player_pos),
            volume: speed,
            pitch,
            seed: self.random.next_long(),
        }));
    }

    /// `Entity.playAmethystStepSound`: the intensity decays since the last chime,
    /// rises by 0.07 (capped at 1), and the chime's volume and pitch follow it.
    fn amethyst_chime(&mut self, at: DVec3, requests: &mut Vec<SoundRequest>) {
        let since = (self.tick - self.last_crystal_tick) as f64;
        self.crystal_intensity = (self.crystal_intensity * AMETHYST_DECAY.powf(since) as f32
            + AMETHYST_INTENSITY_STEP)
            .min(1.0);
        let volume = 0.1 + self.crystal_intensity * 1.2;
        let pitch = 0.5 + self.crystal_intensity * self.random.next_float() * 1.2;
        self.last_crystal_tick = self.tick;
        requests.push(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Inline(AMETHYST_CHIME.to_owned()),
            category: CATEGORY_PLAYERS,
            pos: SoundPos::Fixed(at),
            volume,
            pitch,
            seed: self.random.next_long(),
        }));
    }

    /// One step-sound request at `at` (vanilla plays its own sounds at itself).
    fn play(
        &mut self,
        event: &str,
        at: DVec3,
        volume: f32,
        pitch: f32,
        requests: &mut Vec<SoundRequest>,
    ) {
        if event == "minecraft:intentionally_empty" {
            return; // vanilla's "no sound" event
        }
        requests.push(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Inline(event.to_owned()),
            category: CATEGORY_PLAYERS,
            pos: SoundPos::Fixed(at),
            volume,
            pitch,
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

/// Resolves the blocks of [`StepCtx`] at `pos`: the supporting block from the
/// collision under the box (vanilla `findSupportingBlock` through
/// `mainSupportingBlockPos`; the cell at feet y minus 1e-5 when airborne or nothing
/// supports the box), the effect block at y minus 0.2 (which keeps the supporting
/// cell's x/z on the ground and stays the supporting cell itself on fences, walls and
/// fence gates, vanilla `getOnPos(0.2F)`) and the primary step-sound block
/// (`Player.getPrimaryStepSoundBlockPos`: the cell above the effect pos when it is
/// in `#inside_step_sound_blocks` or `#combination_step_sound_blocks`).
pub fn resolve_step_blocks<'a>(
    pos: DVec3,
    aabb: &Aabb,
    on_ground: bool,
    world: &dyn BlockView,
    shapes: &dyn CollisionShapes,
    data: &'a haste_data::GameData,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> (StepBlock<'a>, StepBlock<'a>, Option<PrimaryStep<'a>>) {
    let climb = crate::player::climbable::SyncedClimbables::new(tags, Some(&data.blocks));
    let block = |pos: BlockPos| {
        let state = world.block_state(pos);
        StepBlock {
            pos,
            state,
            sound: data.block_sound_type(state),
            climbable: climb.is_climbable(state) || data.is_powder_snow(state),
            crystal: in_tag(tags, data, state, "crystal_sound_blocks"),
        }
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
    let effect = block(effect_pos);
    // Player.getPrimaryStepSoundBlockPos: the block above the effect pos when it is
    // in the inside or combination step-sound tags.
    let above = block(effect_pos.add(0, 1, 0));
    let primary = if above.state != BlockStateId::AIR
        && (in_tag(tags, data, above.state, "inside_step_sound_blocks")
            || in_tag(tags, data, above.state, "combination_step_sound_blocks"))
    {
        Some(PrimaryStep {
            combination: in_tag(tags, data, above.state, "combination_step_sound_blocks"),
            block: above,
        })
    } else {
        None
    };
    (supporting, effect, primary)
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
