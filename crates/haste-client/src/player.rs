//! The local player (player workstream): owns the physics state, consumes the packets about the
//! local player and produces the serverbound packets a vanilla client sends for each tick.
//!
//! Per tick, in this order (the order a vanilla client is observed to send them in):
//! 1. `player_input` when the held movement keys changed since the last one sent,
//! 2. physics (`haste_physics::tick`),
//! 3. `player_abilities` when flight was toggled or ended by landing,
//! 4. `player_command` start/stop sprinting when the sprint state changed,
//! 5. one of `move_player_pos_rot` / `_pos` / `_rot` / `_status_only` (send rules below),
//! 6. `client_tick_end`.
//!
//! Movement send rules (wiki "Java Edition protocol", "Set Player Position"; checked against the
//! vanilla server's movement checks): the position is sent when it moved more than 2.0E-4 blocks
//! since the last position sent, or when 20 ticks passed since then; rotation is sent when it
//! changed; with neither, a status-only packet is sent when on-ground or horizontal collision
//! changed. Nothing is sent before the client has reported `player_loaded`.
//!
//! Look angles: the input layer owns absolute yaw/pitch (mouse); the server can rotate the player
//! (`player_position`, `player_rotation`). Those rotations are kept as an offset added to the
//! input's angles ([`LocalPlayer::look_offset`]), so the input layer never has to be told.
// The windowed client (client-app) uses the parts the headless driver does not.
#![allow(dead_code)]

use crate::hand;
use glam::DVec3;
use haste_core::game::GameMode;
use haste_core::input::InputState;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{BlockPos, EntityId, Identifier};
use haste_entity::riding::SeatPoint;
use haste_physics::movement::{BLOCK_BELOW_OFFSET, SPRINT_JUMP_BOOST, VERTICAL_DRAG};
use haste_physics::{PlayerAttributes, PlayerState};
use haste_protocol::packets::play::clientbound::{self as cb, teleport_flags};
use haste_protocol::packets::play::serverbound::{self as sb, input_flags, move_flags};
use haste_protocol::{ClientboundPlay, Login, ServerboundPlay};

/// A position change is sent when its squared length exceeds this (2.0E-4 blocks).
pub const POSITION_THRESHOLD_SQ: f64 = 2.0e-4 * 2.0e-4;
/// The position is re-sent at least every this many ticks even without movement.
pub const POSITION_REMINDER_TICKS: u32 = 20;
/// Food level at or below which the player cannot sprint (wiki "Sprinting"; not in creative).
pub const SPRINT_FOOD_LEVEL: i32 = 6;
/// Ticks a double tap of jump stays armed: vanilla `LocalPlayer` sets its `jumpTriggerTime` to 7
/// on the first press and `Player.aiStep` counts it down, so a second press toggles creative
/// flight while it is still above zero (the second press lands up to 6 ticks after the first).
pub const JUMP_DOUBLE_TAP_TICKS: u8 = 7;
/// Ticks a double tap of forward stays armed: vanilla `LocalPlayer.sprintTriggerTime`, set to
/// `options.sprintWindow` (default 7, `Options.java`) on the first forward press with no
/// impulse and counted down each `aiStep`, so a second press inside the window starts sprinting.
pub const SPRINT_DOUBLE_TAP_TICKS: u8 = 7;
/// Vertical movement while the chunk below is not loaded: vanilla `LivingEntity.travelInAir`
/// skips gravity on the client and pins the vertical movement to -0.1 (0.0 at or below the
/// world's minimum), with the 0.98 vertical drag applied on top.
pub const UNLOADED_CHUNK_FALL_SPEED: f64 = -0.1 * VERTICAL_DRAG as f64;

/// `player_command` actions (wiki "Player Command"; ordinals from the 26.3
/// `ServerboundPlayerCommandPacket.Action` enum: STOP_SLEEPING 0, START_SPRINTING 1,
/// STOP_SPRINTING 2, START_RIDING_JUMP 3, STOP_RIDING_JUMP 4, OPEN_INVENTORY 5,
/// START_FALL_FLYING 6).
pub mod command {
    pub const START_SPRINTING: i32 = 1;
    pub const STOP_SPRINTING: i32 = 2;
    pub const START_FALL_FLYING: i32 = 6;
}

/// `player_abilities` flag bits (clientbound and serverbound).
pub mod ability_flags {
    pub const INVULNERABLE: i8 = 0x01;
    pub const FLYING: i8 = 0x02;
    pub const MAY_FLY: i8 = 0x04;
    pub const INSTABUILD: i8 = 0x08;
}

/// The vehicle we ride this tick: its position, and its yaw, which turns the seat's sideways
/// and forward offsets as the vehicle turns under us (`Entity.positionRider`).
#[derive(Clone, Copy)]
pub struct VehiclePose {
    pub pos: DVec3,
    pub yaw: f32,
}

/// What `player_abilities` last said.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Abilities {
    pub invulnerable: bool,
    pub may_fly: bool,
    pub instabuild: bool,
    /// Creative flying speed (0.05 by default).
    pub flying_speed: f32,
    /// FOV modifier ("walking speed" in older protocol docs; 0.1 by default).
    pub fov_modifier: f32,
}

impl Abilities {
    fn for_mode(mode: GameMode) -> Self {
        let creative = mode == GameMode::Creative;
        Self {
            invulnerable: creative || mode == GameMode::Spectator,
            may_fly: creative || mode == GameMode::Spectator,
            instabuild: creative,
            flying_speed: 0.05,
            fov_modifier: 0.1,
        }
    }
}

/// `minecraft:attribute` registry indices of the attributes the player tick uses. Looked up by
/// name in the data generator's registries (never hard-coded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributeIds {
    pub movement_speed: i32,
    pub jump_strength: i32,
    pub gravity: i32,
    pub step_height: i32,
    pub scale: i32,
    pub sneaking_speed: i32,
    pub block_break_speed: i32,
    pub mining_efficiency: i32,
    pub submerged_mining_speed: i32,
    pub block_interaction_range: i32,
    pub entity_interaction_range: i32,
    /// `attack_speed`: how fast the equip progress returns after an item switch
    /// (`Player.getCurrentItemAttackStrengthDelay`).
    pub attack_speed: i32,
}

impl AttributeIds {
    /// Resolves the ids with `index(name)`, e.g. a registry lookup; `None` if one is missing.
    pub fn resolve(index: impl Fn(&str) -> Option<i32>) -> Option<Self> {
        Some(Self {
            movement_speed: index("movement_speed")?,
            jump_strength: index("jump_strength")?,
            gravity: index("gravity")?,
            step_height: index("step_height")?,
            scale: index("scale")?,
            sneaking_speed: index("sneaking_speed")?,
            block_break_speed: index("block_break_speed")?,
            mining_efficiency: index("mining_efficiency")?,
            submerged_mining_speed: index("submerged_mining_speed")?,
            block_interaction_range: index("block_interaction_range")?,
            entity_interaction_range: index("entity_interaction_range")?,
            attack_speed: index("attack_speed")?,
        })
    }

    /// From `GameData`'s `minecraft:attribute` registry.
    pub fn from_data(data: &haste_data::GameData) -> Option<Self> {
        Self::resolve(|name| {
            data.registries
                .index("minecraft:attribute", &Identifier::minecraft(name))
                .map(|i| i.0 as i32)
        })
    }
}

/// The sprint modifier the server includes in `movement_speed` while we sprint. Physics applies
/// sprinting itself, so it is left out when computing the attribute.
const SPRINTING_MODIFIER: &str = "sprinting";

/// Attribute value from base and modifiers (wiki "Attribute", "Modifiers": add_value, then
/// add_multiplied_base, then add_multiplied_total).
pub fn attribute_value(p: &cb::AttributeProperty, skip: impl Fn(&Identifier) -> bool) -> f64 {
    let mods = || p.modifiers.iter().filter(|m| !skip(&m.id));
    let mut base = p.value;
    for m in mods().filter(|m| m.operation == 0) {
        base += m.amount;
    }
    let mut v = base;
    for m in mods().filter(|m| m.operation == 1) {
        v += base * m.amount;
    }
    for m in mods().filter(|m| m.operation == 2) {
        v *= 1.0 + m.amount;
    }
    v
}

/// What was last sent in a movement packet.
#[derive(Clone, Copy, Debug, Default)]
struct Sent {
    pos: DVec3,
    yaw: f32,
    pitch: f32,
    on_ground: bool,
    horizontal_collision: bool,
    /// Ticks since the position was last sent.
    reminder: u32,
    sprinting: bool,
    input: Option<u8>,
}

/// The local player.
pub struct LocalPlayer {
    pub state: PlayerState,
    pub attrs: PlayerAttributes,
    pub entity_id: EntityId,
    pub game_mode: GameMode,
    pub abilities: Abilities,
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    /// Air supply in ticks (entity metadata index 1; 300 when not under water).
    pub air_supply: i32,
    /// Our own entity's screen-effect bits from our own `set_entity_data`: the on-fire shared
    /// flag (index 0) and the freeze ticks (index 7).
    pub own_flags: crate::screen_effects::OwnFlags,
    /// Selected hotbar slot (0..8).
    pub held_slot: i32,
    /// Whether a `player_position` arrived since login/respawn.
    pub has_position: bool,
    /// Whether `player_loaded` was sent since login/respawn (movement is sent only after).
    pub loaded: bool,
    /// The vehicle the server has us riding (`set_passengers`); `None` when on foot. While set,
    /// the player's own physics does not run: each tick its position is the vehicle's position
    /// plus the seat offset ([`LocalPlayer::ride_tick`]).
    pub vehicle: Option<EntityId>,
    /// The boat we steer ([`crate::boat`]), set on mount together with `vehicle` and cleared on
    /// any dismount: it holds the client's own authoritative simulation of a vehicle whose
    /// controlling passenger we are (vanilla `Entity.isLocalInstanceAuthoritative`).
    boat: Option<crate::boat::Boat>,
    /// Where our feet sit on the vehicle while riding (set on mount), in the vehicle's own
    /// frame: the vehicle's passenger attachment point minus our own vehicle attachment point
    /// (`haste_entity::passenger_seat`).
    seat: SeatPoint,
    /// Teleport id awaiting its `accept_teleportation`; the accept waits one full tick
    /// after the teleport arrived (see [`LocalPlayer::flush_pending_accept`]). The server
    /// queues further teleports until the accept, so one slot is enough.
    pending_accept: Option<i32>,
    /// Ticks the pending accept has been waiting (one full tick, not just until the next
    /// `tick_inner`: the driver drains packets and ends the tick in the same call).
    pending_accept_age: u32,
    /// Counters for tests and the headless printout.
    pub teleports: u64,
    prev_pos: DVec3,
    /// The camera's smoothed eye height and its last-tick value (vanilla `Camera.eyeHeight`/
    /// `eyeHeightOld`, f32 like vanilla). The camera, not the player, smooths: each tick it
    /// halves the distance to the pose's eye height, and the render lerps the two values.
    camera_eye: f32,
    camera_eye_old: f32,
    look_offset: (f32, f32),
    last_input_look: (f32, f32),
    /// The previous tick's forward impulse (`forward && !back`), for the sprint double tap's
    /// rising edge (vanilla reads `hasForwardImpulse` before `input.tick()`).
    last_input_forward: bool,
    sent: Sent,
    attribute_ids: Option<AttributeIds>,
    /// Last `update_attributes` value per attribute id (base + modifiers).
    attributes: Vec<cb::AttributeProperty>,
    attributes_dirty: bool,
    /// Set by the input layer when it saw a double tap of jump (the windowed client has its own
    /// wall-clock detector); the tick-accurate window is [`LocalPlayer::jump_trigger_time`].
    toggle_flight: bool,
    /// The local player's own block sounds (client-side rules, sound.rs).
    pub(crate) own_sounds: crate::sound::OwnBlockSounds,
    /// Own-sound requests this tick, drained into `Session::sound_requests` by `end_tick`.
    pub(crate) sound_out: Vec<crate::sound::SoundRequest>,
    /// Own block particles this tick (our break prediction's destroy grid, the
    /// mining start tick's crack), drained into `Session::block_particle_requests`.
    pub(crate) block_particles_out: Vec<crate::block_particles::BlockParticleRequest>,
    /// Ticks left in the double-tap jump window: vanilla `LocalPlayer.jumpTriggerTime` (7 on the
    /// first press, counted down by `Player.aiStep` before the keys are looked at).
    jump_trigger_time: u8,
    /// Ticks left in the double-tap sprint window: vanilla `LocalPlayer.sprintTriggerTime`
    /// (`options.sprintWindow`, 7 on the first forward press with no impulse, counted down by
    /// `aiStep` before the keys are looked at).
    sprint_trigger_time: u8,
    /// Jump key state of the previous tick, so a fresh press is a rising edge (vanilla reads
    /// `input.keyPresses.jump()` before `input.tick()`).
    jump_held: bool,
    /// Vanilla `LocalPlayer.waterVisionTime` (0..600): ramps up underwater and back down out of
    /// it; `get_water_vision` turns it into the underwater fog visibility (`getWaterVision`).
    water_vision_time: i32,
    /// Breaking, placing, using, attacking, hotbar and inventory (player/interact.rs).
    pub(crate) int: interact::Interaction,
    /// The first-person hand's per-tick animation state (player/hand.rs): swing, equip
    /// progress and bob, filled for the render state at the end of each tick.
    pub hand: hand::Hand,
}

impl LocalPlayer {
    pub fn new(login: &Login) -> LocalPlayer {
        Self::with_entity(login.entity_id, login.game_mode)
    }

    /// A player with our entity id and initial game mode (what `new` takes from `Login`).
    pub fn with_entity(entity_id: EntityId, game_mode: GameMode) -> LocalPlayer {
        let mut p = LocalPlayer {
            state: PlayerState::default(),
            attrs: PlayerAttributes::default(),
            entity_id,
            game_mode,
            abilities: Abilities::for_mode(game_mode),
            health: 20.0,
            food: 20,
            saturation: 5.0,
            air_supply: 300,
            own_flags: crate::screen_effects::OwnFlags::default(),
            held_slot: 0,
            has_position: false,
            loaded: false,
            vehicle: None,
            boat: None,
            seat: SeatPoint::ZERO,
            pending_accept: None,
            pending_accept_age: 0,
            teleports: 0,
            prev_pos: DVec3::ZERO,
            camera_eye: 0.0,
            camera_eye_old: 0.0,
            look_offset: (0.0, 0.0),
            last_input_look: (0.0, 0.0),
            last_input_forward: false,
            sent: Sent::default(),
            attribute_ids: None,
            attributes: Vec::new(),
            attributes_dirty: false,
            toggle_flight: false,
            own_sounds: crate::sound::OwnBlockSounds::new(),
            sound_out: Vec::new(),
            block_particles_out: Vec::new(),
            jump_trigger_time: 0,
            sprint_trigger_time: 0,
            jump_held: false,
            water_vision_time: 0,
            int: interact::Interaction::default(),
            hand: hand::Hand::new(),
        };
        p.state.on_ground = true;
        // Vanilla's camera starts its eye height at 0 and rises on the first level tick; here
        // the first position has not arrived yet, so the pair starts at the standing eye height
        // and the smoothing takes over from there (the rise would otherwise show before the
        // player even exists).
        let eye = p.eye_height() as f32;
        p.camera_eye = eye;
        p.camera_eye_old = eye;
        p
    }

    /// Server rotations as an offset to add to the input layer's yaw/pitch (degrees).
    pub fn look_offset(&self) -> (f32, f32) {
        self.look_offset
    }

    /// Own-sound requests accumulated this tick (footsteps, mining, placing),
    /// for the caller to drain into its sound output or headless counters.
    pub fn take_own_sound_requests(&mut self) -> std::vec::Drain<'_, crate::sound::SoundRequest> {
        self.sound_out.drain(..)
    }

    /// Own block particles accumulated this tick, for the caller to drain into the
    /// session's request queue.
    pub fn take_own_block_particles(
        &mut self,
    ) -> std::vec::Drain<'_, crate::block_particles::BlockParticleRequest> {
        self.block_particles_out.drain(..)
    }

    /// The input layer saw a double tap of jump (creative/spectator). Vanilla keeps the window
    /// itself (`LocalPlayer.jumpTriggerTime`, [`JUMP_DOUBLE_TAP_TICKS`] ticks): the first press
    /// arms it, the second one inside it toggles flight and jumps when flight turns on while the
    /// player stands on the ground. Both are checked in [`LocalPlayer::tick_inner`].
    pub fn toggle_flight(&mut self) {
        self.toggle_flight = true;
    }

    /// Whether the attribute registry ids are known.
    pub fn has_attribute_ids(&self) -> bool {
        self.attribute_ids.is_some()
    }

    /// Sets the attribute registry ids (done automatically by [`LocalPlayer::tick`]).
    pub fn set_attribute_ids(&mut self, ids: AttributeIds) {
        self.attribute_ids = Some(ids);
        self.attributes_dirty = true;
    }

    /// Eye height for the current pose (the camera sits this far above the feet).
    pub fn eye_height(&self) -> f64 {
        self.attrs.eye_height_for(self.state.pose)
    }

    /// Vanilla `LocalPlayer.getWaterVision`: 0 out of water, ramping to 1 over 30 s underwater
    /// (a 0.6 ramp over the first 5 s and 0.4 more over the next 25 s).
    pub fn get_water_vision(&self) -> f32 {
        if !self.state.eyes_in_water {
            return 0.0;
        }
        let t = self.water_vision_time as f32;
        if t >= 600.0 {
            return 1.0;
        }
        let a = (t / 100.0).clamp(0.0, 1.0);
        let b = if t < 100.0 {
            0.0
        } else {
            ((t - 100.0) / 500.0).clamp(0.0, 1.0)
        };
        a * 0.6 + b * 0.39999998
    }

    /// Eye position and look angles interpolated by partial tick, for the camera. The eye
    /// height is the camera's own smoothed pair (vanilla `Camera.setup` lerps
    /// `eyeHeightOld`/`eyeHeight`, not the entity's eye height).
    pub fn camera(&self, partial_tick: f32) -> (DVec3, f32, f32) {
        let t = partial_tick.clamp(0.0, 1.0);
        let pos = self.prev_pos.lerp(self.state.pos, t as f64);
        let eye = self.camera_eye_old + (self.camera_eye - self.camera_eye_old) * t;
        (
            pos + DVec3::new(0.0, f64::from(eye), 0.0),
            self.state.yaw,
            self.state.pitch,
        )
    }

    /// Consumes the packets about the local player. Returns false for packets it does not handle.
    /// The `out` buffer is kept for the caller's uniform signature; this handler defers every
    /// serverbound reply to the next tick (see [`LocalPlayer::flush_pending_accept`]).
    pub fn handle(&mut self, p: &ClientboundPlay, _out: &mut Vec<ServerboundPlay>) -> bool {
        use ClientboundPlay as P;
        match p {
            P::PlayerPosition(t) => {
                self.teleport(t);
                // The accept is not sent from the network reader (immediately, mid-tick): a
                // join teleport answered within microseconds of arrival races the server's
                // own join bookkeeping, which logs "moved too quickly! <spawn position>"
                // (the delta it computed against a not-yet-set position). A vanilla client
                // answers from its game loop; the accept waits one full tick
                // (flush_pending_accept).
                self.pending_accept = Some(t.teleport_id);
                self.pending_accept_age = 0;
                self.mark_sent_position(self.state.on_ground, self.state.horizontal_collision);
            }
            P::PlayerRotation(r) => {
                let yaw = if r.relative_yaw {
                    self.state.yaw + r.yaw
                } else {
                    r.yaw
                };
                let pitch = if r.relative_pitch {
                    self.state.pitch + r.pitch
                } else {
                    r.pitch
                };
                self.set_look(yaw, pitch);
            }
            P::PlayerAbilities(a) => {
                let f = a.flags;
                self.abilities = Abilities {
                    invulnerable: f & ability_flags::INVULNERABLE != 0,
                    may_fly: f & ability_flags::MAY_FLY != 0,
                    instabuild: f & ability_flags::INSTABUILD != 0,
                    flying_speed: a.flying_speed,
                    fov_modifier: a.fov_modifier,
                };
                self.state.flying = f & ability_flags::FLYING != 0;
                self.attrs.flying_speed = a.flying_speed;
            }
            P::UpdateAttributes(u) if EntityId(u.entity_id) == self.entity_id => {
                for prop in &u.properties {
                    match self.attributes.iter_mut().find(|a| a.id == prop.id) {
                        Some(a) => *a = prop.clone(),
                        None => self.attributes.push(prop.clone()),
                    }
                }
                self.attributes_dirty = true;
            }
            P::SetEntityMotion(m) if EntityId(m.entity_id) == self.entity_id => {
                self.state.velocity = m.velocity;
            }
            P::SetHealth(h) => {
                self.health = h.health;
                self.food = h.food;
                self.saturation = h.saturation;
            }
            P::SetHeldSlot(s) => self.held_slot = s.slot,
            P::SetEntityData(d) if EntityId(d.entity_id) == self.entity_id => {
                // Wiki "Entity metadata": index 0 is the shared flags byte (bit 0 = on fire,
                // `Entity.isOnFire`; bit 0x80 = fall flying, `Entity.FLAG_FALL_FLYING` = 7),
                // index 1 the air supply (VarInt), index 7 `DATA_TICKS_FROZEN` (the
                // powder-snow outline's alpha).
                for e in &d.metadata {
                    match (e.index, &e.value) {
                        (1, haste_core::entity::MetadataValue::VarInt(v)) => self.air_supply = *v,
                        (0, haste_core::entity::MetadataValue::Byte(b)) => {
                            self.own_flags.on_fire = *b & 0x01 != 0;
                            // The server owns the flag: it confirms a glide we started and
                            // clears it when the glide ends (landing, no glider).
                            self.state.fall_flying = *b as u8 & 0x80 != 0;
                        }
                        (7, haste_core::entity::MetadataValue::VarInt(t)) => {
                            self.own_flags.frozen_ticks = *t;
                        }
                        _ => {}
                    }
                }
            }
            P::GameEvent(g) if g.event == 3 => {
                if let Some(m) = GameMode::from_id(g.value as i32) {
                    self.game_mode = m;
                    self.abilities = Abilities {
                        flying_speed: self.abilities.flying_speed,
                        fov_modifier: self.abilities.fov_modifier,
                        ..Abilities::for_mode(m)
                    };
                    if !self.abilities.may_fly {
                        self.state.flying = false;
                    }
                }
            }
            P::Respawn(r) => {
                if let Some(m) = GameMode::from_id(r.spawn.game_mode) {
                    self.game_mode = m;
                }
                const KEEP_ATTRIBUTES: u8 = 0x01;
                if r.data_kept & KEEP_ATTRIBUTES == 0 {
                    self.attributes.clear();
                    self.attrs = PlayerAttributes {
                        flying_speed: self.attrs.flying_speed,
                        ..PlayerAttributes::default()
                    };
                }
                let pos = self.state.pos;
                self.state = PlayerState::at(pos);
                self.abilities = Abilities::for_mode(self.game_mode);
                self.has_position = false;
                self.loaded = false;
                self.pending_accept = None;
                self.pending_accept_age = 0;
                self.sent = Sent::default();
                self.jump_trigger_time = 0;
                self.sprint_trigger_time = 0;
                self.jump_held = false;
                self.int.respawned();
            }
            other => return self.handle_interact_packet(other),
        }
        true
    }

    fn set_look(&mut self, yaw: f32, pitch: f32) {
        let pitch = pitch.clamp(-90.0, 90.0);
        self.state.yaw = yaw;
        self.state.pitch = pitch;
        self.look_offset = (yaw - self.last_input_look.0, pitch - self.last_input_look.1);
    }

    /// Applies a `player_position` teleport with its relative flags (wiki "Synchronize Player
    /// Position"; 26.3 adds velocity and the rotate-velocity flag).
    fn teleport(&mut self, p: &cb::PlayerPosition) {
        tracing::debug!("teleport {p:?} from {:?}", self.state);
        let rel = |bit: i32| p.flags & bit != 0;
        let pick = |bit, old: f64, new: f64| if rel(bit) { old + new } else { new };
        let s = &mut self.state;
        let old_yaw = s.yaw;
        let old_pitch = s.pitch;
        s.pos = DVec3::new(
            pick(teleport_flags::X, s.pos.x, p.pos.x),
            pick(teleport_flags::Y, s.pos.y, p.pos.y),
            pick(teleport_flags::Z, s.pos.z, p.pos.z),
        );
        let yaw = if rel(teleport_flags::YAW) {
            old_yaw + p.yaw
        } else {
            p.yaw
        };
        let pitch = if rel(teleport_flags::PITCH) {
            old_pitch + p.pitch
        } else {
            p.pitch
        };
        let mut v = s.velocity;
        if rel(teleport_flags::ROTATE_VELOCITY) {
            // Rotate the old velocity by the change in yaw (pitch rotation not modelled).
            let d = (yaw - old_yaw).to_radians() as f64;
            let (sin, cos) = d.sin_cos();
            v = DVec3::new(v.x * cos - v.z * sin, v.y, v.x * sin + v.z * cos);
        }
        s.velocity = DVec3::new(
            pick(teleport_flags::VEL_X, v.x, p.velocity.x),
            pick(teleport_flags::VEL_Y, v.y, p.velocity.y),
            pick(teleport_flags::VEL_Z, v.z, p.velocity.z),
        );
        s.fall_distance = 0.0;
        self.own_sounds.reset_steps();
        self.set_look(yaw, pitch);
        tracing::debug!("after teleport {:?}", self.state);
        self.prev_pos = self.state.pos;
        // The camera's smoothed eye height is not reset by a teleport (vanilla `Camera` keeps
        // it; only the position jumps, and prev_pos above already pins the interpolation).
        self.has_position = true;
        self.teleports += 1;
    }

    /// Sends the `accept_teleportation` (and the first `player_loaded`) for a teleport that
    /// arrived a full tick ago. A vanilla client answers teleports from its game loop,
    /// never from the network reader; answering within microseconds of the join teleport
    /// races the server's join bookkeeping and it logs
    /// "moved too quickly! <spawn position>" (observed on the vanilla 26.3 server, ~1 join
    /// in 8). One full tick later the server holds the login position, so the accept's
    /// position matches it. The accepted position counts as sent (26.3's accept carries the
    /// resulting position and rotation; an extra move_player_pos_rot right after it, what
    /// older clients sent, gets the client kicked with invalid_player_movement after a
    /// relative `/tp ~ ~10 ~`).
    fn flush_pending_accept(&mut self, out: &mut Vec<ServerboundPlay>) {
        if self.pending_accept.is_none() {
            return;
        }
        self.pending_accept_age += 1;
        if self.pending_accept_age < 2 {
            // Age 1 is the tick the teleport arrived in (the driver drains packets and
            // ends the tick in one call); the accept goes out on the next.
            return;
        }
        let teleport_id = self.pending_accept.take().unwrap();
        tracing::debug!(
            "accept_teleportation {teleport_id} at ({:.1}, {:.1}, {:.1})",
            self.state.pos.x,
            self.state.pos.y,
            self.state.pos.z
        );
        out.push(ServerboundPlay::AcceptTeleportation(
            sb::AcceptTeleportation {
                teleport_id,
                pos: self.state.pos,
                yaw: self.state.yaw,
                pitch: self.state.pitch,
            },
        ));
        if !self.loaded {
            // Observed on the 26.3 server: a movement packet between accept_teleportation
            // and player_loaded gets the client kicked (invalid_player_movement), so the
            // first move goes out next tick.
            out.push(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}));
            self.loaded = true;
        }
    }

    fn mark_sent_position(&mut self, on_ground: bool, horizontal_collision: bool) {
        self.sent.pos = self.state.pos;
        self.sent.yaw = self.state.yaw;
        self.sent.pitch = self.state.pitch;
        self.sent.on_ground = on_ground;
        self.sent.horizontal_collision = horizontal_collision;
        self.sent.reminder = 0;
    }

    /// Vanilla `LivingEntity.jumpFromGround`: the vertical movement becomes the jump strength
    /// (scaled by the jump factor of the block the player stands in or on) unless it is already
    /// higher, and a sprinting player also gets the horizontal boost. `LocalPlayer.aiStep` calls
    /// it when the double tap turns creative flight on while the player is on the ground.
    fn jump_from_ground(&mut self, world: &dyn BlockView, blocks: &dyn BlockPhysics) {
        let at_feet = blocks.jump_factor(world.block_state(BlockPos::from_dvec3(self.state.pos)));
        let factor = if at_feet != 1.0 {
            at_feet
        } else {
            let below =
                BlockPos::from_dvec3(self.state.pos - DVec3::new(0.0, BLOCK_BELOW_OFFSET, 0.0));
            blocks.jump_factor(world.block_state(below))
        };
        let power = self.attrs.jump_strength * f64::from(factor);
        if power > 1.0e-5 {
            self.state.velocity.y = self.state.velocity.y.max(power);
        }
        if self.state.sprinting {
            let angle = f64::from(self.state.yaw) * 0.017_453_292;
            self.state.velocity.x -= angle.sin() * f64::from(SPRINT_JUMP_BOOST);
            self.state.velocity.z += angle.cos() * f64::from(SPRINT_JUMP_BOOST);
        }
    }

    fn apply_attributes(&mut self) {
        let Some(ids) = self.attribute_ids else {
            return;
        };
        if !self.attributes_dirty {
            return;
        }
        self.attributes_dirty = false;
        let d = PlayerAttributes::default();
        let get = |id: i32, default: f64, skip_sprint: bool| {
            self.attributes
                .iter()
                .find(|a| a.id == id)
                .map_or(default, |a| {
                    attribute_value(a, |m| {
                        skip_sprint
                            && m.namespace() == "minecraft"
                            && m.path() == SPRINTING_MODIFIER
                    })
                })
        };
        let a = PlayerAttributes {
            movement_speed: get(ids.movement_speed, d.movement_speed, true),
            jump_strength: get(ids.jump_strength, d.jump_strength, false),
            gravity: get(ids.gravity, d.gravity, false),
            step_height: get(ids.step_height, d.step_height, false),
            sneaking_speed: get(ids.sneaking_speed, d.sneaking_speed, false),
            scale: get(ids.scale, d.scale as f64, false) as f32,
            flying_speed: self.attrs.flying_speed,
            ..d
        };
        self.attrs = a;
        let r = interact::InteractAttributes::default();
        self.int.attrs = interact::InteractAttributes {
            block_break_speed: get(ids.block_break_speed, r.block_break_speed, false),
            mining_efficiency: get(ids.mining_efficiency, r.mining_efficiency, false),
            submerged_mining_speed: get(
                ids.submerged_mining_speed,
                r.submerged_mining_speed,
                false,
            ),
            block_reach: get(ids.block_interaction_range, r.block_reach, false),
            entity_reach: get(ids.entity_interaction_range, r.entity_reach, false),
            attack_speed: get(ids.attack_speed, r.attack_speed, false),
        };
    }

    /// One 20 Hz tick against the real world and game data.
    pub fn tick(
        &mut self,
        input: &InputState,
        world: &haste_world::World,
        data: &haste_data::GameData,
        out: &mut Vec<ServerboundPlay>,
    ) {
        self.prepare(data);
        // No entity data on this path (no entity store here): every entity type is unknown and
        // so pickable, which vanilla would only mean for mod types. The driver fills
        // `InteractEnv::entity_types` from the store (`docs/INTERFACES.md`, `entity_box`).
        let env = interact::InteractEnv {
            data,
            entities: &[],
            entity_types: &[],
        };
        self.tick_interact(input, world, &env, None, out);
    }

    /// Resolves what the player needs from the game data once (attribute ids, tool table).
    pub fn prepare(&mut self, data: &haste_data::GameData) {
        if self.attribute_ids.is_none() {
            if let Some(ids) = AttributeIds::from_data(data) {
                self.set_attribute_ids(ids);
            }
        }
        self.int.prepare(data);
    }

    /// One 20 Hz tick over any world view without interactions (tests use
    /// `haste_physics::testing`). `vehicle` is the pose of the vehicle we ride, if any, this tick
    /// ([`LocalPlayer::ride_tick`]).
    pub fn tick_with(
        &mut self,
        input: &InputState,
        world: &dyn BlockView,
        shapes: &dyn CollisionShapes,
        blocks: &dyn BlockPhysics,
        vehicle: Option<VehiclePose>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        self.tick_inner(
            input,
            &TickCtx {
                world,
                shapes,
                blocks,
                env: None,
            },
            vehicle,
            out,
        );
    }

    /// One 20 Hz tick with the interactions set by [`LocalPlayer::set_interact`] first (hotbar,
    /// attack/break, use/place, pick block), then movement, as a vanilla client orders them.
    /// `vehicle` is the pose of the vehicle we ride, if any, this tick.
    pub fn tick_interact(
        &mut self,
        input: &InputState,
        world: &dyn BlockView,
        env: &interact::InteractEnv<'_>,
        vehicle: Option<VehiclePose>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        self.tick_inner(
            input,
            &TickCtx {
                world,
                shapes: &env.data.shapes,
                blocks: env.data,
                env: Some(env),
            },
            vehicle,
            out,
        );
    }

    /// Mounts or dismounts (`set_passengers`): `seat` is where our feet sit on the vehicle each
    /// tick, in the vehicle's frame (the vehicle's passenger attachment point minus our own,
    /// `haste_entity::passenger_seat`), and `boat` the vehicle's client-side simulation when it
    /// is a boat we steer (`None` for any other vehicle, and on a dismount). On a dismount,
    /// normal physics resumes from where the player is.
    pub fn set_vehicle(
        &mut self,
        vehicle: Option<EntityId>,
        seat: SeatPoint,
        boat: Option<crate::boat::Boat>,
    ) {
        if vehicle.is_some() {
            self.seat = seat;
        }
        self.vehicle = vehicle;
        self.boat = boat;
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
    fn boat_tick(&mut self, ctx: &TickCtx<'_>, out: &mut Vec<ServerboundPlay>) {
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

    fn tick_inner(
        &mut self,
        input: &InputState,
        ctx: &TickCtx<'_>,
        vehicle: Option<VehiclePose>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        self.apply_attributes();
        self.prev_pos = self.state.pos;
        self.last_input_look = (input.yaw, input.pitch);
        let last_input_forward = self.last_input_forward;
        self.last_input_forward = input.forward && !input.back;
        // Vanilla `Camera.tick` runs at the start of the client tick, before the player's own
        // tick (so the target is the pose's eye height as the last tick ended it), and even
        // while the player has no position yet: the camera halves its distance to the target
        // each tick (`eyeHeight += (getEyeHeight() - eyeHeight) * 0.5`).
        self.camera_eye_old = self.camera_eye;
        self.camera_eye += (self.eye_height() as f32 - self.camera_eye) * 0.5;
        self.flush_pending_accept(out);
        // The effects' tick, in vanilla's `LivingEntity.baseTick` order (before the position
        // check below): their durations and blend states drive the screen effects.
        self.int.tick_effects();
        if !(self.has_position && self.loaded) {
            return;
        }

        // Vanilla `Entity.baseTick` starts the tick with the sprint particle
        // (`canSpawnSprintParticle`: sprinting, not in water/lava, not sneaking or
        // spectating, alive) — the velocity is the one the previous tick ended with.
        if self.state.sprinting
            && !self.state.in_water
            && !self.state.in_lava
            && !self.state.sneaking
            && self.game_mode != GameMode::Spectator
            && self.health > 0.0
            && ctx
                .world
                .is_chunk_loaded(BlockPos::from_dvec3(self.state.pos).chunk())
        {
            self.block_particles_out
                .push(crate::block_particles::BlockParticleRequest::Sprint {
                    feet: self.state.pos,
                    velocity: self.state.velocity,
                });
        }

        // 0. Hotbar, crosshair target and interactions (vanilla handles key bindings before
        // the player's own tick).
        let look = (
            input.yaw + self.look_offset.0,
            (input.pitch + self.look_offset.1).clamp(-90.0, 90.0),
        );
        if let Some(env) = ctx.env {
            self.interact_step(look, ctx.world, env, out);
        }

        // 1. player_input when the held keys change (raw keys, as pressed). While riding this
        // is how the vehicle learns our keys (steering, and sneak to dismount: the shift flag
        // makes the server answer with a set_passengers that removes us).
        let flags = input_bits(input);
        if self.sent.input != Some(flags) {
            out.push(ServerboundPlay::PlayerInput(sb::PlayerInput { flags }));
            self.sent.input = Some(flags);
        }

        // 2. The passenger tick (vanilla `rideTick`) replaces physics: a passenger sends
        // rotation only, never position packets (the server owns a passenger's position).
        if self.vehicle.is_some() {
            // The vehicle's own tick first (vanilla `ClientLevel.tickNonPassenger`), then the
            // passenger's `rideTick` seats us on it, then `LocalPlayer.sendChanges`: the rotation
            // and, for a vehicle we control, the `move_vehicle` that carries its simulation.
            self.boat_tick(ctx, out);
            self.ride_tick(input, look, vehicle);
            self.send_rotation(out);
            if let Some(boat) = &self.boat {
                if boat.pos.is_finite() && boat.yaw.is_finite() {
                    out.push(ServerboundPlay::MoveVehicle(sb::MoveVehicle {
                        pos: boat.pos,
                        yaw: boat.yaw,
                        pitch: 0.0,
                        on_ground: boat.on_ground,
                    }));
                }
            }
            out.push(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
            return;
        }

        // 3. Physics. The player does not move while its chunk is not loaded; vanilla's
        // `Player.aiStep` counts the double-tap jump window down before `LocalPlayer.aiStep`
        // looks at the keys. Vanilla `Player.tick` sets `noPhysics = isSpectator()` and clears
        // on-ground for spectators before the travel step.
        self.state.no_physics = self.game_mode == GameMode::Spectator;
        if self.state.no_physics {
            self.state.on_ground = false;
        }
        if self.jump_trigger_time > 0 {
            self.jump_trigger_time -= 1;
        }
        if self.sprint_trigger_time > 0 {
            self.sprint_trigger_time -= 1;
        }
        let mut phys_input = *input;
        phys_input.yaw = input.yaw + self.look_offset.0;
        phys_input.pitch = (input.pitch + self.look_offset.1).clamp(-90.0, 90.0);
        if self.food <= SPRINT_FOOD_LEVEL && !self.abilities.may_fly {
            phys_input.sprint = false;
            self.state.sprinting = false;
        }
        // Vanilla `LocalPlayer.aiStep`'s double tap of forward, after `Player.aiStep` counted
        // `sprintTriggerTime` down above: sneak held, an item in use or the backward key
        // cancels the window. `hasForwardImpulse` is read from the PREVIOUS tick's input
        // (vanilla captures it before `input.tick()`), so the window arms on a tick without
        // forward and starts the sprint on the fresh press inside it. `canStartSprinting` is
        // evaluated on the current keys — not sprinting, a forward impulse, `isSprintingPossible`
        // (enough food or may-fly; the not-blind gate is vanilla's `Player.isMobilityRestricted`,
        // not modelled here), not slowed by an item in use and not moving slowly
        // (`isMovingSlowly`: crouching or the swimming pose out of water) unless under water.
        // The window lasts `options.sprintWindow` ticks (default 7). The sprint key branch
        // (`keyPresses.sprint()`) is the physics tick's own `input.sprint` rule.
        let forward_impulse = input.forward && !input.back;
        if input.sneak || self.is_using_item() || input.back {
            self.sprint_trigger_time = 0;
        }
        let moving_slowly = self.state.crouching
            || (self.state.pose == haste_physics::Pose::Swimming && !self.state.in_water);
        let under_water = self.state.eyes_in_water && self.state.in_water;
        let can_start_sprinting = !self.state.sprinting
            && forward_impulse
            && (self.food > SPRINT_FOOD_LEVEL || self.abilities.may_fly)
            && !self.is_using_item()
            && (!moving_slowly || under_water);
        if can_start_sprinting && !last_input_forward {
            if self.sprint_trigger_time > 0 {
                phys_input.sprint = true;
            } else {
                self.sprint_trigger_time = SPRINT_DOUBLE_TAP_TICKS;
            }
        }
        let was_flying = self.state.flying;
        let was_on_ground = self.state.on_ground;
        let pre_tick_pos = self.state.pos;
        let pre_tick_fall = self.state.fall_distance;
        // Vanilla's double tap: while the window is armed a fresh press of jump (or the input
        // layer's own double-tap signal) toggles flight; without it a press arms the window for
        // 7 ticks. Vanilla keeps the window while the player swims, and skips the toggle on a
        // vehicle the player cannot jump (haste has no vehicles).
        let armed = self.jump_trigger_time > 0;
        let double_tap = std::mem::take(&mut self.toggle_flight);
        let pressed = input.jump && !self.jump_held;
        self.jump_held = input.jump;
        let mut just_toggled_creative_flight = false;
        if self.abilities.may_fly && (pressed || double_tap) {
            if armed {
                if !self.state.swimming {
                    self.jump_trigger_time = 0;
                    self.state.flying = !self.state.flying;
                    just_toggled_creative_flight = true;
                    if self.state.flying && self.state.on_ground {
                        self.jump_from_ground(ctx.world, ctx.blocks);
                    }
                }
            } else {
                self.jump_trigger_time = JUMP_DOUBLE_TAP_TICKS;
            }
        }
        if !self.abilities.may_fly {
            self.state.flying = false;
        }
        // Vanilla `LocalPlayer.aiStep`: a fresh jump press in the air with a glider equipped and
        // no liquid starts gliding (`tryToStartFallFlying` -> `START_FALL_FLYING`), skipped on
        // the press that just toggled creative flight. The flag is set locally so this tick
        // already glides; the server confirms it through the shared flag 7 and clears it when the
        // glide ends. The climbable check (`!onClimbable`) is not modelled (no climbable tag).
        if pressed
            && !just_toggled_creative_flight
            && self.state.can_start_fall_flying(self.has_glider())
        {
            self.state.start_fall_flying();
            out.push(ServerboundPlay::PlayerCommand(sb::PlayerCommand {
                entity_id: self.entity_id.0,
                action: command::START_FALL_FLYING,
                jump_boost: 0,
            }));
        }
        if self.state.flying != was_flying {
            self.own_sounds.reset_steps();
        }
        let chunk = BlockPos::from_dvec3(self.state.pos).chunk();
        if ctx.world.is_chunk_loaded(chunk) {
            haste_physics::tick(
                &mut self.state,
                &phys_input,
                &self.attrs,
                ctx.world,
                ctx.shapes,
                ctx.blocks,
            );
            // Own block sounds of moving: the step rule on the block below the
            // feet, the fall sound on a hard landing (vanilla plays both on
            // the client; the server excludes the acting player).
            let movement = self.state.pos - pre_tick_pos;
            let sound_type = ctx.env.and_then(|e| {
                supporting_block_state(self.state.pos, ctx.world)
                    .and_then(|s| e.data.block_sound_type(s))
            });
            self.own_sounds
                .after_move(movement, self.state.pos, sound_type, &mut self.sound_out);
            if !was_on_ground
                && self.state.on_ground
                && pre_tick_fall > SAFE_FALL_DISTANCE
                && !self.state.in_water
                && !self.state.flying
            {
                if let Some(t) = sound_type {
                    self.own_sounds
                        .landed_hard(self.state.pos, t, &mut self.sound_out);
                }
            }
        } else {
            self.state.yaw = phys_input.yaw;
            self.state.pitch = phys_input.pitch;
            // Vanilla `LivingEntity.travelInAir` (which `Player.travel` calls even while flying,
            // then keeps the flying player's own vertical movement): with the chunk below not
            // loaded the client skips gravity and pins the vertical movement to -0.1 while the
            // player is above the world's minimum, with the 0.98 vertical drag on top. At or below
            // the minimum it is 0.0, so the player is left exactly where it was.
            if !self.state.flying {
                let falling = self.state.pos.y > f64::from(ctx.world.min_y());
                self.state.velocity.y = if falling {
                    UNLOADED_CHUNK_FALL_SPEED
                } else {
                    0.0
                };
                if falling {
                    self.state.pos.y += self.state.velocity.y;
                    self.state.on_ground = false;
                }
            }
        }

        // Vanilla `LocalPlayer.aiStep`: the water-vision ramp, +1/tick with the eyes in water
        // (spectators +10), -10/tick once out of it, both clamped to 0..600.
        if self.state.eyes_in_water {
            let speed = if self.game_mode == GameMode::Spectator {
                10
            } else {
                1
            };
            self.water_vision_time = (self.water_vision_time + speed).clamp(0, 600);
        } else if self.water_vision_time > 0 {
            self.water_vision_time = (self.water_vision_time - 10).clamp(0, 600);
        }

        // 4. Flight started or stopped (toggle or landing).
        if self.state.flying != was_flying {
            let mut f = 0;
            if self.state.flying {
                f |= ability_flags::FLYING;
            }
            out.push(ServerboundPlay::PlayerAbilities(sb::PlayerAbilities {
                flags: f,
            }));
        }

        // 5. Sprint state.
        if self.state.sprinting != self.sent.sprinting {
            out.push(ServerboundPlay::PlayerCommand(sb::PlayerCommand {
                entity_id: self.entity_id.0,
                action: if self.state.sprinting {
                    command::START_SPRINTING
                } else {
                    command::STOP_SPRINTING
                },
                jump_boost: 0,
            }));
            self.sent.sprinting = self.state.sprinting;
        }

        // 6. Movement.
        self.send_movement(out);

        // 6. End of tick.
        out.push(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
    }

    /// Movement send while riding (a passenger never sends position packets: the server owns
    /// its position): rotation when it changed, else a status-only packet when on-ground or
    /// collision changed. Sneak dismounts arrive as `player_input`'s shift flag, answered by
    /// the server with `set_passengers`.
    fn send_rotation(&mut self, out: &mut Vec<ServerboundPlay>) {
        let s = &self.state;
        if !(s.yaw.is_finite() && s.pitch.is_finite()) {
            tracing::error!("non-finite player state, not sent: {s:?}");
            return;
        }
        let rot_changed = s.yaw != self.sent.yaw || s.pitch != self.sent.pitch;
        let mut flags = 0;
        if s.on_ground {
            flags |= move_flags::ON_GROUND;
        }
        if s.horizontal_collision {
            flags |= move_flags::HORIZONTAL_COLLISION;
        }
        let status_changed = s.on_ground != self.sent.on_ground
            || s.horizontal_collision != self.sent.horizontal_collision;
        if rot_changed {
            out.push(ServerboundPlay::MovePlayerRot(sb::MovePlayerRot {
                yaw: s.yaw,
                pitch: s.pitch,
                flags,
            }));
            self.sent.yaw = s.yaw;
            self.sent.pitch = s.pitch;
        } else if status_changed {
            out.push(ServerboundPlay::MovePlayerStatusOnly(
                sb::MovePlayerStatusOnly { flags },
            ));
        }
        self.sent.on_ground = s.on_ground;
        self.sent.horizontal_collision = s.horizontal_collision;
    }

    fn send_movement(&mut self, out: &mut Vec<ServerboundPlay>) {
        let s = &self.state;
        if !(s.pos.is_finite() && s.yaw.is_finite() && s.pitch.is_finite()) {
            tracing::error!("non-finite player state, not sent: {s:?}");
            return;
        }
        self.sent.reminder += 1;
        let pos_changed = (s.pos - self.sent.pos).length_squared() > POSITION_THRESHOLD_SQ
            || self.sent.reminder >= POSITION_REMINDER_TICKS;
        let rot_changed = s.yaw != self.sent.yaw || s.pitch != self.sent.pitch;
        let mut flags = 0;
        if s.on_ground {
            flags |= move_flags::ON_GROUND;
        }
        if s.horizontal_collision {
            flags |= move_flags::HORIZONTAL_COLLISION;
        }
        let status_changed = s.on_ground != self.sent.on_ground
            || s.horizontal_collision != self.sent.horizontal_collision;
        match (pos_changed, rot_changed) {
            (true, true) => {
                tracing::debug!(
                    "move_player_pos_rot to ({:.1}, {:.1}, {:.1})",
                    s.pos.x,
                    s.pos.y,
                    s.pos.z
                );
                out.push(ServerboundPlay::MovePlayerPosRot(sb::MovePlayerPosRot {
                    pos: s.pos,
                    yaw: s.yaw,
                    pitch: s.pitch,
                    flags,
                }));
            }
            (true, false) => {
                tracing::debug!(
                    "move_player_pos to ({:.1}, {:.1}, {:.1})",
                    s.pos.x,
                    s.pos.y,
                    s.pos.z
                );
                out.push(ServerboundPlay::MovePlayerPos(sb::MovePlayerPos {
                    pos: s.pos,
                    flags,
                }));
            }
            (false, true) => out.push(ServerboundPlay::MovePlayerRot(sb::MovePlayerRot {
                yaw: s.yaw,
                pitch: s.pitch,
                flags,
            })),
            (false, false) if status_changed => out.push(ServerboundPlay::MovePlayerStatusOnly(
                sb::MovePlayerStatusOnly { flags },
            )),
            (false, false) => {}
        }
        if pos_changed {
            self.sent.pos = s.pos;
            self.sent.reminder = 0;
        }
        if rot_changed {
            self.sent.yaw = s.yaw;
            self.sent.pitch = s.pitch;
        }
        self.sent.on_ground = s.on_ground;
        self.sent.horizontal_collision = s.horizontal_collision;
    }
}

/// Vanilla's safe fall distance (the `SAFE_FALL_DISTANCE` attribute's default):
/// a landing from above this plays the block's fall sound; below it, nothing.
pub const SAFE_FALL_DISTANCE: f32 = 3.0;

/// The block supporting the player at `pos` (vanilla `Entity.getOnPosLegacy`:
/// the block at feet y minus 0.2), for step and fall sounds.
fn supporting_block_state(pos: DVec3, world: &dyn BlockView) -> Option<haste_core::BlockStateId> {
    let p = BlockPos::new(
        pos.x.floor() as i32,
        (pos.y - 0.2).floor() as i32,
        pos.z.floor() as i32,
    );
    let state = world.block_state(p);
    (state != haste_core::BlockStateId::AIR).then_some(state)
}

/// `player_input` flags for the held keys.
pub fn input_bits(i: &InputState) -> u8 {
    let mut f = 0;
    for (on, bit) in [
        (i.forward, input_flags::FORWARD),
        (i.back, input_flags::BACKWARD),
        (i.left, input_flags::LEFT),
        (i.right, input_flags::RIGHT),
        (i.jump, input_flags::JUMP),
        (i.sneak, input_flags::SNEAK),
        (i.sprint, input_flags::SPRINT),
    ] {
        if on {
            f |= bit;
        }
    }
    f
}

/// What one player tick runs against: the world and its physics tables, and the interactions
/// environment when the tick has interactions (the client's real path; `None` in tests that
/// only move).
struct TickCtx<'a> {
    world: &'a dyn BlockView,
    shapes: &'a dyn CollisionShapes,
    blocks: &'a dyn BlockPhysics,
    env: Option<&'a interact::InteractEnv<'a>>,
}

pub mod interact;
pub mod script;
#[cfg(test)]
mod tests;
