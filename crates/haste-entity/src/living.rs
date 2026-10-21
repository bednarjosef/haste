//! Whether a tracked entity is a living one (vanilla `instanceof LivingEntity`), for the client
//! side of a player's own attack: a critical hit only starts its visual on a living target
//! (`Player.canCriticalAttack`). The store decides living from the entity type (`store.rs`).

use haste_core::ids::EntityId;

use crate::store::EntityStore;

impl EntityStore {
    /// True for a tracked living entity; false for one not in the store.
    pub fn is_living(&self, id: EntityId) -> bool {
        self.get(id).is_some_and(|e| e.is_living)
    }
}
