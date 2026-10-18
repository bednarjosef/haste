// The session's end-of-tick step and its read surface for the game thread: the local player's
// tick with the input (its packets, its predicted block edits), the lid animation steps, the
// screen effects' tick, and the accessors the renderer and the HUD read out between ticks.

use haste_core::input::InputState;
use haste_protocol::haste_core::BlockPos;
use haste_protocol::ServerboundPlay;

use super::sinks::{NoBlocks, StriderWarm};
use super::{EntitySink, Session, WorldSink};
use crate::player::VehiclePose;

impl Session {
    /// The local player's tick with `input` (and the interactions set with
    /// `LocalPlayer::set_interact`) and its packets; predicted block changes go into the world.
    /// In a world without physics data (test fakes) the player stays in place.
    pub fn end_tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &mut W,
        entities: &E,
        out: &mut Vec<ServerboundPlay>,
    ) {
        // Sound requests nobody drained (headless) are dropped here; the game thread
        // drains them between `receive` and `end_tick`.
        self.sound_requests.clear();
        // The player's own block sounds this tick (footsteps, mining, placing): played
        // through the same request channel the packets use, so the game thread's
        // `Sounds::apply` picks them up; headless only counts them.
        for request in self.player.take_own_sound_requests() {
            self.counters.own_sounds += 1;
            self.sound_requests.push(request);
        }
        // The player's own block particles this tick (break grid, mining cracks).
        self.block_particle_requests
            .extend(self.player.take_own_block_particles());
        self.entity_boxes.clear();
        self.entity_types.clear();
        for id in &self.entity_ids {
            if let Some(b) = entities.hitbox(*id) {
                self.entity_boxes.push((*id, b));
                // Unknown (fakes, mod types) -> sentinel, which the picker treats as pickable.
                self.entity_types.push(
                    entities
                        .entity_type(*id)
                        .unwrap_or(haste_core::RegistryIndex(u32::MAX)),
                );
            }
        }
        // The vehicle's position this tick, for the passenger tick (`ride_tick`). A vehicle we
        // do not track (removed before its set_passengers, spawn not yet arrived) leaves the
        // passenger at its last position. The yaw comes with it: the seat's sideways offset
        // turns with the vehicle (`Entity.positionRider` runs every tick).
        let vehicle = self
            .player
            .vehicle
            .and_then(|v| entities.pose(v))
            .map(|(pos, yaw, _)| VehiclePose { pos, yaw });
        // The attached firework rocket's push, vanilla `FireworkRocketEntity.tick`: while the
        // rocket is attached to us and we are fall flying, each tick our velocity becomes
        // `v + look * 0.1 + (look * 1.5 - v) * 0.5`. The rocket ticked before the player in the
        // client's own order (`NetDriver::end_tick` ticks entities first), so the push lands
        // before this tick's glide update.
        if self.attached_firework.is_some() && self.player.state.fall_flying {
            let s = &mut self.player.state;
            s.velocity = haste_physics::elytra::firework_boost(s.velocity, s.yaw, s.pitch);
        }
        match world.physics() {
            Some((view, data)) => {
                self.player.prepare(data);
                let warm_tag = crate::item_based_steering::strider_warm_blocks();
                let warm = self
                    .block_tags
                    .as_ref()
                    .and_then(|tags| tags.members(&warm_tag))
                    .map(|warm| StriderWarm { data, warm });
                let env = crate::player::interact::InteractEnv {
                    data,
                    entities: &self.entity_boxes,
                    entity_types: &self.entity_types,
                    warm_blocks: warm
                        .as_ref()
                        .map(|w| w as &dyn crate::item_based_steering::WarmBlocks),
                };
                self.player.tick_interact(input, view, &env, vehicle, out)
            }
            None => self.player.tick_with(
                input,
                &NoBlocks,
                &NoBlocks,
                &haste_physics::DefaultBlockPhysics,
                vehicle,
                out,
            ),
        }
        for (pos, state) in self.player.take_world_edits() {
            world.set_block(pos, state);
            self.lids.remove(&pos);
            self.bells.remove(&pos);
            self.pots.remove(&pos);
        }
        // Every lid's animation step (vanilla ticks each chest's `ChestLidController` and each
        // shulker box's `updateAnimation` once per client tick).
        for lid in self.lids.values_mut() {
            lid.tick();
        }
        // The screen effects' tick, in vanilla's `LocalPlayer.tick` order: the portal
        // transition effect (`portalProcess.isInsidePortalThisTick`, set by the game thread's
        // world probe through [`Session::screen`]) and the spinning effect, which read the
        // nausea blend factor at the end of the tick.
        let nausea = self.player.int.nausea_factor(1.0);
        self.screen.tick(true, nausea);
    }

    /// The item worn on the head (the helmet armor slot), if any.
    pub fn head_item(&self) -> Option<haste_core::RegistryIndex> {
        self.player.int.armor[0].item
    }

    /// Our own entity's screen-effect bits (on fire, freeze ticks), parsed by `LocalPlayer`.
    pub fn own_flags(&self) -> &crate::screen_effects::OwnFlags {
        &self.player.own_flags
    }

    /// The portal and spinning screen effects, ticked in `end_tick`.
    pub fn screen_effects(&self) -> &crate::screen_effects::PortalEffects {
        &self.screen
    }

    /// Break cracks to draw: other players' (`block_destruction`) and our own.
    pub fn breaking_stages(&self, out: &mut Vec<(BlockPos, u8)>) {
        out.extend(self.breaking.values().copied());
        out.extend(self.player.break_progress());
    }

    /// Lid animations to draw this frame, as (position, value at the last tick's start and
    /// end) pairs for the partial-tick lerp. Only open or animating lids are listed (a closed
    /// chest at rest draws the same as no entry).
    pub fn lid_states(&self, out: &mut Vec<(BlockPos, (f32, f32))>) {
        out.extend(
            self.lids
                .iter()
                .filter(|(_, l)| !l.at_rest_closed())
                .map(|(pos, l)| (*pos, (l.old, l.value))),
        );
    }
}
