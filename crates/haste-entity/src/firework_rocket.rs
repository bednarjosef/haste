//! The firework rockets in flight, for the client's trail (vanilla `FireworkRocketEntity.tick`'s
//! client branch). Kept apart from `store.rs`, which is over its size limit.

use glam::DVec3;

use crate::store::EntityStore;

impl EntityStore {
    /// The position and velocity of each firework rocket tracked, for this tick's trail. No
    /// allocation: one pair per rocket, straight from the slots.
    pub fn firework_rockets(&self) -> impl Iterator<Item = (DVec3, DVec3)> + '_ {
        self.slots
            .iter()
            .flatten()
            .filter(|slot| crate::thrown_item::is_firework_rocket(slot.kind))
            .map(|slot| (slot.pos, slot.velocity))
    }
}
