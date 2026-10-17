//! The passenger side of the local player (vanilla `Entity.rideTick` / `LocalPlayer.rideTick`):
//! mounting and dismounting, the seat, and the ticks of the vehicles we steer, which vanilla
//! runs before their passenger (`ClientLevel.tickNonPassenger`).
use super::{command, LocalPlayer, TickCtx, VehiclePose};
use crate::item_based_steering::{ItemSteered, SteerEnv};
use glam::DVec3;
use haste_core::input::InputState;
use haste_core::EntityId;
use haste_entity::riding::SeatPoint;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

impl LocalPlayer {
    /// Mounts or dismounts (`set_passengers`): `seat` is where our feet sit on the vehicle each
    /// tick, in the vehicle's frame (the vehicle's passenger attachment point minus our own,
    /// `haste_entity::passenger_seat`), and `boat`/`horse`/`item_steered` the vehicle's client-side
    /// simulation when it is a vehicle we steer (`None` for any other vehicle, and on a
    /// dismount). On a dismount, normal physics resumes from where the player is.
    pub fn set_vehicle(
        &mut self,
        vehicle: Option<EntityId>,
        seat: SeatPoint,
        boat: Option<crate::boat::Boat>,
        horse: Option<crate::horse::Horse>,
        item_steered: Option<ItemSteered>,
    ) {
        if vehicle.is_some() {
            self.seat = seat;
        }
        self.vehicle = vehicle;
        self.boat = boat;
        self.item_steered = item_steered;
        let has_horse = horse.is_some();
        self.horse = horse;
        if !has_horse {
            // Vanilla's charge state resets with the vehicle (a new mount starts uncharged).
            self.jump_riding_ticks = 0;
            self.jump_riding_scale = 0.0;
            self.riding_jump_held = false;
        }
    }

    /// The vehicle we ride, if any.
    pub fn vehicle(&self) -> Option<EntityId> {
        self.vehicle
    }

    /// The boat we steer, if any (its simulated pose drives the seat and the `move_vehicle`
    /// packets; for tests).
    pub fn boat(&self) -> Option<&crate::boat::Boat> {
        self.boat.as_ref()
    }

    /// The boat we steer, for the packets that steer it (the server's corrections).
    pub fn boat_mut(&mut self) -> Option<&mut crate::boat::Boat> {
        self.boat.as_mut()
    }

    /// The horse-family vehicle we steer, if any (its simulated pose drives the seat and the
    /// `move_vehicle` packets; for tests).
    pub fn horse(&self) -> Option<&crate::horse::Horse> {
        self.horse.as_ref()
    }

    /// The horse we steer, for the packets that steer it (the server's corrections).
    pub fn horse_mut(&mut self) -> Option<&mut crate::horse::Horse> {
        self.horse.as_mut()
    }

    /// The pig we ride, if any (for tests).
    pub fn pig(&self) -> Option<&crate::pig::Pig> {
        match &self.item_steered {
            Some(ItemSteered::Pig(p)) => Some(p),
            _ => None,
        }
    }

    /// The strider we ride, if any (for tests).
    pub fn strider(&self) -> Option<&crate::strider::Strider> {
        match &self.item_steered {
            Some(ItemSteered::Strider(s)) => Some(s),
            _ => None,
        }
    }

    /// The pig or strider we ride, for the packets about it (the boost's metadata sync).
    pub fn item_steered_mut(&mut self) -> Option<&mut ItemSteered> {
        self.item_steered.as_mut()
    }

    /// The pig or strider we ride while we control it: vanilla `getControllingPassenger` is the
    /// rider only while it holds the animal's stick in either hand. Only then is our
    /// simulation the truth (`isLocalInstanceAuthoritative`): it drives the seat, the
    /// `move_vehicle` packets and the replies to the server's corrections.
    pub fn steered_by_item(&self) -> Option<&ItemSteered> {
        self.item_steered
            .as_ref()
            .filter(|a| self.holds_steer_stick(a.stick()))
    }

    /// [`Self::steered_by_item`], mutably.
    pub fn steered_by_item_mut(&mut self) -> Option<&mut ItemSteered> {
        let held = self
            .item_steered
            .as_ref()
            .is_some_and(|a| self.holds_steer_stick(a.stick()));
        self.item_steered.as_mut().filter(|_| held)
    }

    /// The mount's charged jump scale (vanilla `LocalPlayer.getJumpRidingScale`): what the HUD
    /// jump bar shows while we hold jump on a horse-family vehicle.
    pub fn jump_riding_scale(&self) -> f32 {
        self.jump_riding_scale
    }

    /// The seat we ride in while mounted, in the vehicle's own frame (for tests).
    pub fn seat(&self) -> SeatPoint {
        self.seat
    }

    /// The passenger tick, vanilla's `rideTick`: a separate seam next to the movement tick, not
    /// a branch inside it. The passenger's own physics does not run; its position is the
    /// vehicle's position plus the seat offset turned by the vehicle's yaw (vanilla's
    /// `Entity.positionRider`), and only its look is free (`look` is the player's look for this
    /// tick, look offset already applied). The boat we steer moves under us: the pose comes from
    /// its own simulation, not from the packets, and `input` hands it the held keys vanilla's
    /// `LocalPlayer.rideTick` does. `vehicle` is `None` when the vehicle is not tracked
    /// (removed, or its spawn not yet arrived): the passenger keeps its last position.
    pub fn ride_tick(
        &mut self,
        input: &InputState,
        look: (f32, f32),
        vehicle: Option<VehiclePose>,
    ) {
        // vanilla `LocalPlayer.rideTick`: the passenger is a boat's rudder.
        let posed = if let Some(boat) = &mut self.boat {
            boat.set_input(crate::boat::BoatInput::from_input(input));
            Some(VehiclePose {
                pos: boat.pos,
                yaw: boat.yaw,
            })
        } else if let Some(horse) = &self.horse {
            Some(VehiclePose {
                pos: horse.pos,
                yaw: horse.yaw,
            })
        } else if let Some(animal) = self.steered_by_item() {
            let (pos, yaw, _, _) = animal.pose();
            Some(VehiclePose { pos, yaw })
        } else {
            vehicle
        };
        if let Some(v) = posed {
            self.state.pos = v.pos + self.seat.rotated(v.yaw);
        }
        self.state.velocity = DVec3::ZERO;
        self.state.fall_distance = 0.0;
        self.state.on_ground = false;
        self.state.horizontal_collision = false;
        // vanilla `AbstractBoat.positionRider`: a boat drags our look with its turn and keeps
        // it within 105 degrees of its own heading; any other vehicle leaves the look alone.
        self.state.yaw = match &self.boat {
            Some(boat) => crate::boat::passenger_yaw(look.0, boat),
            None => look.0,
        };
        self.state.pitch = look.1;
    }

    /// The boat's own tick, which vanilla runs before its passenger's (`ClientLevel`
    /// .tickNonPassenger` ticks the vehicle, then `tickPassenger` the passenger): the status,
    /// the buoyancy and drag, the paddles and the swept move, then the `paddle_boat` packet the
    /// client's own instance sends every tick.
    pub(super) fn boat_tick(&mut self, ctx: &TickCtx<'_>, out: &mut Vec<ServerboundPlay>) {
        let Some(boat) = &mut self.boat else {
            return;
        };
        let env = crate::boat::BoatEnv {
            world: ctx.world,
            shapes: ctx.shapes,
            blocks: ctx.blocks,
        };
        boat.tick(&env);
        out.push(ServerboundPlay::PaddleBoat(sb::PaddleBoat {
            left: boat.paddle(0),
            right: boat.paddle(1),
        }));
    }

    /// The horse's own tick and the rider's charged jump (vanilla runs the vehicle before its
    /// passenger, and the charge lives in the passenger's `LocalPlayer.aiStep` — after the
    /// horse's tick, so the horse reads the charge and the input one tick late). The release
    /// hands the horse its charge and sends the `player_command` START_RIDING_JUMP vanilla's
    /// `sendRidingJump` does.
    pub(super) fn horse_tick(
        &mut self,
        input: &InputState,
        look: (f32, f32),
        ctx: &TickCtx<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let Some(horse) = &mut self.horse else {
            return;
        };
        let env = crate::horse::HorseEnv {
            world: ctx.world,
            shapes: ctx.shapes,
            blocks: ctx.blocks,
        };
        horse.tick(&env);
        // The charged jump (vanilla `LocalPlayer.aiStep`, the `jumpableVehicle` branch; the
        // horse family has no jump cooldown). A negative counter is the ten-tick lockout after
        // a release; while it counts up to zero the scale stays empty.
        let held = input.jump;
        let was_held = self.riding_jump_held;
        self.riding_jump_held = held;
        if self.jump_riding_ticks < 0 {
            self.jump_riding_ticks += 1;
            if self.jump_riding_ticks == 0 {
                self.jump_riding_scale = 0.0;
            }
        }
        if was_held && !held {
            self.jump_riding_ticks = -10;
            let amount = crate::horse::charge_amount(self.jump_riding_scale);
            horse.on_player_jump(amount);
            out.push(ServerboundPlay::PlayerCommand(sb::PlayerCommand {
                entity_id: self.entity_id.0,
                action: command::START_RIDING_JUMP,
                jump_boost: amount,
            }));
        } else if !was_held && held {
            self.jump_riding_ticks = 0;
            self.jump_riding_scale = 0.0;
        } else if was_held {
            self.jump_riding_ticks += 1;
            self.jump_riding_scale = crate::horse::charge_scale(self.jump_riding_ticks);
        }
        // This tick's input and look are what the horse's next tick reads.
        let (strafe, forward) = crate::horse::rider_input(input);
        horse.set_input(strafe, forward, look);
    }

    /// The pig's or strider's own tick (vanilla runs the vehicle before its passenger). It
    /// walks full forward on its own while the rider holds its stick (the
    /// `getControllingPassenger` condition); without the stick it is not ours (vanilla
    /// interpolates it from the server's moves), so our simulation follows the server's pose
    /// and resumes from there. The look arrives one tick late, like the horse's input.
    pub(super) fn item_steered_tick(
        &mut self,
        look: (f32, f32),
        vehicle: Option<VehiclePose>,
        ctx: &TickCtx<'_>,
    ) {
        let env = SteerEnv {
            world: ctx.world,
            shapes: ctx.shapes,
            blocks: ctx.blocks,
        };
        let controlled = self.steered_by_item().is_some();
        let Some(animal) = &mut self.item_steered else {
            return;
        };
        if controlled {
            animal.tick(&env);
        } else if let Some(v) = vehicle {
            animal.follow(v.pos, v.yaw);
        }
        animal.set_look(look);
    }
}
