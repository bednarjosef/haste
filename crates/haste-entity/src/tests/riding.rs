use super::*;
// ---------------------------------------------------------------------------
// Riding: set_passengers bookkeeping and the seat offset.
// ---------------------------------------------------------------------------

#[test]
fn set_passengers_mounts_replaces_and_dismounts() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO)); // vehicle
    s.spawn(&spawn(2, DVec3::ZERO));
    s.spawn(&spawn(3, DVec3::ZERO));
    // Mount two passengers.
    s.set_passengers(EntityId(1), &[EntityId(2), EntityId(3)]);
    assert_eq!(s.vehicle_of(EntityId(2)), Some(EntityId(1)));
    assert_eq!(s.vehicle_of(EntityId(3)), Some(EntityId(1)));
    assert_eq!(
        s.passengers_of(EntityId(1)).unwrap(),
        &[EntityId(2), EntityId(3)]
    );
    // A passenger list replaces the previous one: 3 stays, 2 is dropped.
    s.set_passengers(EntityId(1), &[EntityId(3)]);
    assert_eq!(s.vehicle_of(EntityId(2)), None);
    assert_eq!(s.vehicle_of(EntityId(3)), Some(EntityId(1)));
    assert_eq!(s.passengers_of(EntityId(1)).unwrap(), &[EntityId(3)]);
    // An empty list dismounts everyone.
    s.set_passengers(EntityId(1), &[]);
    assert_eq!(s.vehicle_of(EntityId(3)), None);
    assert!(s.passengers_of(EntityId(1)).is_none());
    // A vehicle's list can also name entities never spawned: the bookkeeping holds them
    // (the server does this between the spawn and the set_passengers arriving).
    s.set_passengers(EntityId(1), &[EntityId(9)]);
    assert_eq!(s.vehicle_of(EntityId(9)), Some(EntityId(1)));
}

#[test]
fn removed_entities_drop_out_of_riding_on_both_sides() {
    let mut s = store();
    s.spawn(&spawn(1, DVec3::ZERO));
    s.spawn(&spawn(2, DVec3::ZERO));
    s.spawn(&spawn(3, DVec3::ZERO));
    s.set_passengers(EntityId(1), &[EntityId(2)]);
    s.set_passengers(EntityId(3), &[EntityId(1)]);
    // The vehicle is removed: its passengers are freed, and it stops riding entity 3.
    s.remove(&[EntityId(1)]);
    assert_eq!(s.vehicle_of(EntityId(2)), None);
    assert_eq!(s.vehicle_of(EntityId(1)), None);
    assert!(s.passengers_of(EntityId(3)).is_none());
    // A passenger removed from the store also vanishes from its vehicle's list.
    s.set_passengers(EntityId(3), &[EntityId(2)]);
    s.remove(&[EntityId(2)]);
    assert!(s.passengers_of(EntityId(3)).is_none());
}

/// Vanilla keeps attachment points as `float` and widens them to `double` when they become a
/// position (`Entity.positionRider`), so the exact value is the `float` rounding of the decimal
/// one: compare against that, within a float rounding.
fn assert_close(got: f64, want: f64) {
    assert!(
        (got - f64::from(want as f32)).abs() < 1e-6,
        "{got} is not {want}"
    );
}

/// The local player's seat on `vehicle_type`: the numbers vanilla produces for a player, whose
/// own vehicle attachment point is 0.6 below the seat (`Avatar.DEFAULT_VEHICLE_ATTACHMENT`).
fn seat(vehicle_type: &str, seat: usize, passengers: usize) -> SeatPoint {
    passenger_seat(SeatQuery {
        vehicle_type,
        vehicle_dims: (0.6, 1.8),
        seat,
        passengers,
        passenger: PassengerKind::Player,
        passenger_type: "player",
    })
}

/// The same, for a passenger that is not the local player (its own attachment point differs).
fn seat_of(
    vehicle_type: &str,
    seat: usize,
    passengers: usize,
    passenger: PassengerKind,
    passenger_type: &str,
) -> SeatPoint {
    passenger_seat(SeatQuery {
        vehicle_type,
        vehicle_dims: (0.6, 1.8),
        seat,
        passengers,
        passenger,
        passenger_type,
    })
}

#[test]
fn a_minecart_seat_is_vanillas_passenger_attachment() {
    // `passengerAttachments(0.1875F)`, and the player's own 0.6 below it.
    let s = seat("minecart", 0, 1);
    assert_eq!(s.x, 0.0);
    assert_eq!(s.z, 0.0);
    assert_close(s.y, 0.1875 - 0.6);
    // Every minecart variant shares the seat.
    for name in [
        "chest_minecart",
        "furnace_minecart",
        "tnt_minecart",
        "hopper_minecart",
        "spawner_minecart",
        "command_block_minecart",
    ] {
        assert_eq!(seat(name, 0, 1), s, "{name}");
    }
}

#[test]
fn boat_seats_follow_abstract_boat() {
    // `Boat.rideHeight`: height / 3 = 0.1875, minus the player's 0.6.
    let s = seat("oak_boat", 0, 1);
    assert_close(s.y, 0.5625 / 3.0 - 0.6);
    assert_eq!(s.x, 0.0);
    // Two passengers: the driver sits 0.2 to one side, the second 0.6 to the other.
    let driver = seat("oak_boat", 0, 2);
    let behind = seat("oak_boat", 1, 2);
    assert_eq!(driver.x, 0.2);
    assert_eq!(behind.x, -0.6);
    assert_eq!(driver.y, s.y);
    // A chest boat's lone passenger sits 0.15 to the side
    // (`AbstractChestBoat.getSinglePassengerXOffset`).
    assert_eq!(seat("oak_chest_boat", 0, 1).x, 0.15);
    // `Raft.rideHeight`: height * 0.8888889 = 0.5, and the bamboo raft is the only raft.
    assert_close(seat("bamboo_raft", 0, 1).y, 0.5625 * 0.8888889 - 0.6);
    assert_eq!(seat("bamboo_chest_raft", 0, 1).x, 0.15);
    // Every wood shares the boat seat.
    for name in [
        "spruce_boat",
        "birch_boat",
        "jungle_boat",
        "acacia_boat",
        "dark_oak_boat",
        "mangrove_boat",
        "cherry_boat",
        "pale_oak_boat",
        "poplar_boat",
    ] {
        assert_eq!(seat(name, 0, 1), s, "{name}");
    }
    // An animal in the second seat is pushed a further 0.2 out.
    let animal = seat_of("oak_boat", 1, 2, PassengerKind::Animal, "cow");
    assert_eq!(animal.x, -0.6 + 0.2);
}

#[test]
fn horse_family_seats_are_the_saddle() {
    // `passengerAttachments(1.44375F)` on `sized(1.3964844F, 1.6F)`.
    assert_close(seat("horse", 0, 1).y, 1.44375 - 0.6);
    // `passengerAttachments(1.1125F)`, `sized(1.3964844F, 1.5F)`.
    assert_close(seat("donkey", 0, 1).y, 1.1125 - 0.6);
    // Mule: 1.2125 on a 1.6-tall box; both undead horses: 1.31875.
    assert_close(seat("mule", 0, 1).y, 1.2125 - 0.6);
    for name in ["skeleton_horse", "zombie_horse"] {
        assert_close(seat(name, 0, 1).y, 1.31875 - 0.6);
    }
}

#[test]
fn a_pig_seat_is_vanillas() {
    // `passengerAttachments(0.86875F)`, `sized(0.9F, 0.9F)`.
    let s = seat("pig", 0, 1);
    assert_close(s.y, 0.86875 - 0.6);
    assert_eq!((s.x, s.z), (0.0, 0.0));
}

#[test]
fn camel_seats_come_from_camel() {
    // Standing adult: the body anchor is `height - 0.375` = 2.0, minus the player's 0.6.
    let driver = seat("camel", 0, 1);
    assert_close(driver.y, 2.0 - 0.6);
    // The driver sits 0.5 forward; a second passenger 0.7 back.
    assert_eq!(driver.x, 0.5);
    assert_eq!(seat("camel", 1, 2).x, -0.7);
    // `CamelHusk extends Camel`: the same seat.
    assert_eq!(seat("camel_husk", 0, 1), driver);
    // An animal in the second seat is pushed 0.2 forward of it.
    assert_eq!(
        seat_of("camel", 1, 2, PassengerKind::Animal, "cow").x,
        -0.7 + 0.2
    );
}

#[test]
fn a_strider_seat_is_its_own_height() {
    // The strider attaches no points, and its client-side walk animation is idle when it
    // stands: vanilla's `AT_HEIGHT` fallback on `sized(0.9F, 1.7F)`.
    assert_close(seat("strider", 0, 1).y, 1.7 - 0.6);
}

#[test]
fn unknown_vehicles_fall_back_to_their_own_box() {
    // A type the table does not carry (a mod's vehicle): vanilla's fallback on the box the
    // caller knows it by, minus the passenger's own attachment.
    let s = passenger_seat(SeatQuery {
        vehicle_type: "somemod:walker",
        vehicle_dims: (1.0, 2.5),
        seat: 0,
        passengers: 1,
        passenger: PassengerKind::Player,
        passenger_type: "player",
    });
    assert_close(s.y, 2.5 - 0.6);
    // A zombie in a minecart rides at the cart's own position (`AbstractMinecart`).
    let s = seat_of("minecart", 0, 1, PassengerKind::Villager, "villager");
    assert_eq!(s, SeatPoint::ZERO);
    // A zombie riding a boat uses its own `ridingOffset(-0.7F)` point, which lifts it 0.7 above
    // the seat: 0.1875 - 0.7.
    let s = seat_of("oak_boat", 0, 1, PassengerKind::Player, "zombie");
    assert_close(s.y, 0.5625 / 3.0 - 0.7);
}

#[test]
fn the_vehicle_yaw_turns_the_seat() {
    // A boat's second seat sits 0.6 to one side of the boat. `EntityAttachments.transformPoint`
    // (`Vec3.yRot`) turns that with the vehicle's yaw: at yaw 0 it is a -x offset, at yaw 90 a
    // -z one.
    let s = seat("oak_boat", 1, 2);
    let d0 = s.rotated(0.0);
    assert!((d0.x + 0.6).abs() < 1e-6 && d0.z.abs() < 1e-6, "{d0:?}");
    let d90 = s.rotated(90.0);
    assert!(d90.x.abs() < 1e-6 && (d90.z + 0.6).abs() < 1e-6, "{d90:?}");
}
