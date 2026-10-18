// The packets that feed the lightmap and the weather state: our own mob-effect durations,
// the boss-event flags, the lightning sky flash and the weather game events.

use haste_protocol::haste_core::EntityId;
use haste_protocol::ClientboundPlay;

use super::Session;

impl Session {
    /// The mob-effect, boss-event and weather game-event packets for our own entity; returns
    /// the packet when it is none of them.
    pub(super) fn apply_effects(
        &mut self,
        p: ClientboundPlay,
        own: EntityId,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            // The lightmap's effect inputs (night vision and darkness durations), boss-overlay
            // darkening and the lightning sky flash; these also ride the player's own handling.
            P::UpdateMobEffect(e) if e.entity_id == own.0 => {
                let ids = self.player.int.lightmap_effect_ids;
                self.lightmap.update_effect(ids, e.effect, e.duration);
            }
            P::RemoveMobEffect(e) if e.entity_id == own.0 => {
                let ids = self.player.int.lightmap_effect_ids;
                self.lightmap.remove_effect(ids, e.effect);
            }
            P::BossEvent(b) => self.lightmap.boss_event(b.id.as_u128(), &b.action),
            // The weather game events (`ClientPacketListener.handleGameEvent`):
            // start_raining resets the rain level to 0 (the server follows with
            // rain_level_change steps that ramp it up), stop_raining sets it to 1 (the
            // steps ramp it down), and the two level changes set rain/thunder directly.
            P::GameEvent(g) if g.event == 1 => self.weather.set_rain_level(0.0),
            P::GameEvent(g) if g.event == 2 => self.weather.set_rain_level(1.0),
            P::GameEvent(g) if g.event == 7 => self.weather.set_rain_level(g.value),
            P::GameEvent(g) if g.event == 8 => self.weather.set_thunder_level(g.value),
            // The world border (vanilla `ClientPacketListener`'s border handlers): the
            // interaction step gates its packets on it. The warning distance and time are
            // not modelled yet.
            P::InitializeBorder(b) => {
                self.world_border
                    .initialize(b.x, b.z, b.old_diameter, b.new_diameter, b.speed)
            }
            P::SetBorderCenter(c) => self.world_border.set_center(c.x, c.z),
            P::SetBorderLerpSize(s) => {
                self.world_border
                    .set_lerp_size(s.old_diameter, s.new_diameter, s.speed)
            }
            P::SetBorderSize(s) => self.world_border.set_size(s.diameter),
            _ => return Some(p),
        }
        None
    }
}
