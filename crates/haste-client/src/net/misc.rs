// The leftover packets: the world clock (`set_time`), the server's `disconnect` reason and
// the sound packets recorded for the game thread's sound output.

use haste_protocol::{ClientboundPlay, SyncedRegistries};

use super::{overworld_clock_id, EntitySink, Session};

impl Session {
    /// The `set_time`, `disconnect` and sound packets; returns the packet when it is none of
    /// them.
    pub(super) fn apply_misc<E: EntitySink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        entities: &mut E,
        regs: &SyncedRegistries,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            P::SetTime(t) => {
                entities.set_world_age(t.world_age);
                self.world_age = t.world_age;
                // The day clock is `minecraft:overworld` of `minecraft:world_clock` (the
                // End has its own); `clocks` comes in HashMap order, so look the id up.
                if let Some(c) = t
                    .clocks
                    .iter()
                    .find(|c| c.clock == overworld_clock_id(regs))
                    .or_else(|| t.clocks.first())
                {
                    self.time_of_day = Some(c.time);
                }
            }
            P::Disconnect(d) => {
                tracing::warn!("server disconnected us: {}", d.reason.to_plain(&|_| None))
            }
            P::Sound(_) | P::SoundEntity(_) | P::StopSound(_) => {
                if let Some(request) = crate::sound::record(&p) {
                    self.counters.sound_packets += 1;
                    self.sound_requests.push(request);
                }
            }
            _ => return Some(p),
        }
        None
    }
}
