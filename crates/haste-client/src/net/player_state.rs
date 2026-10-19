// The player-state packets the session keeps on top of the player's own handling: the
// login/respawn lifecycle (`start_configuration`, `transfer`, `login`, `respawn`) and the
// join teleports. These run even when the player handled the packet itself.

use haste_entity::riding::SeatPoint;
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::{ClientboundPlay, SyncedRegistries};

use super::{EntitySink, Session, WorldSink};

impl Session {
    /// The `login`, `respawn` and player-position packets; returns the packet when it is
    /// none of them.
    pub(super) fn apply_player_state<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        world: &mut W,
        entities: &mut E,
        regs: &SyncedRegistries,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            P::StartConfiguration(_) | P::Transfer(_) => self.new_world_pending = true,
            P::Login(l) => {
                self.counters.got_login = true;
                self.player.game_mode =
                    GameMode::from_id(l.spawn.game_mode).unwrap_or(self.player.game_mode);
                // `ClientLevel.seaLevel` comes with the spawn info (biome precipitation's
                // snow line hangs off it).
                self.sea_level = l.spawn.sea_level;
                // The spawn info's hashed seed is `BiomeManager`'s zoom seed.
                world.set_biome_zoom_seed(l.spawn.hashed_seed);
                self.is_flat = l.spawn.is_flat;
                self.view_distance = l.view_distance;
            }
            P::Respawn(r) => {
                self.counters.respawns += 1;
                let dim = regs
                    .dimension_info(r.spawn.dimension_type, &r.spawn.dimension)
                    .unwrap_or_else(|| DimensionInfo {
                        name: r.spawn.dimension.clone(),
                        ..DimensionInfo::overworld()
                    });
                self.sea_level = r.spawn.sea_level;
                self.is_flat = r.spawn.is_flat;
                world.reset(&dim);
                // The spawn info's hashed seed is `BiomeManager`'s zoom seed (after the reset).
                world.set_biome_zoom_seed(r.spawn.hashed_seed);
                entities.clear();
                self.entity_ids.clear();
                self.breaking.clear();
                self.lids.clear();
                self.bells.clear();
                self.pots.clear();
                self.fireworks.clear();
                self.attached_firework = None;
                self.dimension = dim;
                // A respawn is a new body: no vehicle, no riding. A new level also starts
                // with vanilla's default border until its initialize_border arrives.
                self.entity_attributes.clear();
                self.world_border = crate::world_border::WorldBorder::default();
                // And a new player in vanilla's sense (`LocalPlayer` is rebuilt): the
                // hurt tilt and blink window end, the first `set_health` of the new
                // body only stores the health.
                self.hurt_ticks = 0;
                self.damage_cooldown = 0;
                self.health_seen = false;
                self.player
                    .set_vehicle(None, SeatPoint::ZERO, None, None, None);
            }
            P::PlayerPosition(p) => {
                // The server's position corrections a player feels as rubber-banding;
                // info (with the tick) so a scripted run can count them.
                tracing::info!(
                    "player_position: teleport to ({:.1}, {:.1}, {:.1}) at tick {}",
                    p.pos.x,
                    p.pos.y,
                    p.pos.z,
                    self.tick
                );
                self.counters.teleports += 1;
            }
            _ => return Some(p),
        }
        None
    }
}
