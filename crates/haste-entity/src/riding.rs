//! Riding bookkeeping (entities): which entity is a passenger of which vehicle, from
//! `set_passengers`, and where a passenger's seat is. One [`Riding`] lives inside the
//! [`EntityStore`](crate::store::EntityStore); a passenger list replaces the previous one, and
//! entities removed from the store drop out of both sides.
//!
//! The seat follows vanilla's `EntityAttachments`: the vehicle's `PASSENGER` points (attached by
//! its `EntityType.Builder`, `Fallback.AT_HEIGHT` when none are) minus the passenger's own
//! `VEHICLE` point, turned by the vehicle's yaw (`Entity.positionRider`).

use ahash::AHashMap;
use haste_core::ids::EntityId;
use haste_core::DVec3;

/// What the passenger is, for the seat rules that look at the passenger's own type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassengerKind {
    /// A player (`Avatar`), or any entity that is neither an `Animal` nor a `Villager`.
    Player,
    /// `passenger instanceof Animal`: boats and camels push the seat 0.2 blocks out sideways
    /// (`AbstractBoat.getPassengerAttachmentPoint`, `Camel.getPassengerAttachmentPoint`).
    Animal,
    /// `passenger instanceof Villager || passenger instanceof WanderingTrader`: minecarts drop
    /// the seat to the cart's own position (`AbstractMinecart.LOWERED_PASSENGER_ATTACHMENT`).
    Villager,
}

/// Where a passenger's feet sit, in blocks, relative to its vehicle: vanilla's
/// `Entity.positionRider`, which is the vehicle's passenger attachment point minus the
/// passenger's own vehicle attachment point. `x`/`z` are in the vehicle's frame - they turn
/// with it ([`SeatPoint::rotated`]) - and `y` is straight up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeatPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl SeatPoint {
    /// The vehicle's own position: what `AbstractMinecart` uses for villagers, and what a
    /// passenger with no attachment sits at.
    pub const ZERO: SeatPoint = SeatPoint {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// `EntityAttachments.transformPoint` (`Vec3.yRot`): the vehicle's yaw turns the seat's
    /// sideways and forward offsets. Returns the offset to add to the vehicle's position.
    ///
    /// The passenger's own vehicle attachment point is not turned with it: every vanilla
    /// `VEHICLE` point has `x = z = 0` (`ridingOffset` and `Avatar.DEFAULT_VEHICLE_ATTACHMENT`),
    /// so the passenger's yaw, which vanilla rotates it by, changes nothing.
    pub fn rotated(self, vehicle_yaw: f32) -> DVec3 {
        let radians = -f64::from(vehicle_yaw) * (std::f64::consts::PI / 180.0);
        // Vanilla takes cos/sin of a `float` angle, then widens to `double`.
        let cos = f64::from(f32::cos(radians as f32));
        let sin = f64::from(f32::sin(radians as f32));
        DVec3::new(
            self.x * cos + self.z * sin,
            self.y,
            self.z * cos - self.x * sin,
        )
    }
}

/// Where a passenger sits on a vehicle: everything vanilla's `Entity.positionRider` needs.
pub struct SeatQuery<'a> {
    /// The vehicle's registry path (`minecraft:oak_boat` -> `oak_boat`).
    pub vehicle_type: &'a str,
    /// The vehicle's current (width, height), for types this module does not carry (mods).
    pub vehicle_dims: (f32, f32),
    /// The passenger's index in the vehicle's passenger list (`getPassengers().indexOf`).
    pub seat: usize,
    /// How many passengers the vehicle carries.
    pub passengers: usize,
    /// What the passenger is, for the rules that look at the passenger's own type.
    pub passenger: PassengerKind,
    /// The passenger's registry path, for `Entity.getVehicleAttachmentPoint`.
    pub passenger_type: &'a str,
}

/// Where the passenger's feet sit, in the vehicle's frame, with the passenger's own vehicle
/// attachment already subtracted: vanilla's `Entity.positionRider` applied to a local player.
/// `vehicle_yaw` is not part of this - it turns the result each tick
/// ([`SeatPoint::rotated`]), because the vehicle turns under the passenger.
pub fn passenger_seat(q: SeatQuery<'_>) -> SeatPoint {
    let vehicle = attachment_point(&q);
    let own = rider_attachment(q.passenger_type);
    SeatPoint {
        x: vehicle.x - own.x,
        y: vehicle.y - own.y,
        z: vehicle.z - own.z,
    }
}

/// One point of a vanilla `EntityAttachments` (a `Vec3`), in the float precision vanilla stores
/// it in: `y` up, `z` behind the vehicle (negative is behind), `x` to its left.
#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
    z: f32,
}

const fn point(x: f32, y: f32, z: f32) -> Point {
    Point { x, y, z }
}

/// The place a vehicle class overrides the points its `EntityType` attached, in
/// `Entity.getPassengerAttachmentPoint`.
enum SeatRule {
    /// The type's own `passengerAttachments` points, or vanilla's fallback `(0, height, 0)`
    /// when it attached none (`EntityAttachment.PASSENGER`, `Fallback.AT_HEIGHT`).
    Points,
    /// `AbstractBoat.getPassengerAttachmentPoint`: `raft` is `Raft.rideHeight`
    /// (`height * 0.8888889`) where `Boat.rideHeight` is `height / 3`, and `chest` is
    /// `AbstractChestBoat.getSinglePassengerXOffset` (0.15, where a plain boat has 0.0).
    Boat { raft: bool, chest: bool },
    /// `Camel.getPassengerAttachmentPoint`, standing: the body anchor (`height - 0.375`) and
    /// the camel's own seat offsets.
    ///
    /// Not yet ported, because the store tracks no animation or age state: sitting and the sit
    /// transition (`Camel.getBodyAnchorAnimationYOffset`), baby camels (`getAgeScale`), and the
    /// horse family's `standAnimO` term in `AbstractHorse.getPassengerAttachmentPoint`. A
    /// standing adult uses the numbers below.
    Camel,
}

/// A vehicle type's seat data: the `passengerAttachments` points and the `sized` box the
/// fallback uses, both read out of `EntityType.Builder` in `EntityTypes.java`.
struct VehicleSeat {
    /// The type's height from `sized`: the `AT_HEIGHT` fallback, and the ride height for boats.
    height: f32,
    points: &'static [Point],
    rule: SeatRule,
}

impl VehicleSeat {
    const fn points(points: &'static [Point], height: f32) -> VehicleSeat {
        VehicleSeat {
            height,
            points,
            rule: SeatRule::Points,
        }
    }

    const fn boat(height: f32, raft: bool, chest: bool) -> VehicleSeat {
        VehicleSeat {
            height,
            points: &[],
            rule: SeatRule::Boat { raft, chest },
        }
    }

    const fn camel(height: f32) -> VehicleSeat {
        VehicleSeat {
            height,
            points: &[],
            rule: SeatRule::Camel,
        }
    }
}

/// `registry id -> seat`, generated from `EntityTypes.java`: every `passengerAttachments`
/// call with its `sized` height, the boat and raft entries from `boatFactory`, `chestBoatFactory`,
/// `raftFactory` and `chestRaftFactory` (`AbstractBoat`, `Raft`, `AbstractChestBoat`), and the
/// camel entry (`Camel`). Types that attach no points appear here only when a class overrides
/// the seat; the rest use [`SeatRule::Points`] against the caller's dimensions, which is
/// vanilla's fallback for a type we do not know.
///
/// The numbers are vanilla's own, read off its registry definitions (the same values a
/// recorder-mod dump of the live game would produce). When oracles extends the entity dump
/// with attachment points, this table is replaced by that file (AGENTS.md rule 7); the table
/// above keeps the box as a constant rather than a lookup so a vehicle we do carry sits at the
/// vanilla height even before the dimensions oracle maps names to indexes (`dims.rs`).
const VEHICLES: &[(&str, VehicleSeat)] = &[
    // Minecarts: `passengerAttachments(0.1875F)`, `AbstractMinecart`'s lowered seat for
    // villagers, `sized(0.98F, 0.7F)`.
    (
        "minecart",
        VehicleSeat::points(&[point(0.0, 0.1875, 0.0)], 0.7),
    ),
    (
        "chest_minecart",
        VehicleSeat::points(&[point(0.0, 0.1875, 0.0)], 0.7),
    ),
    (
        "furnace_minecart",
        VehicleSeat::points(&[point(0.0, 0.1875, 0.0)], 0.7),
    ),
    (
        "tnt_minecart",
        VehicleSeat::points(&[point(0.0, 0.1875, 0.0)], 0.7),
    ),
    (
        "hopper_minecart",
        VehicleSeat::points(&[point(0.0, 0.1875, 0.0)], 0.7),
    ),
    (
        "spawner_minecart",
        VehicleSeat::points(&[point(0.0, 0.1875, 0.0)], 0.7),
    ),
    (
        "command_block_minecart",
        VehicleSeat::points(&[point(0.0, 0.1875, 0.0)], 0.7),
    ),
    // Boats and the one raft family: no points, `AbstractBoat` computes the seat from
    // `sized(1.375F, 0.5625F)`.
    ("oak_boat", VehicleSeat::boat(0.5625, false, false)),
    ("oak_chest_boat", VehicleSeat::boat(0.5625, false, true)),
    ("spruce_boat", VehicleSeat::boat(0.5625, false, false)),
    ("spruce_chest_boat", VehicleSeat::boat(0.5625, false, true)),
    ("birch_boat", VehicleSeat::boat(0.5625, false, false)),
    ("birch_chest_boat", VehicleSeat::boat(0.5625, false, true)),
    ("jungle_boat", VehicleSeat::boat(0.5625, false, false)),
    ("jungle_chest_boat", VehicleSeat::boat(0.5625, false, true)),
    ("acacia_boat", VehicleSeat::boat(0.5625, false, false)),
    ("acacia_chest_boat", VehicleSeat::boat(0.5625, false, true)),
    ("dark_oak_boat", VehicleSeat::boat(0.5625, false, false)),
    (
        "dark_oak_chest_boat",
        VehicleSeat::boat(0.5625, false, true),
    ),
    ("mangrove_boat", VehicleSeat::boat(0.5625, false, false)),
    (
        "mangrove_chest_boat",
        VehicleSeat::boat(0.5625, false, true),
    ),
    ("cherry_boat", VehicleSeat::boat(0.5625, false, false)),
    ("cherry_chest_boat", VehicleSeat::boat(0.5625, false, true)),
    ("pale_oak_boat", VehicleSeat::boat(0.5625, false, false)),
    (
        "pale_oak_chest_boat",
        VehicleSeat::boat(0.5625, false, true),
    ),
    ("poplar_boat", VehicleSeat::boat(0.5625, false, false)),
    ("poplar_chest_boat", VehicleSeat::boat(0.5625, false, true)),
    ("bamboo_raft", VehicleSeat::boat(0.5625, true, false)),
    ("bamboo_chest_raft", VehicleSeat::boat(0.5625, true, true)),
    // Horses and the camel: `Camel.getPassengerAttachmentPoint` replaces `AbstractHorse`'s.
    (
        "horse",
        VehicleSeat::points(&[point(0.0, 1.44375, 0.0)], 1.6),
    ),
    (
        "donkey",
        VehicleSeat::points(&[point(0.0, 1.1125, 0.0)], 1.5),
    ),
    ("mule", VehicleSeat::points(&[point(0.0, 1.2125, 0.0)], 1.6)),
    (
        "skeleton_horse",
        VehicleSeat::points(&[point(0.0, 1.31875, 0.0)], 1.6),
    ),
    (
        "zombie_horse",
        VehicleSeat::points(&[point(0.0, 1.31875, 0.0)], 1.6),
    ),
    ("camel", VehicleSeat::camel(2.375)),
    ("camel_husk", VehicleSeat::camel(2.375)),
    // The strider attaches nothing: a standing strider's client-side walk animation adds
    // nothing either (`Strider.getPassengerAttachmentPoint` adds 0.12 * walkAnimation speed),
    // so the seat is vanilla's `AT_HEIGHT` fallback on `sized(0.9F, 1.7F)`.
    ("strider", VehicleSeat::points(&[], 1.7)),
    // Llamas: `Llama.getPassengerAttachmentPoint` keeps the type's own point.
    (
        "llama",
        VehicleSeat::points(&[point(0.0, 1.37, -0.3)], 1.87),
    ),
    (
        "trader_llama",
        VehicleSeat::points(&[point(0.0, 1.37, -0.3)], 1.87),
    ),
    // The rest of the `passengerAttachments` builders, in `EntityTypes.java`'s order. A
    // player rides none of these, but any of them can carry a passenger, and a mod's vehicle
    // id is not in here at all - it falls back to its own box.
    ("cat", VehicleSeat::points(&[point(0.0, 0.5125, 0.0)], 0.7)),
    (
        "chicken",
        VehicleSeat::points(&[point(0.0, 0.7, -0.1)], 0.7),
    ),
    ("cow", VehicleSeat::points(&[point(0.0, 1.36875, 0.0)], 1.4)),
    (
        "mooshroom",
        VehicleSeat::points(&[point(0.0, 1.36875, 0.0)], 1.4),
    ),
    (
        "drowned",
        VehicleSeat::points(&[point(0.0, 2.0125, 0.0)], 1.95),
    ),
    (
        "elder_guardian",
        VehicleSeat::points(&[point(0.0, 2.350625, 0.0)], 1.9975),
    ),
    (
        "enderman",
        VehicleSeat::points(&[point(0.0, 2.80625, 0.0)], 2.9),
    ),
    (
        "endermite",
        VehicleSeat::points(&[point(0.0, 0.2375, 0.0)], 0.3),
    ),
    (
        "ender_dragon",
        VehicleSeat::points(&[point(0.0, 3.0, 0.0)], 8.0),
    ),
    ("evoker", VehicleSeat::points(&[point(0.0, 2.0, 0.0)], 1.95)),
    (
        "fox",
        VehicleSeat::points(&[point(0.0, 0.6375, -0.25)], 0.7),
    ),
    (
        "frog",
        VehicleSeat::points(&[point(0.0, 0.375, -0.25)], 0.5),
    ),
    (
        "ghast",
        VehicleSeat::points(&[point(0.0, 4.0625, 0.0)], 4.0),
    ),
    (
        "happy_ghast",
        VehicleSeat::points(
            &[
                point(0.0, 4.0, 1.7),  // front seat
                point(-1.7, 4.0, 0.0), // left seat
                point(0.0, 4.0, -1.7), // back seat
                point(1.7, 4.0, 0.0),  // right seat
            ],
            4.0,
        ),
    ),
    ("goat", VehicleSeat::points(&[point(0.0, 1.1125, 0.0)], 1.3)),
    (
        "guardian",
        VehicleSeat::points(&[point(0.0, 0.975, 0.0)], 0.85),
    ),
    (
        "hoglin",
        VehicleSeat::points(&[point(0.0, 1.49375, 0.0)], 1.4),
    ),
    (
        "zoglin",
        VehicleSeat::points(&[point(0.0, 1.49375, 0.0)], 1.4),
    ),
    ("husk", VehicleSeat::points(&[point(0.0, 2.075, 0.0)], 1.95)),
    (
        "illusioner",
        VehicleSeat::points(&[point(0.0, 2.0, 0.0)], 1.95),
    ),
    (
        "ocelot",
        VehicleSeat::points(&[point(0.0, 0.6375, 0.0)], 0.7),
    ),
    (
        "parrot",
        VehicleSeat::points(&[point(0.0, 0.4625, 0.0)], 0.9),
    ),
    (
        "phantom",
        VehicleSeat::points(&[point(0.0, 0.3375, 0.0)], 0.5),
    ),
    ("pig", VehicleSeat::points(&[point(0.0, 0.86875, 0.0)], 0.9)),
    (
        "piglin",
        VehicleSeat::points(&[point(0.0, 2.0125, 0.0)], 1.95),
    ),
    (
        "piglin_brute",
        VehicleSeat::points(&[point(0.0, 2.0125, 0.0)], 1.95),
    ),
    (
        "pillager",
        VehicleSeat::points(&[point(0.0, 2.0, 0.0)], 1.95),
    ),
    (
        "ravager",
        VehicleSeat::points(&[point(0.0, 2.2625, -0.0625)], 2.2),
    ),
    (
        "sheep",
        VehicleSeat::points(&[point(0.0, 1.2375, 0.0)], 1.3),
    ),
    (
        "silverfish",
        VehicleSeat::points(&[point(0.0, 0.2375, 0.0)], 0.3),
    ),
    (
        "skeleton",
        VehicleSeat::points(&[point(0.0, 2.0125, 0.0)], 1.99),
    ),
    (
        "sniffer",
        VehicleSeat::points(&[point(0.0, 2.09375, 0.0)], 1.75),
    ),
    (
        "spider",
        VehicleSeat::points(&[point(0.0, 0.765, 0.0)], 0.9),
    ),
    (
        "turtle",
        VehicleSeat::points(&[point(0.0, 0.55625, -0.25)], 0.4),
    ),
    ("vex", VehicleSeat::points(&[point(0.0, 0.7375, 0.0)], 0.8)),
    (
        "vindicator",
        VehicleSeat::points(&[point(0.0, 2.0, 0.0)], 1.95),
    ),
    ("warden", VehicleSeat::points(&[point(0.0, 3.15, 0.0)], 2.9)),
    (
        "witch",
        VehicleSeat::points(&[point(0.0, 2.2625, 0.0)], 1.95),
    ),
    (
        "wolf",
        VehicleSeat::points(&[point(0.0, 0.81875, -0.0625)], 0.85),
    ),
    (
        "zombie",
        VehicleSeat::points(&[point(0.0, 2.0125, 0.0)], 1.95),
    ),
    (
        "zombie_villager",
        VehicleSeat::points(&[point(0.0, 2.125, 0.0)], 1.95),
    ),
    (
        "zombified_piglin",
        VehicleSeat::points(&[point(0.0, 2.0, 0.0)], 1.95),
    ),
    (
        "zombie_nautilus",
        VehicleSeat::points(&[point(0.0, 1.1375, 0.0)], 0.95),
    ),
    (
        "nautilus",
        VehicleSeat::points(&[point(0.0, 1.1375, 0.0)], 0.95),
    ),
];

/// `ridingOffset(x)` from `EntityType.Builder`, i.e. the passenger's own `VEHICLE` attachment
/// point `(0, -x, 0)`: the passenger drops x below the seat. A type built with
/// `vehicleAttachment(Avatar.DEFAULT_VEHICLE_ATTACHMENT)` instead gets the player's 0.6; every
/// other type has no `VEHICLE` point at all (`Fallback.AT_FEET`), and sits at the seat.
const RIDING_OFFSETS: &[(&str, f32)] = &[
    ("allay", 0.04),
    ("bogged", -0.7),
    ("drowned", -0.7),
    ("evoker", -0.6),
    ("ghast", 0.5),
    ("giant", -3.75),
    ("happy_ghast", 0.5),
    ("hoglin", -0.7),
    ("husk", -0.7),
    ("illusioner", -0.6),
    ("parched", -0.7),
    ("phantom", -0.125),
    ("piglin", -0.7),
    ("piglin_brute", -0.7),
    ("pillager", -0.6),
    ("skeleton", -0.7),
    ("stray", -0.7),
    ("vex", 0.04),
    ("vindicator", -0.6),
    ("wither_skeleton", -0.875),
    ("zombie", -0.7),
    ("zombie_villager", -0.7),
    ("zombified_piglin", -0.7),
];

/// The passenger's own vehicle attachment point (`Entity.getVehicleAttachmentPoint`): how far
/// below the seat its feet go, in the passenger's own frame.
fn rider_attachment(passenger_type: &str) -> SeatPoint {
    // `Avatar.DEFAULT_VEHICLE_ATTACHMENT`, attached by `Player` and `Mannequin`'s builders.
    let y = if matches!(passenger_type, "player" | "mannequin") {
        0.6
    } else {
        -f64::from(
            RIDING_OFFSETS
                .iter()
                .find(|(id, _)| *id == passenger_type)
                .map_or(0.0, |(_, offset)| *offset),
        )
    };
    // Every vanilla VEHICLE point has x = z = 0, so the passenger's yaw never moves it.
    SeatPoint { x: 0.0, y, z: 0.0 }
}

/// The vehicle's passenger attachment point, in the vehicle's frame.
fn attachment_point(q: &SeatQuery<'_>) -> SeatPoint {
    let Some((_, seat)) = VEHICLES.iter().find(|(id, _)| *id == q.vehicle_type) else {
        // A type we do not carry (a mod's vehicle): vanilla's fallback on the box we know it
        // by, `Fallback.AT_HEIGHT`.
        return SeatPoint {
            x: 0.0,
            y: f64::from(q.vehicle_dims.1),
            z: 0.0,
        };
    };
    let (x, y, z) = match seat.rule {
        SeatRule::Points if seat.points.is_empty() => (0.0, f64::from(seat.height), 0.0),
        SeatRule::Points => {
            // `EntityAttachments.getClamped`: a seat index past the end of the list uses the
            // last point.
            let p = seat.points[q.seat.min(seat.points.len() - 1)];
            (f64::from(p.x), f64::from(p.y), f64::from(p.z))
        }
        SeatRule::Boat { raft, chest } => {
            let ride_height = if raft {
                f64::from(seat.height) * 0.8888889
            } else {
                f64::from(seat.height) / 3.0
            };
            let mut x = if q.passengers > 1 {
                if q.seat == 0 {
                    0.2
                } else {
                    -0.6
                }
            } else if chest {
                0.15
            } else {
                0.0
            };
            if q.passengers > 1 && q.passenger == PassengerKind::Animal {
                x += 0.2;
            }
            (x, ride_height, 0.0)
        }
        SeatRule::Camel => {
            let mut x = 0.5;
            if q.passengers > 1 {
                if q.seat != 0 {
                    x = -0.7;
                }
                if q.passenger == PassengerKind::Animal {
                    x += 0.2;
                }
            }
            (x, f64::from(seat.height) - 0.375, 0.0)
        }
    };
    if q.passenger == PassengerKind::Villager && is_minecart(q.vehicle_type) {
        // `AbstractMinecart.LOWERED_PASSENGER_ATTACHMENT`, instead of the cart's own point.
        return SeatPoint::ZERO;
    }
    SeatPoint { x, y, z }
}

/// The six minecart types, whose class lowers villagers' seats to the cart's own position.
fn is_minecart(vehicle_type: &str) -> bool {
    matches!(
        vehicle_type,
        "minecart"
            | "chest_minecart"
            | "furnace_minecart"
            | "tnt_minecart"
            | "hopper_minecart"
            | "spawner_minecart"
            | "command_block_minecart"
    )
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
