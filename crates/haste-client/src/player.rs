//! The local player (player workstream): owns the physics state, consumes the packets about the
//! local player and produces the serverbound packets a vanilla client sends for each tick.
//!
//! Per tick, in this order (the order a vanilla client is observed to send them in):
//! 1. `set_carried_item` when the hotbar slot changed underneath the last value sent
//!    (`gameMode.tick` -> `ensureHasSentCarriedItem`),
//! 2. the interactions (hotbar keys, clicks, held use, `continueAttack` — `handleKeybinds`),
//! 3. physics (`haste_physics::tick_with`) with `player_command` START_FALL_FLYING and
//!    `player_abilities` from `aiStep`,
//! 4. `player_input` when the held movement keys changed since the last one sent
//!    (`sendChanges` runs after `aiStep`),
//! 5. `player_command` start/stop sprinting when the sprint state changed,
//! 6. one of `move_player_pos_rot` / `_pos` / `_rot` / `_status_only` (send rules below),
//! 7. `client_tick_end` — every tick a world exists, before `player_loaded` too.
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
use haste_data::BlockStates;
use haste_entity::riding::SeatPoint;
use haste_physics::movement::{BLOCK_BELOW_OFFSET, SPRINT_JUMP_BOOST};
use haste_physics::{PlayerAttributes, PlayerState};
use haste_protocol::packets::play::clientbound::{self as cb, teleport_flags};
use haste_protocol::packets::play::serverbound::{self as sb, input_flags, move_flags};
use haste_protocol::{ClientboundPlay, Login, ServerboundPlay};
use level_load::LevelLoad;

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
#[cfg(test)]
pub use unloaded_chunk::UNLOADED_CHUNK_FALL_SPEED;

/// `player_command` actions (wiki "Player Command"; ordinals from the 26.3
/// `ServerboundPlayerCommandPacket.Action` enum: STOP_SLEEPING 0, START_SPRINTING 1,
/// STOP_SPRINTING 2, START_RIDING_JUMP 3, STOP_RIDING_JUMP 4, OPEN_INVENTORY 5,
/// START_FALL_FLYING 6).
pub mod command {
    pub const START_SPRINTING: i32 = 1;
    pub const STOP_SPRINTING: i32 = 2;
    pub const START_RIDING_JUMP: i32 = 3;
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
    /// The hotbar slot the server was last told about (`MultiPlayerGameMode.carriedIndex`).
    /// `LocalPlayer::ensure_carried_item` (vanilla `ensureHasSentCarriedItem`, the start of
    /// every `gameMode.tick`) sends the echo when the slot changed underneath it — the
    /// client's own pick or the server's `set_held_slot`.
    carried_index: i32,
    /// Whether a `player_position` arrived since login/respawn.
    pub has_position: bool,
    /// Whether `player_loaded` was sent since login/respawn (movement is sent only after):
    /// it goes out once the level is ready, not with the teleport's accept.
    pub loaded: bool,
    /// The wait for the level after this login/respawn (vanilla `LevelLoadTracker`): the
    /// player's tick is held until the chunk at its position has streamed in.
    level_load: LevelLoad,
    /// The vehicle the server has us riding (`set_passengers`); `None` when on foot. While set,
    /// the player's own physics does not run: each tick its position is the vehicle's position
    /// plus the seat offset ([`LocalPlayer::ride_tick`]).
    pub vehicle: Option<EntityId>,
    /// The boat we steer ([`crate::boat`]), set on mount together with `vehicle` and cleared on
    /// any dismount: it holds the client's own authoritative simulation of a vehicle whose
    /// controlling passenger we are (vanilla `Entity.isLocalInstanceAuthoritative`).
    boat: Option<crate::boat::Boat>,
    /// The horse-family vehicle we steer ([`crate::horse`]), like `boat` above.
    horse: Option<crate::horse::Horse>,
    /// The pig or strider we steer with its stick ([`crate::item_based_steering`]), like
    /// `boat` above: it ticks only while the rider holds the stick (vanilla
    /// `Pig`/`Strider.getControllingPassenger`).
    item_steered: Option<crate::item_based_steering::ItemSteered>,
    /// The mount's charged jump state (vanilla `LocalPlayer.jumpRidingTicks` /
    /// `jumpRidingScale`): counts the held jump while on a horse-family vehicle, and the scale
    /// is what the `player_command` START_RIDING_JUMP carries and the HUD jump bar shows.
    jump_riding_ticks: i32,
    jump_riding_scale: f32,
    /// The jump key state of the previous tick, for the charge's rising and falling edges
    /// (vanilla reads `wasJumping` from before `input.tick()`).
    riding_jump_held: bool,
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
    pub(crate) prev_pos: DVec3,
    /// The camera's smoothed eye height and its last-tick value (vanilla `Camera.eyeHeight`/
    /// `eyeHeightOld`, f32 like vanilla). The camera, not the player, smooths: each tick it
    /// halves the distance to the pose's eye height, and the render lerps the two values.
    camera_eye: f32,
    camera_eye_old: f32,
    /// The camera entity when it is not us (vanilla `Minecraft.getCameraEntity`), set by the
    /// session before each tick (see the `camera` module).
    camera_entity: Option<crate::game::camera_entity::CameraEntitySnapshot>,
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
    /// The local player's footsteps (client-side rules, sound/steps.rs).
    steps: crate::sound::StepSounds,
    /// The local player's own fall sounds (client-side rules, sound/falls.rs).
    fall_sounds: crate::sound::FallSounds,
    /// Own-sound requests this tick, drained into `Session::sound_requests` by `end_tick`.
    pub(crate) sound_out: Vec<crate::sound::SoundRequest>,
    /// Own block particles this tick (our break prediction's destroy grid, the
    /// mining start tick's crack), drained into `Session::block_particle_requests`.
    pub(crate) block_particles_out: Vec<crate::block_particles::BlockParticleRequest>,
    /// Own item particles this tick (a consumable's eat/drink crumbs), drained into
    /// `Session::item_particle_requests`.
    pub(crate) item_particles_out: Vec<crate::item_particles::ItemParticleRequest>,
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
    /// Vanilla `LocalPlayer.handsBusy` (`isHandsBusy`): true while rowing — a movement key
    /// held while controlling a boat (`rideTick` sets it each passenger tick, clears it on
    /// dismount). While it is, `Minecraft.startAttack` and `startUseItem` do nothing.
    hands_busy: bool,
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
            carried_index: 0,
            has_position: false,
            loaded: false,
            level_load: LevelLoad::start(),
            vehicle: None,
            boat: None,
            horse: None,
            item_steered: None,
            jump_riding_ticks: 0,
            jump_riding_scale: 0.0,
            riding_jump_held: false,
            seat: SeatPoint::ZERO,
            pending_accept: None,
            pending_accept_age: 0,
            teleports: 0,
            prev_pos: DVec3::ZERO,
            camera_eye: 0.0,
            camera_eye_old: 0.0,
            camera_entity: None,
            look_offset: (0.0, 0.0),
            last_input_look: (0.0, 0.0),
            last_input_forward: false,
            sent: Sent::default(),
            attribute_ids: None,
            attributes: Vec::new(),
            attributes_dirty: false,
            toggle_flight: false,
            own_sounds: crate::sound::OwnBlockSounds::new(),
            steps: crate::sound::StepSounds::new(),
            fall_sounds: crate::sound::FallSounds::new(),
            sound_out: Vec::new(),
            block_particles_out: Vec::new(),
            item_particles_out: Vec::new(),
            jump_trigger_time: 0,
            sprint_trigger_time: 0,
            jump_held: false,
            water_vision_time: 0,
            int: interact::Interaction::default(),
            hands_busy: false,
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

    /// Own item particles accumulated this tick (a consumable in use), for the caller to
    /// drain into the session's request queue.
    pub fn take_own_item_particles(
        &mut self,
    ) -> std::vec::Drain<'_, crate::item_particles::ItemParticleRequest> {
        self.item_particles_out.drain(..)
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

    /// Consumes the packets about the local player. Returns false for packets it does not handle.
    /// The `out` buffer is kept for the caller's uniform signature; this handler defers every
    /// serverbound reply to the next tick (see [`LocalPlayer::flush_pending_accept`]) — except
    /// a teleport's break abort, which vanilla sends from `handleMovePlayer` itself.
    pub fn handle(&mut self, p: &ClientboundPlay, out: &mut Vec<ServerboundPlay>) -> bool {
        use ClientboundPlay as P;
        match p {
            P::PlayerPosition(t) => {
                self.teleport(t);
                // The accept is not sent from the network reader (immediately, mid-tick): a
                // join teleport answered within microseconds of arrival races the server's
                // own join bookkeeping, which logs "moved too quickly! <spawn position>"
                // (the delta it computed against a not-yet-set position). A vanilla client
                // answers from its game loop; the join teleport's accept waits one full
                // tick, every later one goes out in the tick it arrived in
                // (flush_pending_accept).
                self.pending_accept = Some(t.teleport_id);
                self.pending_accept_age = 0;
                // `ClientPacketListener.handleMovePlayer` also calls
                // `BlockStatePredictionHandler.onTeleport` and
                // `MultiPlayerGameMode.stopDestroyBlock`: a running break is aborted
                // here, before this tick's key handling.
                self.int.prediction.on_teleport();
                self.stop_destroy(out);
                // `xLast`/`yLast`/`zLast` are left untouched, so the first sendPosition
                // after the teleport reports the new position whatever the threshold (the
                // movement waits for the accept, as vanilla's handler has already answered).
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
                        // Index 8 `DATA_LIVING_ENTITY_FLAGS`: bit 0x01 is `isUsingItem`, bit
                        // 0x02 the used hand; the server clears it when a consume completes
                        // (`LivingEntity.onSyncedDataUpdated`).
                        (8, haste_core::entity::MetadataValue::Byte(b)) => {
                            self.int
                                .synced_using_item(*b & 0x01 != 0, (*b & 0x02 != 0) as i32);
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
                    // `GameType.updatePlayerAbilities`, run by `setLocalMode` on the change.
                    self.state.flying = spectator::flying_after_mode_change(m, self.state.flying);
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
                // `GameType.updatePlayerAbilities` again, on the respawn's game mode.
                self.state.flying =
                    spectator::flying_after_mode_change(self.game_mode, self.state.flying);
                self.has_position = false;
                self.loaded = false;
                // Vanilla starts a new `LevelLoadTracker` on every respawn
                // (`startWaitingForNewLevel`), dimension change included.
                self.level_load = LevelLoad::start();
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
        self.steps.reset();
        self.set_look(yaw, pitch);
        tracing::debug!("after teleport {:?}", self.state);
        self.prev_pos = self.state.pos;
        // The camera's smoothed eye height is not reset by a teleport (vanilla `Camera` keeps
        // it; only the position jumps, and prev_pos above already pins the interpolation).
        self.has_position = true;
        self.teleports += 1;
    }

    /// `MultiPlayerGameMode.ensureHasSentCarriedItem` (`gameMode.tick`, the start of every
    /// client tick): the server is told the selected hotbar slot whenever it changed
    /// underneath the last value sent — the client's own pick or the server's
    /// `set_held_slot` (vanilla's `ClientPacketListener.handleSetHeldSlot` only sets the
    /// inventory slot, so the next tick echoes `set_carried_item` back).
    fn ensure_carried_item(&mut self, out: &mut Vec<ServerboundPlay>) {
        if self.held_slot != self.carried_index {
            self.carried_index = self.held_slot;
            out.push(ServerboundPlay::SetCarriedItem(sb::SetCarriedItem {
                slot: self.held_slot as i16,
            }));
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
            border: None,
            warm_blocks: None,
            ui: None,
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
                warm_blocks: None,
            },
            vehicle,
            out,
        );
    }

    /// One 20 Hz tick: interactions first (hotbar, attack/break, use/place, pick block),
    /// then movement, as a vanilla client orders them. `vehicle`: this tick's vehicle.
    pub fn tick_interact(
        &mut self,
        input: &InputState,
        world: &dyn BlockView,
        env: &interact::InteractEnv<'_>,
        vehicle: Option<VehiclePose>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        // This tick's predicted blocks are part of the world the movement sees (vanilla:
        // `handleKeybinds`' setBlock precedes the player tick; see prediction.rs).
        let predicted = self.int.prediction.view(world);
        self.tick_inner(
            input,
            &TickCtx {
                world: &predicted,
                shapes: &env.data.shapes,
                blocks: env.data,
                env: Some(env),
                warm_blocks: env.warm_blocks,
            },
            vehicle,
            out,
        );
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
        self.camera_eye += (self.camera_eye_target() - self.camera_eye) * 0.5;
        self.flush_pending_accept(out);
        if !self.level_ready_gate(ctx.world, out) {
            // `Minecraft.tick` ends with ClientTickEnd every tick a level exists — before
            // `player_loaded`, and with a screen open. Nothing else leaves the client yet.
            out.push(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
            return;
        }
        // The effects' tick, in vanilla's `LivingEntity.baseTick` order (before the position
        // check below): their durations and blend states drive the screen effects.
        self.int.tick_effects();

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

        // 0. `gameMode.tick` (`Minecraft.tick` runs it before `handleKeybinds`):
        // `ensureHasSentCarriedItem` echoes the hotbar slot when it changed.
        self.ensure_carried_item(out);

        // Hotbar, crosshair target and interactions (vanilla handles key bindings before
        // the player's own tick).
        let look = (
            input.yaw + self.look_offset.0,
            (input.pitch + self.look_offset.1).clamp(-90.0, 90.0),
        );
        if let Some(env) = ctx.env {
            self.interact_step(look, input.sneak, ctx.world, env, out);
        }

        // `LocalPlayer.sendInput`: the input packet leaves with `sendChanges`, after `aiStep`
        // (physics, START_FALL_FLYING, the abilities toggle) and before the sprint command
        // and the movement. While riding this is how the vehicle learns our keys (steering,
        // and sneak to dismount: the shift flag makes the server answer with a
        // set_passengers that removes us).
        let flags = input_bits(input);
        let send_input = |p: &mut Self, out: &mut Vec<ServerboundPlay>| {
            if p.sent.input != Some(flags) {
                out.push(ServerboundPlay::PlayerInput(sb::PlayerInput { flags }));
                p.sent.input = Some(flags);
            }
        };

        // 2. The passenger tick (vanilla `rideTick`) replaces physics: a passenger sends
        // rotation only, never position packets (the server owns a passenger's position).
        if self.vehicle.is_some() {
            // The vehicle's own tick first (vanilla `ClientLevel.tickNonPassenger`), then the
            // passenger's `rideTick` seats us on it, then `LocalPlayer.sendChanges`: the
            // input, the rotation and, for a vehicle we control, the `move_vehicle` that
            // carries its simulation.
            self.boat_tick(ctx, out);
            self.horse_tick(input, look, ctx, out);
            self.item_steered_tick(look, vehicle, ctx);
            self.ride_tick(input, look, vehicle);
            send_input(self, out);
            self.send_rotation(out);
            let item_steered = self.steered_by_item();
            let steered = match (&self.boat, &self.horse, item_steered) {
                (Some(boat), _, _) => Some((boat.pos, boat.yaw, 0.0, boat.on_ground)),
                (_, Some(horse), _) => Some((horse.pos, horse.yaw, horse.pitch, horse.on_ground)),
                (_, _, Some(animal)) => Some(animal.pose()),
                _ => None,
            };
            if let Some((pos, yaw, pitch, on_ground)) = steered {
                if pos.is_finite() && yaw.is_finite() {
                    out.push(ServerboundPlay::MoveVehicle(sb::MoveVehicle {
                        pos,
                        yaw,
                        pitch,
                        on_ground,
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
        let mut phys_input = self.movement_input(input);
        // What the physics tick's sprint rules cannot see (vanilla `isSprintingPossible`'s
        // blindness and food terms, `isSlowDueToUsingItem`).
        self.state.sprint_blocked =
            self.int.is_blind() || (self.food <= SPRINT_FOOD_LEVEL && !self.abilities.may_fly);
        // Vanilla `LocalPlayer.isSlowDueToUsingItem` / `itemUseSpeedMultiplier`: the used
        // item's `use_effects` keep a sprint from starting unless `can_sprint`, and scale the
        // movement input while using.
        self.apply_item_use_effects(ctx.env);
        // Vanilla `LocalPlayer.aiStep`: when not `noPhysics` and not a passenger (the passenger
        // tick returned above), the four feet probes free a player inside a suffocating block
        // (sand, a closing piston or door) before the sprint rules below.
        if !self.state.no_physics {
            let bb = self.state.aabb(&self.attrs);
            haste_physics::move_towards_closest_space(
                &mut self.state,
                &bb,
                ctx.world,
                ctx.shapes,
                ctx.blocks,
            );
        }
        // Vanilla `LocalPlayer.aiStep`'s double tap of forward, after `Player.aiStep` counted
        // `sprintTriggerTime` down above: sneak held, an item in use or the backward key
        // cancels the window. `hasForwardImpulse` is read from the PREVIOUS tick's input
        // (vanilla captures it before `input.tick()`), so the window arms on a tick without
        // forward and starts the sprint on the fresh press inside it. `canStartSprinting` is
        // evaluated on the current keys (`haste_physics::can_start_sprinting`, the same rule the
        // physics tick applies to the sprint key).
        // The window lasts `options.sprintWindow` ticks (default 7). The sprint key branch
        // (`keyPresses.sprint()`) is the physics tick's own `input.sprint` rule.
        let forward_impulse = input.forward && !input.back;
        if input.sneak || self.is_using_item() || input.back {
            self.sprint_trigger_time = 0;
        }
        let under_water = self.state.eyes_in_water && self.state.in_water;
        let can_start_sprinting =
            haste_physics::can_start_sprinting(&self.state, forward_impulse, under_water);
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
        // The velocity this tick's move starts with (vanilla's `getDeltaMovement` at
        // `Entity.move`): the step rule's swim-sound volume reads it.
        let pre_tick_velocity = self.state.velocity;
        let pressed = input.jump && !self.jump_held;
        self.jump_held = input.jump;
        let just_toggled_creative_flight =
            self.creative_flight_step(pressed, ctx.world, ctx.blocks);
        // Vanilla `LocalPlayer.aiStep`: a fresh jump press in the air with a glider equipped and
        // no liquid starts gliding (`tryToStartFallFlying` -> `START_FALL_FLYING`), skipped on
        // the press that just toggled creative flight and while on a climbable
        // (`!this.onClimbable()`). The flag is set locally so this tick already glides; the
        // server confirms it through the shared flag 7 and clears it when the glide ends.
        // The climbable identity is built from the synced tags and the state table in place, so
        // the borrow splits from the `&mut self.state` the physics tick below needs.
        let climb = climbable::SyncedClimbables::new(self.int.block_tags(), ctx.blocks());
        if pressed
            && !just_toggled_creative_flight
            && !haste_physics::on_climbable(&self.state, ctx.world, &climb)
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
            self.steps.reset();
        }
        let chunk = BlockPos::from_dvec3(self.state.pos).chunk();
        if ctx.world.is_chunk_loaded(chunk) {
            // The active movement effects (vanilla `LivingEntity` reads them off itself): the
            // physics tick applies levitation, slow falling and jump boost.
            let (levitation, slow_falling, jump_boost) = self.int.movement_effects();
            self.state.levitation = levitation;
            self.state.slow_falling = slow_falling;
            self.state.jump_boost = jump_boost;
            haste_physics::tick_with(
                &mut self.state,
                &phys_input,
                &self.attrs,
                ctx.world,
                ctx.shapes,
                ctx.blocks,
                &climb,
            );
            // Own block sounds of moving: footsteps from vanilla's step rule, the
            // fall sound on a hard landing (the server excludes the acting player
            // from its broadcasts, so the client plays both itself).
            let movement = self.state.pos - pre_tick_pos;
            let sound_type = ctx.env.and_then(|e| {
                supporting_block_state(self.state.pos, ctx.world)
                    .and_then(|s| e.data.block_sound_type(s))
            });
            if let Some(e) = ctx.env {
                let (supporting, effect, primary) = crate::sound::resolve_step_blocks(
                    self.state.pos,
                    &self.state.aabb(&self.attrs),
                    self.state.on_ground,
                    ctx.world,
                    ctx.shapes,
                    e.data,
                    self.int.block_tags(),
                );
                let steps = crate::sound::StepCtx {
                    player_pos: self.state.pos,
                    movement,
                    velocity: pre_tick_velocity,
                    on_ground: self.state.on_ground,
                    flying: self.state.flying,
                    discrete: self.state.sneaking,
                    crouching: self.state.crouching,
                    swimming: self.state.swimming,
                    in_water: self.state.in_water,
                    supporting,
                    effect,
                    primary,
                };
                self.steps.after_move(&steps, &mut self.sound_out);
            }
            if !was_on_ground && self.state.on_ground {
                // Vanilla `Entity.checkFallDamage` runs inside the move that grounds the
                // player with the fall distance including this tick's final fall (our
                // physics resets it on grounding, so it is added back here).
                let fall = pre_tick_fall as f64 - movement.y.min(0.0);
                let landed_on = supporting_block_state(self.state.pos, ctx.world).zip(sound_type);
                let rule = ctx.env.and_then(|e| {
                    landed_on.map(|(state, _)| {
                        crate::sound::FallBlockRule::of(
                            state,
                            self.int.block_tags(),
                            Some(&e.data.blocks),
                        )
                    })
                });
                if let (Some((_, t)), Some(rule)) = (landed_on, rule) {
                    self.fall_sounds.landed(
                        self.state.pos,
                        fall,
                        self.abilities.may_fly,
                        Some(crate::sound::LandedOn { sound: t }),
                        rule,
                        &mut self.sound_out,
                    );
                }
            }
        } else {
            self.state.yaw = phys_input.yaw;
            self.state.pitch = phys_input.pitch;
            self.unloaded_chunk_step(ctx.world);
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

        // `LocalPlayer.sendChanges` opens with the input packet (after `aiStep`, whose
        // START_FALL_FLYING and abilities toggle went out above).
        send_input(self, out);

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

        // 6. Movement (`LocalPlayer.sendPosition`: only while we are the camera entity).
        if self.is_controlled_camera() {
            self.send_movement(out);
        }

        // 6. End of tick.
        out.push(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
    }

    /// Movement send while riding (a passenger never sends position packets: the server owns
    /// its position): `LocalPlayer.sendChanges` sends `MovePlayerPacket.Rot` every tick
    /// unconditionally — the server reads a rider's rotation every tick. Sneak dismounts arrive
    /// as `player_input`'s shift flag, answered by the server with `set_passengers`.
    fn send_rotation(&mut self, out: &mut Vec<ServerboundPlay>) {
        let s = &self.state;
        if !(s.yaw.is_finite() && s.pitch.is_finite()) {
            tracing::error!("non-finite player state, not sent: {s:?}");
            return;
        }
        let mut flags = 0;
        if s.on_ground {
            flags |= move_flags::ON_GROUND;
        }
        if s.horizontal_collision {
            flags |= move_flags::HORIZONTAL_COLLISION;
        }
        out.push(ServerboundPlay::MovePlayerRot(sb::MovePlayerRot {
            yaw: s.yaw,
            pitch: s.pitch,
            flags,
        }));
        self.sent.yaw = s.yaw;
        self.sent.pitch = s.pitch;
        self.sent.on_ground = s.on_ground;
        self.sent.horizontal_collision = s.horizontal_collision;
    }

    fn send_movement(&mut self, out: &mut Vec<ServerboundPlay>) {
        // A teleport whose accept has not gone out yet: the first movement would name the
        // post-teleport position before the server has taken the accept (vanilla's
        // `handleMovePlayer` answers before the next `sendPosition` runs). It goes out with
        // the accept's own tick, `xLast` still holding the pre-teleport position.
        if self.pending_accept.is_some() {
            return;
        }
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
    /// The `#strider_warm_blocks` resolution for the strider we steer, from the driver.
    warm_blocks: Option<&'a dyn crate::item_based_steering::WarmBlocks>,
}

impl TickCtx<'_> {
    /// The block-state table when the tick's `blocks` came from game data (`None` for the test
    /// worlds that only answer physics per state id).
    fn blocks(&self) -> Option<&BlockStates> {
        self.env.map(|env| &env.data.blocks)
    }
}

mod camera;
pub mod climbable;
mod creative_flight;
pub mod interact;
mod level_load;
mod riding;
pub mod script;
pub mod spectator;
mod teleport_accept;
#[cfg(test)]
mod tests;
mod unloaded_chunk;
