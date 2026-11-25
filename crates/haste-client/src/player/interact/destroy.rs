//! Block breaking (player workstream): `MultiPlayerGameMode.startDestroyBlock` /
//! `continueDestroyBlock` / `stopDestroyBlock` / `sameDestroyTarget`, the mining parameters
//! (`Player.getDigSpeed`'s inputs) they read, and the destroy `player_action`s plus the
//! client-side break prediction they send.

use glam::DVec3;
use haste_core::game::GameMode;
use haste_core::view::BlockPhysics;
use haste_core::view::BlockView;
use haste_core::{BlockPos, BlockStateId, Direction, RegistryIndex};
use haste_physics::mining::MiningParams;
use haste_physics::raycast::BlockHit;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::super::LocalPlayer;
use super::{action, InteractEnv, DESTROY_DELAY};

impl LocalPlayer {
    /// Our own breaking: block and crack stage 0..=9.
    pub fn break_progress(&self) -> Option<(BlockPos, u8)> {
        let (pos, _, _) = self.int.destroying.as_ref()?;
        if self.int.progress <= 0.0 {
            return None;
        }
        Some((*pos, ((self.int.progress * 10.0) as u8).min(9)))
    }

    /// `MultiPlayerGameMode.startDestroyBlock`: creative breaks at once (with `destroyDelay` 5,
    /// and nothing with a tool that cannot break blocks in creative); survival starts the
    /// break and finishes it in the same call when one tick of progress is enough.
    ///
    /// Returns vanilla's pair: `.0` is `startDestroyBlock`'s own "handled" (false only when
    /// the block cannot be acted on at all — a restricted mode, or outside the world
    /// border; `Minecraft.continueAttack` swings only then), `.1` the `endAttack`
    /// `Minecraft.startAttack` reads afterwards (the block is air now, so continueAttack is
    /// skipped this tick).
    pub(super) fn start_destroy(
        &mut self,
        h: BlockHit,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) -> (bool, bool) {
        let state = world.block_state(h.pos);
        // A block we broke this tick (or whose break the server has not acknowledged yet) is
        // gone for us even if the world has not applied the edit yet.
        let predicted_gone = self.int.prediction.is_predicted(h.pos);
        if env.data.is_air(state) || predicted_gone {
            return (false, false);
        }
        // `Player.blockActionRestricted`: spectator (like adventure) never breaks; no packet.
        if self.game_mode == GameMode::Adventure || self.game_mode == GameMode::Spectator {
            // Breaking in adventure depends on the held item's can_break predicate (not decoded).
            return (false, false);
        }
        // `MultiPlayerGameMode.startDestroyBlock` refuses blocks outside the world border the
        // same way: no packet, nothing predicted.
        if !env.border.is_none_or(|b| b.is_within_bounds(h.pos)) {
            return (false, false);
        }
        if self.game_mode == GameMode::Creative {
            // startDestroyBlock has no destroyDelay check: a fresh click breaks at once even
            // while the 5-tick delay from a previous break is still counting down (the delay
            // only gates continueDestroyBlock while the button is held).
            self.int.destroy_delay = DESTROY_DELAY;
            // Item.canDestroyBlock: a tool with can_destroy_blocks_in_creative false cannot
            // break in creative. The packet still goes out; only the block change is skipped.
            let can_break = self.can_break_in_creative(env.data);
            // No hit sound or crack: the instabuild branch skips addBreakingBlockEffects.
            self.send_destroy(action::START_DESTROY, h, state, can_break, env, out);
            return (true, can_break);
        }
        if !self.same_destroy_target(h.pos) {
            if self.int.destroying.is_some() {
                // The old break is aborted with the new direction, as startDestroyBlock sends.
                self.abort_destroy(h.face, out);
            }
            let params = self.mining_params(state, env);
            let d = params.progress_per_tick();
            tracing::debug!("start breaking {:?}: {params:?} -> {d} per tick", h.pos);
            let instant = d >= 1.0;
            // Hit sound + crack only when not instant (startDestroyBlock's else branch).
            if !instant {
                self.own_mining_hit(h.pos, h.face, state, env);
            }
            self.send_destroy(action::START_DESTROY, h, state, instant, env, out);
            if instant {
                return (true, true);
            }
            self.int.destroying = Some((h.pos, h.face, self.main_hand().clone()));
            self.int.progress = 0.0;
            self.int.destroy_ticks = 0;
            return (true, false);
        }
        (true, false)
    }

    /// `MultiPlayerGameMode.continueDestroyBlock`: the `destroyDelay` countdown, the creative
    /// instant break, then one tick of progress for the block under the crosshair — stop and
    /// the 5-tick delay at 1, CHANGE_DESTROY_DIRECTION when the same block is hit from another
    /// face, and a restart when the target changed. `true` when the arm swings this tick.
    pub(super) fn continue_destroy(
        &mut self,
        h: BlockHit,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) -> bool {
        if self.int.destroy_delay > 0 {
            self.int.destroy_delay -= 1;
            return true;
        }
        if self.game_mode == GameMode::Creative
            && env.border.is_none_or(|b| b.is_within_bounds(h.pos))
        {
            self.int.destroy_delay = DESTROY_DELAY;
            // continueDestroyBlock does not re-check the tool: the packet goes out every 5
            // ticks, but only a breakable block actually changes (Item.canDestroyBlock).
            let can_break = self.can_break_in_creative(env.data);
            self.send_destroy(
                action::START_DESTROY,
                h,
                world.block_state(h.pos),
                can_break,
                env,
                out,
            );
            return true;
        }
        if self.game_mode == GameMode::Adventure || self.game_mode == GameMode::Spectator {
            return false;
        }
        if !self.same_destroy_target(h.pos) {
            // startDestroyBlock restarts the break and returns its own "handled" — swing and
            // punch only when the block can be acted on at all.
            let (handled, _) = self.start_destroy(h, world, env, out);
            return handled;
        }
        let state = world.block_state(h.pos);
        if env.data.is_air(state) {
            self.int.destroying = None;
            return false;
        }
        self.int.progress += self.mining_params(state, env).progress_per_tick();
        self.int.destroy_ticks += 1;
        if self.int.progress >= 1.0 {
            tracing::debug!(
                "finished breaking {:?} after {} destroy ticks",
                h.pos,
                self.int.destroy_ticks
            );
            self.send_destroy(action::STOP_DESTROY, h, state, true, env, out);
            self.int.destroying = None;
            self.int.progress = 0.0;
            self.int.destroy_delay = DESTROY_DELAY;
        } else if self
            .int
            .destroying
            .as_ref()
            .is_some_and(|(_, face, _)| *face != h.face)
        {
            // Same block, another face under the crosshair (new in 26.3); no sequence, and
            // only when the block did not finish this tick.
            out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                status: action::CHANGE_DESTROY_DIRECTION,
                pos: h.pos,
                face: h.face as u8,
                sequence: 0,
            }));
            self.int.destroying = Some((h.pos, h.face, self.main_hand().clone()));
        }
        true
    }

    /// `MultiPlayerGameMode.stopDestroyBlock`: abort the break (Direction.DOWN, no sequence)
    /// and drop the progress, as vanilla does when the button is released or the crosshair
    /// leaves the block. The attack strength ticker resets with it (vanilla calls
    /// `player.resetAttackStrengthTicker()` here).
    pub(crate) fn stop_destroy(&mut self, out: &mut Vec<ServerboundPlay>) {
        if let Some((_, _, _)) = self.int.destroying {
            self.abort_destroy(Direction::Down, out);
            self.hand.reset_attack_strength_ticker();
        }
    }

    /// `MultiPlayerGameMode.sameDestroyTarget`: same block and same held item
    /// (`ItemStack.isSameItemSameComponents`: item and components, the count ignored).
    fn same_destroy_target(&self, pos: BlockPos) -> bool {
        let held = self.main_hand();
        self.int.destroying.as_ref().is_some_and(|(p, _, item)| {
            *p == pos && item.item == held.item && item.components_raw == held.components_raw
        })
    }

    pub(super) fn abort_destroy(&mut self, face: Direction, out: &mut Vec<ServerboundPlay>) {
        if let Some((pos, _, _)) = self.int.destroying.take() {
            // Abort carries no prediction (sequence 0).
            out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                status: action::ABORT_DESTROY,
                pos,
                face: face as u8,
                sequence: 0,
            }));
            self.int.progress = 0.0;
        }
    }

    /// Mining parameters for a block with the held item.
    pub fn mining_params(&self, state: BlockStateId, env: &InteractEnv<'_>) -> MiningParams {
        let data = env.data;
        let block = data.blocks.block_of(state);
        let index = data.blocks.block_index(block).map_or(u32::MAX, |i| i.0);
        let tags = self.int.tags.as_ref();
        let (tool_speed, tool_correct) = self
            .held_tool(data)
            .map(|t| {
                t.speed_for(|set| {
                    tags.is_some_and(|tg| tg.matches(set, block, index))
                        || matches!(set, haste_physics::mining::BlockSet::Block(b) if b == block)
                        || matches!(set, haste_physics::mining::BlockSet::List(l) if l.contains(block))
                })
            })
            .unwrap_or((1.0, None));
        let level = |id: Option<i32>| {
            id.and_then(|i| self.int.effects.get(&i))
                .map_or(0, |e| (e.amplifier + 1).max(0) as u32)
        };
        let e = self.int.effect_ids;
        let a = self.int.attrs;
        // Vanilla's `speed += getAttributeValue(MINING_EFFICIENCY)`. The server syncs the
        // attribute, but the value comes from the held stack's Efficiency enchantment, which the
        // client also decodes itself; take whichever is present (they agree in real play).
        let stack_efficiency = self
            .int
            .components
            .as_ref()
            .filter(|h| h.stack == *self.main_hand())
            .map_or(0.0, |h| h.efficiency);
        MiningParams {
            hardness: data.destroy_time(state),
            requires_correct_tool: data.requires_correct_tool(state),
            tool_speed,
            tool_correct,
            mining_efficiency: a.mining_efficiency.max(stack_efficiency),
            haste_level: level(e.map(|e| e.haste)).max(level(e.map(|e| e.conduit_power))),
            fatigue_level: level(e.map(|e| e.mining_fatigue)),
            block_break_speed: a.block_break_speed,
            submerged_mining_speed: a.submerged_mining_speed,
            eyes_in_water: self.state.eyes_in_water,
            on_ground: self.state.on_ground,
        }
    }

    /// The held stack's `minecraft:tool` component (`ItemStack.get(DataComponents.TOOL)`) when
    /// the stack overrides it, else the item's default from the data generator's reports.
    fn held_tool<'t>(
        &'t self,
        data: &haste_data::GameData,
    ) -> Option<&'t haste_physics::mining::Tool> {
        let item = self.main_hand();
        if item.is_empty() {
            return None;
        }
        if let Some(h) = self.int.components.as_ref().filter(|h| h.stack == *item) {
            if h.tool.is_some() {
                return h.tool.as_ref();
            }
        }
        let name = data
            .registries
            .name("minecraft:item", item.item.unwrap_or(RegistryIndex(0)))?;
        self.int.tools.as_ref()?.get(name)
    }

    /// `Item.canDestroyBlock` for the main hand in creative: a tool whose
    /// `can_destroy_blocks_in_creative` is false cannot break blocks; anything else can.
    fn can_break_in_creative(&self, data: &haste_data::GameData) -> bool {
        !self
            .held_tool(data)
            .is_some_and(|t| !t.can_destroy_blocks_in_creative)
    }

    /// The mining hit sound of a block we start breaking (vanilla plays it
    /// client-side on the start tick; the block's type comes from the
    /// `block_sounds.json` dump). Unknown blocks play nothing.
    fn own_mining_hit(
        &mut self,
        pos: BlockPos,
        face: Direction,
        state: BlockStateId,
        env: &InteractEnv<'_>,
    ) {
        if let Some(t) = env.data.block_sound_type(state) {
            self.own_sounds.mining_hit(
                pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5),
                t,
                &mut self.sound_out,
            );
        }
        // The start tick's crack particle (vanilla: `addBreakingBlockEffects` with
        // `playSound=true`; the follow-up ticks are the server's 2019/2020 events).
        self.block_particles_out
            .push(crate::block_particles::BlockParticleRequest::Crack { pos, face });
    }

    pub(super) fn send_destroy(
        &mut self,
        status: i32,
        h: BlockHit,
        state: BlockStateId,
        breaks: bool,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let seq = self.int.next_sequence();
        if !self.int.recent.contains(&h.pos) {
            if self.int.recent.len() >= 8 {
                self.int.recent.remove(0);
            }
            self.int.recent.push(h.pos);
        }
        tracing::debug!(
            "player_action {status} {:?} seq {seq} on_ground {} progress {}",
            h.pos,
            self.state.on_ground,
            self.int.progress
        );
        out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
            status,
            pos: h.pos,
            face: h.face as u8,
            sequence: seq,
        }));
        if breaks {
            // Predict the block gone; a waterlogged block leaves water, which we leave to the
            // server's update. The position we stood at is retained with it, for the ack's
            // snap-back when the server's state there collides with us.
            let to = env.data.fluid(state).is_none().then_some(BlockStateId::AIR);
            self.int.predict(seq, h.pos, state, to, self.state.pos);
            // The break sound, played the moment the block goes for us (the server's
            // 2001 broadcast excludes the breaker; its sound half we do not decode).
            if let Some(t) = env.data.block_sound_type(state) {
                self.own_sounds.block_broken(
                    h.pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5),
                    t,
                    &mut self.sound_out,
                );
            }
            // The destroy particles with it (vanilla: `playerWillDestroy` plays the
            // 2001 level event on the client too; the server's broadcast excludes us).
            self.block_particles_out
                .push(crate::block_particles::BlockParticleRequest::Destroy { pos: h.pos, state });
        }
    }
}
