//! The entity events the server sends by id (`ClientboundEntityEventPacket` and the hurt, attack
//! and TNT-cart cues): each one starts the client-side animation it names.

use haste_core::EntityId;

use crate::store::{EntityStore, HURT_TICKS};

impl EntityStore {
    /// `hurt_animation` / `damage_event`: (re)start the 10-tick red flash.
    pub fn mark_hurt(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.hurt_ticks = HURT_TICKS;
            }
        }
    }

    /// The death event (entity status 3, `LivingEntity.handleEntityEvent`): starts the death
    /// animation (`deathTime` counting from the next tick) and holds the hurt tint until the
    /// server's `remove_entities`. Health reaching 0 in the metadata starts it too
    /// (`LivingEntity.isDeadOrDying` -> `tickDeath`); the first of the two wins.
    pub fn mark_dead(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.start_death();
            }
        }
    }

    /// Entity event 4: a melee attack animation (`IronGolem.handleEntityEvent` sets
    /// `attackAnimationTick = 10`; several mobs share the status).
    pub fn mark_attack(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.attack_ticks = 10;
            }
        }
    }

    /// Entity event 32 (`ArmorStand.handleEntityEvent`): the armour stand was hit; its
    /// render-side wobble counts from now (`lastHit = gameTime`).
    pub fn mark_hit(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                e.last_hit = e.tick_count as i64;
            }
        }
    }

    /// Entity event 70 (`MinecartTNT.handleEntityEvent` -> `primeFuse`): the TNT cart's fuse
    /// starts at 80 ticks and counts down client-side (`MinecartTNT.tick`).
    pub fn prime_tnt_minecart(&mut self, id: EntityId) {
        if let Some(i) = self.slot(id) {
            if let Some(e) = &mut self.slots[i as usize] {
                crate::minecart::prime(&mut e.metadata);
            }
        }
    }
}
