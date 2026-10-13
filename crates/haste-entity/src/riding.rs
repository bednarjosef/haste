//! Riding bookkeeping (entities): which entity is a passenger of which vehicle, from
//! `set_passengers`, and where a passenger's seat is. One [`Riding`] lives inside the
//! [`EntityStore`](crate::store::EntityStore); a passenger list replaces the previous one, and
//! entities removed from the store drop out of both sides.

use ahash::AHashMap;
use haste_core::ids::EntityId;

/// Where a passenger's feet sit, in blocks above the vehicle's position: explicit values for
/// the vanilla vehicles (minecraft.wiki "Riding"), the vehicle's own height for everything
/// else. `type_name` is the entity type's registry path (e.g. `minecart`, `oak_boat`);
/// unknown types (mods) get the default, never a panic.
///
/// These are the wiki's seat heights for the vehicles players actually ride; boats, rafts and
/// their chest variants share one seat. They should be replaced by a recorder-mod dump of the
/// real attachment points when oracles extends the entity dump (AGENTS.md rule 7: data comes
/// from the game, not hand-written tables).
pub fn passenger_offset(type_name: &str, dims: (f32, f32)) -> f64 {
    if is_boat_family(type_name) {
        return 0.15;
    }
    match type_name {
        "minecart" => 0.1,
        "horse" | "skeleton_horse" | "zombie_horse" | "donkey" | "mule" => 0.6,
        "camel" => 1.1,
        "pig" => 0.4,
        "strider" => 0.6,
        _ => f64::from(dims.1),
    }
}

/// Boats, rafts and their chest variants (one per wood, e.g. `oak_boat`, `mangrove_chest_raft`)
/// share the boat seat.
fn is_boat_family(type_name: &str) -> bool {
    type_name == "boat"
        || type_name == "raft"
        || type_name.ends_with("_boat")
        || type_name.ends_with("_raft")
}

/// Passenger lists per vehicle and the reverse map, kept in step by
/// [`Riding::set_passengers`] and [`Riding::on_removed`].
#[derive(Default)]
pub struct Riding {
    vehicle_of: AHashMap<EntityId, EntityId>,
    passengers_of: AHashMap<EntityId, Vec<EntityId>>,
}

impl Riding {
    pub fn new() -> Riding {
        Riding::default()
    }

    /// `set_passengers` for one vehicle: the list replaces the previous one (passengers that
    /// dropped out lose their vehicle entry), and every passenger listed gains this vehicle.
    pub fn set_passengers(&mut self, vehicle: EntityId, passengers: &[EntityId]) {
        if let Some(old) = self.passengers_of.get(&vehicle) {
            for id in old {
                if !passengers.contains(id) {
                    self.vehicle_of.remove(id);
                }
            }
        }
        for id in passengers {
            self.vehicle_of.insert(*id, vehicle);
        }
        match self.passengers_of.get_mut(&vehicle) {
            // Reuse the list's allocation: this runs once per packet, not per tick.
            Some(list) => {
                list.clear();
                list.extend_from_slice(passengers);
            }
            None if !passengers.is_empty() => {
                self.passengers_of.insert(vehicle, passengers.to_vec());
            }
            None => {}
        }
        if passengers.is_empty() {
            self.passengers_of.remove(&vehicle);
        }
    }

    /// Entities removed from the store drop out of both sides: their own passengers are
    /// freed, and they are dropped from the passenger list of the vehicle they rode.
    pub fn on_removed(&mut self, ids: &[EntityId]) {
        for id in ids {
            // Stop riding: drop out of the vehicle's list.
            if let Some(vehicle) = self.vehicle_of.remove(id) {
                if let Some(list) = self.passengers_of.get_mut(&vehicle) {
                    list.retain(|p| p != id);
                    if list.is_empty() {
                        self.passengers_of.remove(&vehicle);
                    }
                }
            }
            // Stop being a vehicle: our passengers are freed.
            if let Some(passengers) = self.passengers_of.remove(id) {
                for p in passengers {
                    self.vehicle_of.remove(&p);
                }
            }
        }
    }

    /// The vehicle an entity rides, if any.
    pub fn vehicle_of(&self, id: EntityId) -> Option<EntityId> {
        self.vehicle_of.get(&id).copied()
    }

    /// The passengers of a vehicle, in seat order.
    pub fn passengers_of(&self, id: EntityId) -> Option<&[EntityId]> {
        self.passengers_of.get(&id).map(|v| v.as_slice())
    }

    /// Forgets everything (a fresh store after a respawn).
    pub fn clear(&mut self) {
        self.vehicle_of.clear();
        self.passengers_of.clear();
    }
}
