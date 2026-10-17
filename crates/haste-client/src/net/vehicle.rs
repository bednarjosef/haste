//! The vehicles the local player rides, as the server seats us in them: our seat on the
//! vehicle and the client-authoritative mounts we steer (the boat, the horse family, the pig
//! and the strider), built from the entity store and the last synced attributes.

use haste_entity::riding::{passenger_seat, PassengerKind, SeatPoint, SeatQuery};
use haste_protocol::haste_core::entity::MetadataValue;
use haste_protocol::haste_core::EntityId;
use haste_protocol::packets::play::clientbound as cb;

use super::{EntitySink, Session, WorldSink, NO_BLOCKS, SADDLE_SLOT};

/// Vanilla `Mob.isSaddled` for a mount: the saddle slot (wire id 7) worn, and the slot usable
/// (`canUseSlot(SADDLE)`: the pig and the strider when grown, the horse family also only when
/// tamed, `AbstractHorse.DATA_ID_FLAGS` bit 2 at synched index 18). An untamed horse is never
/// steered: the server's own AI bucks the rider (`RunAroundLikeCrazyGoal`) and we follow it.
fn is_saddled<E: EntitySink + ?Sized>(vehicle: EntityId, entities: &E, needs_tame: bool) -> bool {
    let worn = entities.equipment(vehicle).is_some_and(|eq| {
        eq.iter()
            .any(|(slot, stack)| *slot == SADDLE_SLOT && !stack.is_empty())
    });
    let baby = matches!(
        entities.metadata_value(vehicle, haste_entity::registry::BABY_METADATA_INDEX),
        Some(MetadataValue::Boolean(true))
    );
    let tamed = matches!(
        entities.metadata_value(vehicle, crate::horse::DATA_ID_FLAGS),
        Some(MetadataValue::Byte(b)) if b & crate::horse::FLAG_TAME != 0
    );
    worn && !baby && (tamed || !needs_tame)
}

impl Session {
    /// Our seat on `vehicle` if this passenger list names us, else `None` (we are not riding
    /// it): vanilla's `Entity.positionRider` for the local player. The seat order comes from
    /// the list, and the vehicle's attachment points from its type in the entity_type registry.
    pub(super) fn seat_of<E: EntitySink + ?Sized>(
        &self,
        vehicle: EntityId,
        passengers: &[EntityId],
        entities: &E,
    ) -> Option<SeatPoint> {
        let seat = passengers.iter().position(|p| *p == self.entity_id)?;
        let (kind, dims) = entities.entity_type_info(vehicle)?;
        let vehicle_type = self
            .entity_type_names
            .get(kind.0 as usize)
            .map_or("?", |id| id.path());
        Some(passenger_seat(SeatQuery {
            vehicle_type,
            vehicle_dims: dims,
            seat,
            passengers: passengers.len(),
            passenger: PassengerKind::Player,
            // The passenger is us: `Avatar.DEFAULT_VEHICLE_ATTACHMENT` (0.6 below the seat),
            // which is `minecraft:player`'s own vehicle attachment point.
            passenger_type: "player",
        }))
    }

    /// The boat we steer, when the vehicle the server just seated us in is one we drive (the
    /// first passenger of a `AbstractBoat`): vanilla's `Entity.isLocalClientAuthoritative`. Our
    /// own simulation of the boat starts where the server last told us it was, with the
    /// dimensions its registry entry carries; after that the state we simulate is the truth on
    /// this side and the server follows our `move_vehicle` packets.
    pub(super) fn boat_at<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &self,
        vehicle: EntityId,
        entities: &E,
        world: &W,
    ) -> Option<crate::boat::Boat> {
        let kind = entities.entity_type(vehicle)?;
        let path = self
            .entity_type_names
            .get(kind.0 as usize)
            .map(|id| id.path())?;
        if !crate::boat::is_boat_type(path) {
            return None;
        }
        let (pos, yaw, _) = entities.pose(vehicle)?;
        let (_, dims) = entities.entity_type_info(vehicle)?;
        let env = match world.physics() {
            Some((view, data)) => crate::boat::BoatEnv {
                world: view,
                shapes: &data.shapes,
                blocks: data,
            },
            // A world without physics data (test fakes): a boat in a void has no water and no
            // ground, so it starts in the air.
            None => crate::boat::BoatEnv {
                world: &NO_BLOCKS,
                shapes: &NO_BLOCKS,
                blocks: &haste_physics::DefaultBlockPhysics,
            },
        };
        Some(crate::boat::Boat::new(pos, yaw, dims, &env))
    }

    /// The horse-family mount we steer, when the vehicle the server just seated us in is one we
    /// control (vanilla `AbstractHorse.getControllingPassenger`: the first passenger, a player,
    /// on a saddled horse): our own simulation starts where the server last told us it was,
    /// with the attributes its `update_attributes` carried (the family's base values before
    /// any sync lands) and the dimensions its registry entry carries.
    pub(super) fn horse_at<E: EntitySink + ?Sized>(
        &self,
        vehicle: EntityId,
        entities: &E,
    ) -> Option<crate::horse::Horse> {
        let kind = entities.entity_type(vehicle)?;
        let path = self
            .entity_type_names
            .get(kind.0 as usize)
            .map(|id| id.path())?;
        if !crate::horse::is_horse_type(path) {
            return None;
        }
        // Vanilla `Mob.isSaddled` with `AbstractHorse.canUseSlot(SADDLE)`: tamed and grown.
        if !is_saddled(vehicle, entities, true) {
            return None;
        }
        let (pos, yaw, _) = entities.pose(vehicle)?;
        let (_, dims) = entities.entity_type_info(vehicle)?;
        let (speed, jump, gravity) =
            self.vehicle_attributes(vehicle, crate::horse::DEFAULT_MOVEMENT_SPEED);
        Some(crate::horse::Horse::new(
            pos, yaw, dims, speed, jump, gravity,
        ))
    }

    /// The pig we steer with a carrot on a stick, when the vehicle the server just seated us
    /// in is one we control (vanilla `Pig.getControllingPassenger`: the first passenger, a
    /// player, of a saddled pig — the carrot on a stick is checked per tick while riding, so
    /// it is not part of this). Like the horse, our own simulation starts where the server
    /// last told us it was.
    pub(super) fn pig_at<E: EntitySink + ?Sized>(
        &self,
        vehicle: EntityId,
        entities: &E,
    ) -> Option<crate::item_based_steering::ItemSteered> {
        let kind = entities.entity_type(vehicle)?;
        let path = self
            .entity_type_names
            .get(kind.0 as usize)
            .map(|id| id.path())?;
        let default_speed = if crate::pig::is_pig_type(path) {
            crate::pig::DEFAULT_MOVEMENT_SPEED
        } else if crate::strider::is_strider_type(path) {
            crate::strider::DEFAULT_MOVEMENT_SPEED
        } else {
            return None;
        };
        // Vanilla `Mob.isSaddled` with `Pig`/`Strider.canUseSlot(SADDLE)`: grown.
        if !is_saddled(vehicle, entities, false) {
            return None;
        }
        let (pos, yaw, _) = entities.pose(vehicle)?;
        let (_, dims) = entities.entity_type_info(vehicle)?;
        let (speed, _, gravity) = self.vehicle_attributes(vehicle, default_speed);
        Some(if crate::pig::is_pig_type(path) {
            crate::item_based_steering::ItemSteered::Pig(crate::pig::Pig::new(
                pos, yaw, dims, speed, gravity,
            ))
        } else {
            let mut s = crate::strider::Strider::new(pos, yaw, dims, speed, gravity);
            s.suffocating = matches!(
                entities.metadata_value(vehicle, crate::strider::DATA_SUFFOCATING),
                Some(MetadataValue::Boolean(true))
            );
            crate::item_based_steering::ItemSteered::Strider(s)
        })
    }

    /// The steered mount's attributes from the last `update_attributes` of `vehicle`
    /// (`movement_speed`, `jump_strength`, `gravity`; `default_speed` is the animal family's
    /// own base value, the fallback vanilla's own client would read before a sync).
    fn vehicle_attributes(&self, vehicle: EntityId, default_speed: f64) -> (f64, f64, f64) {
        let attrs = self.entity_attributes.get(&vehicle);
        let value = |name: &str, default: f64| {
            attrs
                .and_then(|a| {
                    a.iter()
                        .find(|p| self.attribute_is(haste_core::RegistryIndex(p.id as u32), name))
                })
                .map(|p| self.attribute_value(p))
                .unwrap_or(default)
        };
        (
            value("movement_speed", default_speed),
            value("jump_strength", crate::horse::DEFAULT_JUMP_STRENGTH),
            value("gravity", crate::horse::DEFAULT_GRAVITY),
        )
    }

    /// Whether an attribute property's registry index is `minecraft:<name>` (the synced
    /// `minecraft:attribute` registry's order, when the server sent one, else the data
    /// generator's; both agree on vanilla servers).
    fn attribute_is(&self, id: haste_core::RegistryIndex, name: &str) -> bool {
        self.attribute_names
            .as_ref()
            .and_then(|names| names.get(id.0 as usize))
            .is_some_and(|n| n.path() == name)
    }

    /// An attribute's value with its modifiers (the same fold the player's attributes use).
    fn attribute_value(&self, p: &cb::AttributeProperty) -> f64 {
        crate::player::attribute_value(p, |_| false)
    }
}
