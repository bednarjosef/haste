// The `take_item_entity` packet and its pickup particle (vanilla
// `ClientPacketListener.handleTakeItemEntity` and `ItemPickupParticle`): the sound the
// collection plays, the item (or orb) flying from where it was to the collector over three
// ticks, and the store update (a dropped item's stack shrinks and the entity is removed when
// it empties; orbs stay, anything else is removed).

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::entity::{MetadataEntry, MetadataValue};
use haste_core::{EntityId, RegistryIndex};
use haste_protocol::packets::play::clientbound as cb;

use super::{EntitySink, Session};
use crate::sound::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// `SoundSource.PLAYERS`'s index in vanilla's `SoundSource` order (0 master, 1 music, 2
/// records, 3 weather, 4 blocks, 5 hostile, 6 neutral, 7 players, ...).
const CATEGORY_PLAYERS: i32 = 7;

/// Entity metadata index of `Entity.DATA_SILENT` (a `Boolean`; the fourth shared field after
/// the flags, air supply, custom-name-visible and custom-name entries).
const SILENT_METADATA_INDEX: u8 = 4;

/// Entity metadata index of a dropped item's stack (`ItemEntity.DATA_ITEM`) and of an
/// experience orb's value (`ExperienceOrb.DATA_VALUE`): both are the first field after
/// `Entity`'s eight shared entries.
const ITEM_OR_ORB_METADATA_INDEX: u8 = 8;

/// The entity the collected entity is (or was) flying to.
const LIFE_TIME: u32 = 3;

/// One pickup particle (vanilla `ItemPickupParticle`): the collected entity's frozen render
/// position and age, the collector's target position at the start and end of the last tick,
/// and the particle's age in ticks. The render thread interpolates the eased position from it.
#[derive(Clone, Debug, PartialEq)]
pub struct PickupParticle {
    /// The collected entity's id (also its render phase, so the item spins as it did).
    pub id: EntityId,
    /// What is drawn flying to the collector.
    pub visual: PickupVisual,
    /// The collected entity's age in ticks plus one (vanilla extracts the render state at
    /// partial tick 1.0, freezing the spin and bob).
    pub age: f32,
    /// Where the collected entity was at the moment of collection (frozen).
    pub start: DVec3,
    /// The collector's target position at the end of the previous tick.
    pub target_old: DVec3,
    /// The collector's target position at the end of the last tick.
    pub target: DVec3,
    /// The entity the particle flies to (its position is re-read each tick).
    pub collector: EntityId,
    /// Ticks since the particle was created (removed at [`LIFE_TIME`]).
    pub life: u32,
}

/// What a pickup particle draws: vanilla hands the collected entity's render state to the
/// particle, so an item draws as the item and an orb as the orb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickupVisual {
    /// A dropped item entity's stack (the item itself).
    Item { item: RegistryIndex, count: i32 },
    /// An experience orb (`ExperienceOrb.getIcon`'s value tier).
    Orb { value: i32 },
}

/// The particle's position at `partial` through the tick: vanilla `ItemPickupParticleGroup
/// .ParticleInstance.fromParticle` — `t = ((life + partial) / 3)^2`, the target at
/// `lerp(partial, targetOld, target)`, and the position at `lerp(t, start, target)`.
pub fn pickup_position(p: &PickupParticle, partial: f32) -> DVec3 {
    let t = (p.life as f32 + partial) / LIFE_TIME as f32;
    let t = t * t;
    let target = p.target_old.lerp(p.target, partial as f64);
    p.start.lerp(target, t as f64)
}

impl Session {
    /// `minecraft:take_item_entity` (vanilla `ClientPacketListener.handleTakeItemEntity`):
    /// play the pickup sound unless the entity is silent, add the pickup particle, then shrink
    /// an item entity's stack (removing it when empty) or remove the entity (orbs stay).
    pub(super) fn take_item_entity<E: EntitySink + ?Sized>(
        &mut self,
        t: cb::TakeItemEntity,
        entities: &mut E,
    ) {
        let collected = EntityId(t.collected);
        let collector = EntityId(t.collector);
        let Some((start, _, _)) = entities.pose(collected) else {
            return;
        };
        let kind = entities.entity_type(collected);
        let path = kind.and_then(|k| self.entity_type_path(k));
        let is_orb = path == Some("experience_orb");
        let is_item = path == Some("item");

        // `from.isSilent()`: the sound is skipped (the particle and the store update are not).
        let silent = matches!(
            entities.metadata_value(collected, SILENT_METADATA_INDEX),
            Some(MetadataValue::Boolean(true))
        );
        if !silent {
            // `(random.nextFloat() - random.nextFloat()) * spread + base`, the order the packet
            // handler draws the two floats.
            let r1 = self.pickup_rng.next_float();
            let r2 = self.pickup_rng.next_float();
            let (event, volume, pitch) = if is_orb {
                (
                    "minecraft:entity.experience_orb.pickup",
                    0.1,
                    (r1 - r2) * 0.35 + 0.9,
                )
            } else {
                ("minecraft:entity.item.pickup", 0.2, (r1 - r2) * 1.4 + 2.0)
            };
            self.sound_requests.push(SoundRequest::Play(PlayRequest {
                sound: SoundRef::Inline(event.to_owned()),
                category: CATEGORY_PLAYERS,
                pos: SoundPos::Fixed(start),
                volume,
                pitch,
                seed: self.pickup_rng.next_long(),
            }));
        }

        // The dropped stack as of before the shrink: it names the item, the copy count and
        // (through the entity's age) the spin and bob the particle freezes.
        let stack = match entities.metadata_value(collected, ITEM_OR_ORB_METADATA_INDEX) {
            Some(MetadataValue::ItemStack(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        let visual = if is_orb {
            let value = match entities.metadata_value(collected, ITEM_OR_ORB_METADATA_INDEX) {
                Some(MetadataValue::VarInt(v)) => *v,
                _ => 0,
            };
            Some(PickupVisual::Orb { value })
        } else {
            stack
                .as_ref()
                .and_then(|s| s.item)
                .map(|item| PickupVisual::Item {
                    item,
                    count: stack.as_ref().map_or(1, |s| s.count),
                })
        };
        if let (Some(visual), Some(target)) = (visual, self.collector_target(collector, entities)) {
            let age = entities.entity_age(collected).unwrap_or(0) as f32 + 1.0;
            self.pickup_particles.push(PickupParticle {
                id: collected,
                visual,
                age,
                start,
                target_old: target,
                target,
                collector,
                life: 0,
            });
        }

        // `itemEntity.getItem().shrink(amount)`, removed when it empties; an orb is left for
        // the server; anything else (arrows) is removed.
        if is_item {
            if let Some(mut stack) = stack {
                stack.count -= t.count;
                if stack.is_empty() {
                    entities.remove(&[collected]);
                } else {
                    entities.set_metadata(
                        collected,
                        &[MetadataEntry {
                            index: ITEM_OR_ORB_METADATA_INDEX,
                            value: MetadataValue::ItemStack(stack),
                        }],
                    );
                }
            }
        } else if !is_orb {
            entities.remove(&[collected]);
        }
    }

    /// The pickup particles' tick (vanilla `ParticleEngine.tick` -> `ItemPickupParticle.tick`):
    /// one tick older, the target position saved as the old one and re-read from the collector,
    /// removed after [`LIFE_TIME`] ticks.
    pub(super) fn tick_pickup_particles<E: EntitySink + ?Sized>(&mut self, entities: &E) {
        if self.pickup_particles.is_empty() {
            return;
        }
        for i in 0..self.pickup_particles.len() {
            let collector = self.pickup_particles[i].collector;
            let target = self.collector_target(collector, entities);
            let p = &mut self.pickup_particles[i];
            p.life += 1;
            p.target_old = p.target;
            if let Some(t) = target {
                p.target = t;
            }
        }
        self.pickup_particles.retain(|p| p.life < LIFE_TIME);
    }

    /// The collector's target position (vanilla `ItemPickupParticle.updatePosition`):
    /// `(getY() + getEyeY()) / 2` at the collector's own x/z. Our own player's eye height is
    /// exact; another entity's uses its default dimension eye height (`0.85 * height`).
    fn collector_target<E: EntitySink + ?Sized>(
        &self,
        collector: EntityId,
        entities: &E,
    ) -> Option<DVec3> {
        if collector == self.entity_id {
            let feet = self.player.state.pos;
            let eye = feet.y + self.player.eye_height();
            Some(DVec3::new(feet.x, (feet.y + eye) / 2.0, feet.z))
        } else {
            let (pos, _, _) = entities.pose(collector)?;
            let eye = entities
                .entity_type_info(collector)
                .map(|(_, (_, h))| f64::from(h) * 0.85)
                .unwrap_or(1.62);
            Some(DVec3::new(pos.x, pos.y + eye / 2.0, pos.z))
        }
    }

    /// The `minecraft:entity_type` registry path for a synced index (`None` for a type the
    /// registry does not name, e.g. a mod type).
    fn entity_type_path(&self, kind: RegistryIndex) -> Option<&str> {
        self.entity_type_names
            .get(kind.0 as usize)
            .map(|n| n.path())
    }

    /// A fresh unseeded generator (vanilla's `ClientPacketListener.random`, a thread-safe
    /// `RandomSource.createThreadSafe()`); the pickup pitch and sound seed are drawn from it.
    pub(super) fn new_pickup_rng() -> LegacyRandom {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0x5eed);
        LegacyRandom::new(seed)
    }
}

#[cfg(test)]
mod tests;
