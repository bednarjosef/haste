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

use glam::DVec3;
use haste_core::game::GameMode;
use haste_core::input::InputState;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{BlockPos, EntityId, Identifier};
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

/// `player_command` actions (wiki "Player Command").
pub mod command {
    pub const START_SPRINTING: i32 = 1;
    pub const STOP_SPRINTING: i32 = 2;
}

/// `player_abilities` flag bits (clientbound and serverbound).
pub mod ability_flags {
    pub const INVULNERABLE: i8 = 0x01;
    pub const FLYING: i8 = 0x02;
    pub const MAY_FLY: i8 = 0x04;
    pub const INSTABUILD: i8 = 0x08;
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
    /// Selected hotbar slot (0..8).
    pub held_slot: i32,
    /// Whether a `player_position` arrived since login/respawn.
    pub has_position: bool,
    /// Whether `player_loaded` was sent since login/respawn (movement is sent only after).
    pub loaded: bool,
    /// Counters for tests and the headless printout.
    pub teleports: u64,
    prev_pos: DVec3,
    prev_eye: f64,
    look_offset: (f32, f32),
    last_input_look: (f32, f32),
    sent: Sent,
    attribute_ids: Option<AttributeIds>,
    /// Last `update_attributes` value per attribute id (base + modifiers).
    attributes: Vec<cb::AttributeProperty>,
    attributes_dirty: bool,
    toggle_flight: bool,
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
            held_slot: 0,
            has_position: false,
            loaded: false,
            teleports: 0,
            prev_pos: DVec3::ZERO,
            prev_eye: 0.0,
            look_offset: (0.0, 0.0),
            last_input_look: (0.0, 0.0),
            sent: Sent::default(),
            attribute_ids: None,
            attributes: Vec::new(),
            attributes_dirty: false,
            toggle_flight: false,
        };
        p.state.on_ground = true;
        p.prev_eye = p.eye_height();
        p
    }

    /// Server rotations as an offset to add to the input layer's yaw/pitch (degrees).
    pub fn look_offset(&self) -> (f32, f32) {
        self.look_offset
    }

    /// Double-tap jump (creative/spectator): toggles flight on the next tick if allowed.
    pub fn toggle_flight(&mut self) {
        self.toggle_flight = true;
    }

    /// Sets the attribute registry ids (done automatically by [`LocalPlayer::tick`]).
    pub fn set_attribute_ids(&mut self, ids: AttributeIds) {
        self.attribute_ids = Some(ids);
        self.attributes_dirty = true;
    }

    fn eye_height(&self) -> f64 {
        self.attrs.eye_height_for(self.state.pose)
    }

    /// Eye position and look angles interpolated by partial tick, for the camera.
    pub fn camera(&self, partial_tick: f32) -> (DVec3, f32, f32) {
        let t = partial_tick.clamp(0.0, 1.0) as f64;
        let pos = self.prev_pos.lerp(self.state.pos, t);
        let eye = self.prev_eye + (self.eye_height() - self.prev_eye) * t;
        (
            pos + DVec3::new(0.0, eye, 0.0),
            self.state.yaw,
            self.state.pitch,
        )
    }

    /// Consumes the packets about the local player. Returns false for packets it does not handle.
    pub fn handle(&mut self, p: &ClientboundPlay, out: &mut Vec<ServerboundPlay>) -> bool {
        use ClientboundPlay as P;
        match p {
            P::PlayerPosition(t) => {
                self.teleport(t);
                out.push(ServerboundPlay::AcceptTeleportation(
                    sb::AcceptTeleportation {
                        teleport_id: t.teleport_id,
                        pos: self.state.pos,
                        yaw: self.state.yaw,
                        pitch: self.state.pitch,
                    },
                ));
                // 26.3's accept_teleportation carries the resulting position and rotation; an
                // extra move_player_pos_rot right after it (what older clients sent) gets the
                // client kicked with invalid_player_movement after a relative `/tp ~ ~10 ~`
                // (observed on the vanilla 26.3 server). The accepted position counts as sent.
                self.mark_sent_position(self.state.on_ground, self.state.horizontal_collision);
                if !self.loaded {
                    // Observed on the 26.3 server: a movement packet between
                    // accept_teleportation and player_loaded gets the client kicked
                    // (invalid_player_movement), so the first move goes out next tick.
                    out.push(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}));
                    self.loaded = true;
                }
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
                self.sent = Sent::default();
            }
            _ => return false,
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
        self.set_look(yaw, pitch);
        tracing::debug!("after teleport {:?}", self.state);
        self.prev_pos = self.state.pos;
        self.prev_eye = self.eye_height();
        self.has_position = true;
        self.teleports += 1;
    }

    fn mark_sent_position(&mut self, on_ground: bool, horizontal_collision: bool) {
        self.sent.pos = self.state.pos;
        self.sent.yaw = self.state.yaw;
        self.sent.pitch = self.state.pitch;
        self.sent.on_ground = on_ground;
        self.sent.horizontal_collision = horizontal_collision;
        self.sent.reminder = 0;
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
    }

    /// One 20 Hz tick against the real world and game data.
    pub fn tick(
        &mut self,
        input: &InputState,
        world: &haste_world::World,
        data: &haste_data::GameData,
        out: &mut Vec<ServerboundPlay>,
    ) {
        if self.attribute_ids.is_none() {
            if let Some(ids) = AttributeIds::from_data(data) {
                self.set_attribute_ids(ids);
            }
        }
        self.tick_with(input, world, &data.shapes, data, out);
    }

    /// One 20 Hz tick over any world view (tests use `haste_physics::testing`).
    pub fn tick_with(
        &mut self,
        input: &InputState,
        world: &dyn BlockView,
        shapes: &dyn CollisionShapes,
        blocks: &dyn BlockPhysics,
        out: &mut Vec<ServerboundPlay>,
    ) {
        self.apply_attributes();
        self.prev_pos = self.state.pos;
        self.prev_eye = self.eye_height();
        self.last_input_look = (input.yaw, input.pitch);
        if !(self.has_position && self.loaded) {
            return;
        }

        // 1. player_input when the held keys change (raw keys, as pressed).
        let flags = input_bits(input);
        if self.sent.input != Some(flags) {
            out.push(ServerboundPlay::PlayerInput(sb::PlayerInput { flags }));
            self.sent.input = Some(flags);
        }

        // 2. Physics. The player does not move while its chunk is not loaded.
        let mut phys_input = *input;
        phys_input.yaw = input.yaw + self.look_offset.0;
        phys_input.pitch = (input.pitch + self.look_offset.1).clamp(-90.0, 90.0);
        if self.food <= SPRINT_FOOD_LEVEL && !self.abilities.may_fly {
            phys_input.sprint = false;
            self.state.sprinting = false;
        }
        let was_flying = self.state.flying;
        if std::mem::take(&mut self.toggle_flight) && self.abilities.may_fly {
            self.state.flying = !self.state.flying;
        }
        if !self.abilities.may_fly {
            self.state.flying = false;
        }
        let chunk = BlockPos::from_dvec3(self.state.pos).chunk();
        if world.is_chunk_loaded(chunk) {
            haste_physics::tick(
                &mut self.state,
                &phys_input,
                &self.attrs,
                world,
                shapes,
                blocks,
            );
        } else {
            self.state.yaw = phys_input.yaw;
            self.state.pitch = phys_input.pitch;
        }

        // 3. Flight started or stopped (toggle or landing).
        if self.state.flying != was_flying {
            let mut f = 0;
            if self.state.flying {
                f |= ability_flags::FLYING;
            }
            out.push(ServerboundPlay::PlayerAbilities(sb::PlayerAbilities {
                flags: f,
            }));
        }

        // 4. Sprint state.
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

        // 5. Movement.
        self.send_movement(out);

        // 6. End of tick.
        out.push(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
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
            (true, true) => out.push(ServerboundPlay::MovePlayerPosRot(sb::MovePlayerPosRot {
                pos: s.pos,
                yaw: s.yaw,
                pitch: s.pitch,
                flags,
            })),
            (true, false) => out.push(ServerboundPlay::MovePlayerPos(sb::MovePlayerPos {
                pos: s.pos,
                flags,
            })),
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

pub mod script;
#[cfg(test)]
mod tests;
