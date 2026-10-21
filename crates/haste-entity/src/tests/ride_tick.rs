use super::*;
// ---------------------------------------------------------------------------
// The riders' ticks (`ClientLevel.tickPassenger` -> `Entity.rideTick` -> `positionRider`).
// The entities use a registry index no dump names, so the seat rules see an unknown type and
// fall back to the vehicle's own box (the type-specific seats are covered in `riding.rs`).
// ---------------------------------------------------------------------------

/// A spawn whose type index the registry does not name (the seat rules then see "").
fn unnamed(id: i32, pos: DVec3) -> EntitySpawn {
    EntitySpawn {
        kind: RegistryIndex(u32::MAX),
        ..spawn(id, pos)
    }
}

/// Where a rider of `vehicle` belongs after the tick: the vehicle's position plus its seat
/// offset for `seat` of `passengers`, turned by the vehicle's yaw.
fn seat_position(s: &EntityStore, vehicle: EntityId, seat: usize, passengers: usize) -> DVec3 {
    let v = s.get(vehicle).unwrap();
    let offset = passenger_seat(SeatQuery {
        vehicle_type: "",
        vehicle_dims: v.dimensions,
        seat,
        passengers,
        passenger: PassengerKind::Player,
        passenger_type: "",
    });
    v.pos + offset.rotated(v.yaw)
}

/// A two-seat vehicle (id 1) with a player-like rider (id 2) in seat 0 and an animal-like
/// rider (id 3) in seat 1, at `pos` facing `yaw`.
fn two_seat_vehicle(pos: DVec3, yaw: f32) -> EntityStore {
    let mut s = store();
    s.spawn(&EntitySpawn {
        yaw,
        ..unnamed(1, pos)
    });
    s.spawn(&unnamed(2, pos));
    s.spawn(&unnamed(3, pos));
    s.set_passengers(EntityId(1), &[EntityId(2), EntityId(3)]);
    s
}

#[test]
fn riders_are_placed_on_their_seats_in_the_same_tick() {
    let mut s = two_seat_vehicle(DVec3::new(10.0, 64.0, 20.0), 0.0);
    s.tick();
    assert_eq!(
        s.get(EntityId(2)).unwrap().pos,
        seat_position(&s, EntityId(1), 0, 2)
    );
    assert_eq!(
        s.get(EntityId(3)).unwrap().pos,
        seat_position(&s, EntityId(1), 1, 2)
    );
}

#[test]
fn each_rider_ticks_once_per_tick_through_its_vehicle() {
    let mut s = two_seat_vehicle(DVec3::new(10.0, 64.0, 20.0), 0.0);
    s.tick();
    s.tick();
    // The vehicle and both riders each ran `commonTick` once per tick, never twice.
    for id in [1, 2, 3] {
        assert_eq!(s.get(EntityId(id)).unwrap().tick_count, 2, "entity {id}");
    }
}

#[test]
fn a_rider_follows_a_moving_vehicle_and_its_previous_position_is_the_old_seat() {
    let mut s = two_seat_vehicle(DVec3::new(10.0, 64.0, 20.0), 0.0);
    s.tick();
    let seat_before = s.get(EntityId(2)).unwrap().pos;
    let vehicle_before = s.get(EntityId(1)).unwrap().pos;
    s.move_delta(EntityId(1), DVec3::new(0.5, 0.0, -0.25), false);
    s.tick();
    let rider = s.get(EntityId(2)).unwrap();
    // `Entity.setOldPosAndRot` ran at the start of the tick, so the previous position is the
    // seat the rider held last tick; the current one is the seat on the vehicle as it now is.
    assert_eq!(rider.prev_pos, seat_before);
    assert_eq!(rider.pos, seat_position(&s, EntityId(1), 0, 2));
    // The rider moves exactly as far as its vehicle did this tick.
    let vehicle_step = s.get(EntityId(1)).unwrap().pos - vehicle_before;
    assert_ne!(vehicle_step, DVec3::ZERO);
    assert_eq!(rider.pos - seat_before, vehicle_step);
}

#[test]
fn a_rider_follows_a_turning_vehicle() {
    let pos = DVec3::new(10.0, 64.0, 20.0);
    let mut s = two_seat_vehicle(pos, 0.0);
    s.tick();
    // The vehicle turns a quarter circle; the seats turn with it (`Entity.positionRider` takes
    // the vehicle's yaw), so each rider lands on the seat of the new yaw.
    s.teleport(EntityId(1), pos, 90.0, 0.0, false);
    for _ in 0..3 {
        s.tick();
    }
    let v = s.get(EntityId(1)).unwrap();
    assert_eq!(v.yaw, 90.0);
    assert_eq!(
        s.get(EntityId(2)).unwrap().pos,
        seat_position(&s, EntityId(1), 0, 2)
    );
    assert_eq!(
        s.get(EntityId(3)).unwrap().pos,
        seat_position(&s, EntityId(1), 1, 2)
    );
}

#[test]
fn a_packet_does_not_move_a_rider_off_its_seat() {
    let mut s = two_seat_vehicle(DVec3::new(10.0, 64.0, 20.0), 0.0);
    s.tick();
    let seat = seat_position(&s, EntityId(1), 0, 2);
    // Position packets for the rider itself: a short move, then an absolute position within
    // the snap distance (both interpolated), and a rotation. None of them may hold the rider
    // off the seat once the tick has placed it.
    s.move_delta(EntityId(2), DVec3::new(1.0, 0.0, 0.0), true);
    s.teleport(EntityId(2), DVec3::new(12.0, 70.0, 22.0), 45.0, 10.0, true);
    s.set_rotation(EntityId(2), 135.0, -20.0, true);
    s.tick();
    assert_eq!(s.get(EntityId(2)).unwrap().pos, seat);
}

#[test]
fn a_rider_that_is_not_in_the_store_is_skipped() {
    // A vehicle may list passengers the store never spawned (they arrive later); the tick
    // places the ones it has and leaves the vehicle alone.
    let mut s = store();
    s.spawn(&unnamed(1, DVec3::new(0.0, 64.0, 0.0)));
    s.spawn(&unnamed(3, DVec3::new(0.0, 64.0, 0.0)));
    s.set_passengers(EntityId(1), &[EntityId(9), EntityId(3)]);
    s.tick();
    assert_eq!(
        s.get(EntityId(3)).unwrap().pos,
        seat_position(&s, EntityId(1), 1, 2)
    );
    assert_eq!(s.get(EntityId(1)).unwrap().tick_count, 1);
}
