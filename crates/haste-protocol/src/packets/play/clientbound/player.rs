use super::*;

packet! {
    /// Joins the game.
    Login = "minecraft:login" {
        /// The player's entity id.
        entity_id: i32,
        /// Hardcore.
        hardcore: bool,
        /// All dimension names on the server.
        dimension_names: Vec<Identifier>,
        /// Ignored by clients.
        max_players: i32 as VarInt,
        /// Server view distance.
        view_distance: i32 as VarInt,
        /// Server simulation distance.
        simulation_distance: i32 as VarInt,
        /// Reduced debug info.
        reduced_debug_info: bool,
        /// Show the respawn screen.
        enable_respawn_screen: bool,
        /// Limited crafting.
        do_limited_crafting: bool,
        /// Dimension and game mode.
        spawn: SpawnInfo,
        /// Online mode (26.x field per the wiki).
        online_mode: bool,
        /// Secure chat enforced.
        enforces_secure_chat: bool,
    }
}
packet! {
    /// Teleports the local player; answered with `accept_teleportation`.
    PlayerPosition = "minecraft:player_position" {
        /// Id to confirm.
        teleport_id: i32 as VarInt,
        /// Position (absolute or relative per `flags`).
        pos: DVec3,
        /// Velocity.
        velocity: DVec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// [`teleport_flags`] bits.
        flags: i32,
    }
}
packet! {
    /// Respawn or dimension change.
    Respawn = "minecraft:respawn" {
        /// Dimension and game mode.
        spawn: SpawnInfo,
        /// 0x01 keep attributes, 0x02 keep metadata.
        data_kept: u8,
    }
}
packet! {
    /// Experience.
    SetExperience = "minecraft:set_experience" {
        /// Bar progress 0..1.
        bar: f32,
        /// Level.
        level: i32 as VarInt,
        /// Total experience.
        total: i32 as VarInt,
    }
}

packet! {
    /// Health and food.
    SetHealth = "minecraft:set_health" {
        /// Health (0 = dead, 20 = full).
        health: f32,
        /// Food 0..20.
        food: i32 as VarInt,
        /// Saturation.
        saturation: f32,
    }
}
packet! {
    /// Selected hotbar slot.
    SetHeldSlot = "minecraft:set_held_slot" {
        /// Slot 0..8.
        slot: i32 as VarInt,
    }
}
/// One statistic in `award_stats`.
#[derive(Clone, Debug, PartialEq)]
pub struct Statistic {
    /// Index into `minecraft:stat_type`.
    pub category: i32,
    /// Id in the category's registry.
    pub stat: i32,
    /// New value.
    pub value: i32,
}
impl Wire for Statistic {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            category: r.var_int()?,
            stat: r.var_int()?,
            value: r.var_int()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.category);
        w.var_int(self.stat);
        w.var_int(self.value);
    }
}

packet! {
    /// Statistics (answer to `client_command` request stats).
    AwardStats = "minecraft:award_stats" {
        /// Changed statistics.
        stats: Vec<Statistic>,
    }
}
packet! {
    /// Player abilities.
    PlayerAbilities = "minecraft:player_abilities" {
        /// 0x01 invulnerable, 0x02 flying, 0x04 may fly, 0x08 instant build.
        flags: i8,
        /// Flying speed.
        flying_speed: f32,
        /// FOV modifier.
        fov_modifier: f32,
    }
}

packet! {
    /// End of combat (unused by vanilla clients).
    PlayerCombatEnd = "minecraft:player_combat_end" {
        /// Duration in ticks.
        duration: i32 as VarInt,
    }
}

packet! {
    /// Start of combat (unused by vanilla clients).
    PlayerCombatEnter = "minecraft:player_combat_enter" {}
}

packet! {
    /// The player died: death screen.
    PlayerCombatKill = "minecraft:player_combat_kill" {
        /// Player entity id.
        player_id: i32 as VarInt,
        /// Death message.
        message: Component,
    }
}
packet! {
    /// Locator bar waypoint. Kept raw: the wiki and Pumpkin disagree on the
    /// identifier, colour and waypoint type encodings.
    Waypoint = "minecraft:waypoint" {
        /// Raw body.
        body: Bytes as Rest,
    }
}
