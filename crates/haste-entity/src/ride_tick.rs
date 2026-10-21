//! The riders' ticks: `ClientLevel.tickNonPassenger` ticks every entity that is not a passenger,
//! and `ClientLevel.tickPassenger` ticks each of its passengers right after it, in seat order,
//! recursively. A passenger ticks its own body (`Entity.commonTick`, then `rideTick`: `tick()`),
//! and then its vehicle places it on its seat (`Entity.positionRider`), so a rider follows the
//! vehicle it is on in the same tick the vehicle moved.
use ahash::AHashMap;
use haste_core::ids::{EntityId, RegistryIndex};

use crate::registry::entity_type_name;
use crate::riding::{passenger_seat, PassengerKind, Riding, SeatQuery};
use crate::store::tick::{tick_entity, TickEnv};
use crate::store::Entity;

/// How deep a passenger chain is ticked. Vanilla's chains are finite; a passenger list that
/// names its own vehicle back (a malformed packet) must not recurse without end.
const MAX_RIDE_DEPTH: u32 = 16;

/// The entity storage the riders' ticks walk: the slots, the id index and the riding lists.
pub(crate) struct RideTree<'a> {
    pub slots: &'a mut [Option<Entity>],
    pub by_id: &'a AHashMap<EntityId, u32>,
    pub riding: &'a Riding,
}

impl RideTree<'_> {
    /// `ClientLevel.tickNonPassenger` over every stored entity: each root ticks, then its
    /// passengers (recursively). A passenger is never ticked on its own.
    pub(crate) fn tick_all(&mut self, env: &mut TickEnv<'_>) {
        let riding = self.riding;
        for index in 0..self.slots.len() {
            let Some(slot) = self.slots[index].as_mut() else {
                continue;
            };
            if riding.vehicle_of(slot.id).is_some() {
                continue;
            }
            tick_entity(slot, env, false);
            self.tick_passengers(index, 0, env);
        }
    }

    /// `ClientLevel.tickPassenger` for the passengers of the vehicle in slot `vehicle`: each
    /// passenger ticks, then takes its seat on the vehicle as it is now (`rideTick` ->
    /// `positionRider`), then its own passengers are ticked.
    fn tick_passengers(&mut self, vehicle: usize, depth: u32, env: &mut TickEnv<'_>) {
        if depth >= MAX_RIDE_DEPTH {
            return;
        }
        let riding = self.riding;
        let by_id = self.by_id;
        let Some(vehicle_entity) = self.slots[vehicle].as_ref() else {
            return;
        };
        let Some(passengers) = riding.passengers_of(vehicle_entity.id) else {
            return;
        };
        let vehicle_pos = vehicle_entity.pos;
        let vehicle_yaw = vehicle_entity.yaw;
        let vehicle_kind = vehicle_entity.kind;
        let vehicle_dims = vehicle_entity.dimensions;
        for (seat, id) in passengers.iter().enumerate() {
            let Some(&index) = by_id.get(id) else {
                continue;
            };
            let index = index as usize;
            let Some(passenger) = self.slots[index].as_mut() else {
                continue;
            };
            tick_entity(passenger, env, true);
            let offset = seat_offset(
                vehicle_kind,
                vehicle_dims,
                seat,
                passengers.len(),
                passenger,
            );
            passenger.pos = vehicle_pos + offset.rotated(vehicle_yaw);
            self.tick_passengers(index, depth + 1, env);
        }
    }
}

/// The seat offset of `passenger` in a vehicle's frame (`Entity.positionRider`'s point, before
/// the vehicle's yaw turns it).
fn seat_offset(
    vehicle_kind: RegistryIndex,
    vehicle_dims: (f32, f32),
    seat: usize,
    passengers: usize,
    passenger: &Entity,
) -> crate::riding::SeatPoint {
    passenger_seat(SeatQuery {
        vehicle_type: type_path(vehicle_kind),
        vehicle_dims,
        seat,
        passengers,
        passenger: passenger_kind(passenger),
        passenger_type: type_path(passenger.kind),
    })
}

/// A registry id's path (`minecraft:oak_boat` -> `oak_boat`); empty for an unknown id.
fn type_path(kind: RegistryIndex) -> &'static str {
    entity_type_name(kind).map_or("", |name| name.split_once(':').map_or(name, |(_, path)| path))
}

/// What a passenger is, for the seat rules: a player, an `Animal`, or a villager or wandering
/// trader (`AbstractMinecart`'s lowered seat). Everything else counts as a player.
fn passenger_kind(passenger: &Entity) -> PassengerKind {
    if passenger.is_player_type {
        return PassengerKind::Player;
    }
    match type_path(passenger.kind) {
        "villager" | "wandering_trader" => PassengerKind::Villager,
        path if is_animal(path) => PassengerKind::Animal,
        _ => PassengerKind::Player,
    }
}

/// The `Animal` subclasses of vanilla 26.3 (their registry paths), the ones a boat or a camel
/// seats differently from a player.
const ANIMAL_TYPES: &[&str] = &[
    "armadillo",
    "axolotl",
    "bee",
    "camel",
    "cat",
    "chicken",
    "cow",
    "donkey",
    "fox",
    "frog",
    "goat",
    "happy_ghast",
    "hoglin",
    "horse",
    "llama",
    "mooshroom",
    "mule",
    "ocelot",
    "panda",
    "parrot",
    "pig",
    "polar_bear",
    "rabbit",
    "sheep",
    "skeleton_horse",
    "sniffer",
    "strider",
    "trader_llama",
    "turtle",
    "wolf",
    "zombie_horse",
];

fn is_animal(path: &str) -> bool {
    ANIMAL_TYPES.contains(&path)
}
