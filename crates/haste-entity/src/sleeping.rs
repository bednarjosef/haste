//! The leave-bed part of `LivingEntity.stopSleeping`, run for a remote player by the wake-up
//! animation (`ClientPacketListener.handleAnimate` action 0, `Player.stopSleepInBed`). The store
//! keeps the pose, not the sleeping position, so only the pose and dimensions change here: the bed
//! block's occupied flag and the stand-up position are not applied.

use haste_core::entity::Pose;
use haste_core::ids::EntityId;

use crate::store::EntityStore;

impl EntityStore {
    /// `LivingEntity.stopSleeping`'s pose change: a sleeping entity becomes standing and its
    /// dimensions follow. An entity that is not sleeping, or is not held, is left alone.
    pub fn wake_up(&mut self, id: EntityId) {
        let Some(&slot) = self.by_id.get(&id) else {
            return;
        };
        let Some(e) = self.slots[slot as usize].as_mut() else {
            return;
        };
        if e.pose != Pose::Sleeping {
            return;
        }
        e.pose = Pose::Standing;
        e.dimensions = self.dims.get(e.kind, Pose::Standing);
    }
}
