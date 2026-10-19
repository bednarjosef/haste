// The entity packets: spawns and removals, passenger and vehicle corrections, movement and
// rotation, metadata, equipment and attributes, and the player-info updates.

use glam::DVec3;
use haste_entity::riding::SeatPoint;
use haste_protocol::haste_core::entity::MetadataEntry;
use haste_protocol::haste_core::EntityId;
use haste_protocol::packets::play::clientbound::teleport_flags;
use haste_protocol::{ClientboundPlay, ServerboundPlay, SyncedRegistries};

use super::{EntitySink, Session, WorldSink, HEAD_SLOT, VEHICLE_CORRECTION_EPSILON};

impl Session {
    /// The entity and passenger packets; returns the packet when it is none of them.
    pub(super) fn apply_entities<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        p: ClientboundPlay,
        own: EntityId,
        world: &mut W,
        entities: &mut E,
        regs: &SyncedRegistries,
        out: &mut Vec<ServerboundPlay>,
    ) -> Option<ClientboundPlay> {
        use ClientboundPlay as P;
        match p {
            P::AddEntity(a) => {
                let s = a.spawn();
                self.entity_ids.insert(s.id);
                if self.is_firework_type(s.kind) {
                    self.fireworks.insert(s.id);
                }
                if self.is_lightning_type(s.kind) {
                    self.lightmap.lightning();
                }
                entities.spawn(&s)
            }
            P::RemoveEntities(r) => {
                let ids: Vec<EntityId> = r.entity_ids.iter().map(|i| EntityId(*i)).collect();
                // If our vehicle is removed, we dismount (the store's riding bookkeeping drops
                // us too, as a passenger of the removed vehicle).
                if self.player.vehicle.is_some_and(|v| ids.contains(&v)) {
                    self.player
                        .set_vehicle(None, SeatPoint::ZERO, None, None, None);
                }
                for id in &ids {
                    self.entity_ids.remove(id);
                    self.fireworks.remove(id);
                    self.entity_attributes.remove(id);
                    self.remote_use.remove(id);
                    if self.attached_firework == Some(*id) {
                        self.attached_firework = None;
                    }
                }
                entities.remove(&ids);
            }
            P::SetPassengers(s) => {
                let vehicle = EntityId(s.vehicle);
                let passengers: Vec<EntityId> = s.passengers.iter().map(|i| EntityId(*i)).collect();
                entities.set_passengers(vehicle, &passengers);
                // A boat, horse or pig we steer is simulated on our side from the mount on (see
                // [`crate::boat`], [`crate::horse`], [`crate::pig`]); the correction below
                // clears it on any dismount.
                let first = passengers.first() == Some(&self.entity_id);
                let boat = first
                    .then(|| self.boat_at(vehicle, entities, world))
                    .flatten();
                let horse = first.then(|| self.horse_at(vehicle, entities)).flatten();
                let pig = first.then(|| self.pig_at(vehicle, entities)).flatten();
                // Mount, re-seat, or dismount. The seat comes from the vehicle's type through
                // vanilla's attachment rules (`haste_entity::passenger_seat`), which also names
                // our own seat index: a boat's second passenger sits elsewhere than the first.
                match self.seat_of(vehicle, &passengers, entities) {
                    Some(seat) => {
                        // The mount message only on the foot-to-riding transition (vanilla's
                        // `!wasPlayerMounted`): not on a re-seat, which the same packet does.
                        let was_riding = self.player.vehicle().is_some();
                        self.player
                            .set_vehicle(Some(vehicle), seat, boat, horse, pig);
                        if !was_riding {
                            self.mounted_message = true;
                        }
                    }
                    None if self.player.vehicle() == Some(vehicle) => {
                        // Dismount: our vehicle's new list no longer names us. Sneak's shift
                        // flag in player_input is what made the server send this.
                        self.player
                            .set_vehicle(None, SeatPoint::ZERO, None, None, None);
                    }
                    None => {}
                }
            }
            // The server's correction of the vehicle we steer (vanilla
            // `ClientPacketListener.handleMoveVehicle`): our simulation is snapped to where the
            // server put the vehicle, and the answer carries it straight back as our next
            // `move_vehicle`.
            P::MoveVehicle(m) => {
                if let Some(boat) = self.player.boat_mut() {
                    if (m.pos - boat.pos).length() > VEHICLE_CORRECTION_EPSILON {
                        boat.correct(m.pos, m.yaw);
                    }
                    out.push(ServerboundPlay::MoveVehicle(
                        haste_protocol::packets::play::serverbound::MoveVehicle {
                            pos: boat.pos,
                            yaw: boat.yaw,
                            pitch: 0.0,
                            on_ground: boat.on_ground,
                        },
                    ));
                }
                if let Some(horse) = self.player.horse_mut() {
                    if (m.pos - horse.pos).length() > VEHICLE_CORRECTION_EPSILON {
                        horse.correct(m.pos, m.yaw);
                    }
                    out.push(ServerboundPlay::MoveVehicle(
                        haste_protocol::packets::play::serverbound::MoveVehicle {
                            pos: horse.pos,
                            yaw: horse.yaw,
                            pitch: horse.pitch,
                            on_ground: horse.on_ground,
                        },
                    ));
                }
                // A pig or strider only while we hold its stick (otherwise it is not ours and
                // vanilla does not answer).
                if let Some(animal) = self.player.steered_by_item_mut() {
                    if (m.pos - animal.pose().0).length() > VEHICLE_CORRECTION_EPSILON {
                        animal.correct(m.pos, m.yaw);
                    }
                    let (pos, yaw, pitch, on_ground) = animal.pose();
                    out.push(ServerboundPlay::MoveVehicle(
                        haste_protocol::packets::play::serverbound::MoveVehicle {
                            pos,
                            yaw,
                            pitch,
                            on_ground,
                        },
                    ));
                }
            }
            P::MoveEntityPos(m) => {
                entities.move_delta(EntityId(m.entity_id), m.deltas.total(), m.on_ground)
            }
            P::MoveEntityPosRot(m) => {
                let id = EntityId(m.entity_id);
                entities.move_delta(id, m.deltas.total(), m.on_ground);
                entities.set_rotation(id, m.yaw, m.pitch, m.on_ground);
            }
            P::MoveEntityRot(m) => {
                entities.set_rotation(EntityId(m.entity_id), m.yaw, m.pitch, m.on_ground)
            }
            P::EntityPositionSync(s) => {
                if let Some(pos) = s.end_pos() {
                    entities.teleport(EntityId(s.entity_id), pos, s.yaw, s.pitch, s.on_ground);
                }
            }
            P::TeleportEntity(t) => {
                let id = EntityId(t.entity_id);
                let (old, oy, op) = entities.pose(id).unwrap_or((DVec3::ZERO, 0.0, 0.0));
                let rel = |bit: i32| t.flags & bit != 0;
                let pick = |bit: i32, old: f64, new: f64| if rel(bit) { old + new } else { new };
                let pos = DVec3::new(
                    pick(teleport_flags::X, old.x, t.pos.x),
                    pick(teleport_flags::Y, old.y, t.pos.y),
                    pick(teleport_flags::Z, old.z, t.pos.z),
                );
                let yaw = if rel(teleport_flags::YAW) {
                    oy + t.yaw
                } else {
                    t.yaw
                };
                let pitch = if rel(teleport_flags::PITCH) {
                    op + t.pitch
                } else {
                    t.pitch
                };
                entities.teleport(id, pos, yaw, pitch, t.on_ground);
            }
            P::RotateHead(h) => entities.set_head_yaw(EntityId(h.entity_id), h.head_yaw),
            P::SwingAnimation(s) => {
                entities.swing(EntityId(s.entity_id), s.hand, s.animation, s.duration)
            }
            // The item pickup animation (vanilla `handleTakeItemEntity`): the sound, the
            // particle and the stack shrink / entity removal all live in the pickup module.
            P::TakeItemEntity(t) => self.take_item_entity(t, entities),
            P::SetEntityMotion(m) => entities.set_velocity(EntityId(m.entity_id), m.velocity),
            P::HurtAnimation(h) => {
                entities.mark_hurt(EntityId(h.entity_id));
                if EntityId(h.entity_id) == own {
                    self.start_hurt_tilt(h.yaw);
                }
            }
            P::DamageEvent(d) => super::damage::apply_damage_event(self, &d, entities, regs),
            P::EntityEvent(e) if e.status == 3 => entities.mark_dead(EntityId(e.entity_id)),
            // Entity status 4 is the melee attack animation (IronGolem.handleEntityEvent).
            P::EntityEvent(e) if e.status == 4 => entities.mark_attack(EntityId(e.entity_id)),
            // Entity status 0 on an arrow is the potion puff of a stuck tipped arrow
            // reverting (`Arrow.handleEntityEvent`); other entities' status 0 is not ours.
            P::EntityEvent(e) if e.status == 0 => {
                entities.arrow_potion_puff(EntityId(e.entity_id), &mut self.arrow_particles)
            }
            P::SetEntityData(d) => {
                // Our own entity's data is handled by the player (`LocalPlayer::handle`: the air
                // supply and the screen effects' on-fire and freeze ticks), and `_ if
                // player_packet` above already swallowed it; only others go to the store.
                if EntityId(d.entity_id) != own {
                    let id = EntityId(d.entity_id);
                    entities.set_metadata(id, &d.metadata);
                    // A firework rocket's attached-to target (index 9): remember it while it is
                    // attached to us, so `end_tick` can apply its push.
                    if self.fireworks.contains(&id) {
                        if let Some(attached) = firework_attached_target(&d.metadata) {
                            if attached == Some(own) {
                                self.attached_firework = Some(id);
                            } else if self.attached_firework == Some(id) {
                                self.attached_firework = None;
                            }
                        }
                    }
                    // The pig or strider we ride: the boost's total (a use click on its stick
                    // made the server write it, vanilla `onSyncedDataUpdated` ->
                    // `ItemBasedSteering.onSynced`) and the strider's suffocating flag. The
                    // horse we steer: the server's rear-up flag (the same flag our own
                    // simulation writes locally).
                    if self.player.vehicle() == Some(id) {
                        if let Some(animal) = self.player.item_steered_mut() {
                            animal.on_metadata(&d.metadata);
                        }
                        if let Some(horse) = self.player.horse_mut() {
                            horse.on_metadata(&d.metadata);
                        }
                    }
                }
            }
            P::SetEquipment(e) => {
                for (slot, item) in e.equipment {
                    if EntityId(e.entity_id) == own && slot == HEAD_SLOT {
                        // Our own head slot, if the server sends it this way (vanilla sends our
                        // own armor through `container_set_slot`): the worn item's camera
                        // overlay reads it.
                        self.player.int.armor[0] = item.clone();
                    }
                    entities.set_equipment(EntityId(e.entity_id), slot, item);
                }
            }
            // Another entity's attributes (ours are the player's own, handled above): kept for
            // the mounts we steer, whose simulation reads them.
            P::UpdateAttributes(u) if EntityId(u.entity_id) != own => {
                let id = EntityId(u.entity_id);
                let entry = self.entity_attributes.entry(id).or_default();
                for prop in &u.properties {
                    match entry.iter_mut().find(|a| a.id == prop.id) {
                        Some(a) => *a = prop.clone(),
                        None => entry.push(prop.clone()),
                    }
                }
            }
            P::PlayerInfoUpdate(u) => {
                entities.player_info(&u.infos());
                crate::game::skins::added_profiles(&u, &mut self.profiles);
            }
            _ => return Some(p),
        }
        None
    }
}

/// Entity metadata serializer type id of `OPTIONAL_UNSIGNED_INT` (the wire type of
/// `FireworkRocketEntity.DATA_ATTACHED_TO_TARGET`; `EntityDataSerializers`' registration order).
const OPTIONAL_UNSIGNED_INT: i32 = 19;

/// The firework rocket's attached-to entity id from a metadata update: index 9
/// (`DATA_ATTACHED_TO_TARGET`), an `OPTIONAL_UNSIGNED_INT` (VarInt, 0 = empty, else id + 1).
/// `None` when this update does not carry the entry (metadata updates are partial).
fn firework_attached_target(metadata: &[MetadataEntry]) -> Option<Option<EntityId>> {
    let e = metadata.iter().find(|e| e.index == 9)?;
    let haste_protocol::haste_core::entity::MetadataValue::Other { type_id, raw } = &e.value else {
        return None;
    };
    if *type_id != OPTIONAL_UNSIGNED_INT {
        return None;
    }
    let v = read_var_int(raw)?;
    Some(if v == 0 { None } else { Some(EntityId(v - 1)) })
}

/// A VarInt as the wire writes it (LEB128, at most 5 bytes for an i32).
fn read_var_int(mut b: &[u8]) -> Option<i32> {
    let mut value = 0u32;
    for i in 0..5 {
        let byte = *b.first()?;
        b = &b[1..];
        value |= u32::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Some(value as i32);
        }
    }
    None
}
