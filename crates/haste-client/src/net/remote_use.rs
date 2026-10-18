//! A remote entity's item-use effects (vanilla `LivingEntity.tick` on the client runs
//! `updatingUsingItem` -> `updateUsingItem` -> `ItemStack.onUseTick` for entities the
//! server drives): the use steps the entity store records each tick, run through the
//! same `ConsumeEffects::emit` path the local player's use takes — the item particles
//! positioned from the entity's feet, eye and rotations.
//!
//! The sound half is drawn and then dropped: vanilla's client plays no local sound for a
//! remote entity's use — `Consumable.emitParticlesAndSounds` calls the entity's
//! `playSound`, whose excluded listener is that entity, and `ClientLevel.playSeededSound`
//! plays only when the excluded listener is the local player. The watcher hears the use
//! through the server's broadcast `minecraft:sound` packets, which the session records and
//! plays like any other (`Session::apply` -> `Sounds::apply`); emitting our own would
//! double it. The draws still run in vanilla's order, because the item particles' positions
//! come after them in the same sequence.

use haste_data::GameData;
use haste_entity::using_item::UseTick;

use super::Session;

impl Session {
    /// Runs the remote uses the entity store ticked (`UseTick`s from
    /// `EntityStore::tick_in_world`): each item's `minecraft:consumable` decides whether
    /// the use emits this tick (`Consumable.shouldEmitParticlesAndSounds` — no component,
    /// no effect), and the emit queues the item particles with the entity's own draw
    /// sequence. Unknown items emit nothing.
    pub fn apply_remote_use(&mut self, ticks: &[UseTick], data: &GameData) {
        for t in ticks {
            let Some(name) = data.registries.name("minecraft:item", t.item) else {
                continue;
            };
            let Some(consumable) = data.item_components.consumable(name) else {
                continue;
            };
            // `useItemRemaining` = the use duration minus the ticks it has run.
            let duration = consumable.consume_ticks();
            if !crate::player::interact::should_emit(duration, duration - t.used) {
                continue;
            }
            let fx = self.remote_use.entry(t.entity).or_default();
            fx.emit(
                &consumable,
                Some(t.item),
                crate::item_particles::EntityPose {
                    pos: t.pos,
                    eye_y: t.eye_y,
                    look: t.look,
                },
                &mut self.remote_use_sounds,
                &mut self.item_particle_requests,
            );
        }
        // See the module doc: the remote uses' own sound requests are never played.
        self.remote_use_sounds.clear();
    }
}

#[cfg(test)]
mod tests;
