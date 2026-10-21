//! The entity-event and hit-feedback entry points of `EntityStore`: one method per entity
//! status or packet that changes a tracked entity's client-side state. Moved out of
//! `store.rs` to keep that file within the size limit; a pure move.

use haste_core::ids::EntityId;

use crate::store::{EntityStore, HURT_TICKS};

impl EntityStore {
    /// `hurt_animation` / `damage_event`: (re)start the 10-tick red flash.
    pub fn mark_hurt(&mut self, id: EntityId) {
        if let Some(e) = self.get_mut(id) {
            e.hurt_ticks = HURT_TICKS;
        }
    }

    /// The death event (entity status 3, `LivingEntity.handleEntityEvent`): starts the death
    /// animation (`deathTime` counting from the next tick) and holds the hurt tint until the
    /// server's `remove_entities`. Health reaching 0 in the metadata starts it too
    /// (`LivingEntity.isDeadOrDying` -> `tickDeath`); the first of the two wins.
    pub fn mark_dead(&mut self, id: EntityId) {
        if let Some(e) = self.get_mut(id) {
            e.start_death();
        }
    }

    /// Entity event 4: a melee attack animation (`IronGolem.handleEntityEvent` sets
    /// `attackAnimationTick = 10`; several mobs share the status).
    pub fn mark_attack(&mut self, id: EntityId) {
        if let Some(e) = self.get_mut(id) {
            e.attack_ticks = 10;
        }
    }

    /// Entity event 32 (`ArmorStand.handleEntityEvent`): the armour stand was hit; its
    /// render-side wobble counts from now (`lastHit = gameTime`).
    pub fn mark_hit(&mut self, id: EntityId) {
        if let Some(e) = self.get_mut(id) {
            e.last_hit = e.tick_count as i64;
        }
    }

    /// Entity event 70 (`MinecartTNT.handleEntityEvent` -> `primeFuse`): the TNT cart's fuse
    /// starts at 80 ticks and counts down client-side (`MinecartTNT.tick`).
    pub fn prime_tnt_minecart(&mut self, id: EntityId) {
        if let Some(e) = self.get_mut(id) {
            crate::minecart::prime(&mut e.metadata);
        }
    }
}
