//! `Entity.getEyeY`: the height of an entity's eyes, from its type's eye height in the dimensions
//! table. The store keeps the standing pose only, so the sneaking and swimming eye heights are not
//! modelled here.

use haste_core::ids::EntityId;

use crate::store::EntityStore;

impl EntityStore {
    /// `Entity.getEyeY()` for the entity `id`: its feet's y plus the standing eye height of its
    /// type. None when the store does not hold the entity.
    pub fn eye_y(&self, id: EntityId) -> Option<f64> {
        self.get(id)
            .map(|e| e.pos.y + f64::from(self.dims.eye_height(e.kind, false)))
    }
}
