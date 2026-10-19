//! The local player's own model state (`LocalPlayer`/`AbstractClientPlayer`): the client's
//! player, which the server never spawns as a tracked entity. The client creates one
//! [`Entity`] at join and feeds it per tick through [`Entity::tick_local`], so its model runs
//! the same `setupAnim`/`setupRotations` path a remote player's does — fed with exact state
//! instead of packets. Also the worn-equipment resolution both paths share.
use haste_core::entity::{ItemStackData, Pose};
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::DVec3;
use uuid::Uuid;

use crate::dims::EntityDimensions;
use crate::store::{Entity, DEATH_TICKS};
use crate::swing::SwingState;

impl Entity {
    /// The local player's model state, created at join with the login's entity id and profile
    /// id. Ticked per tick from the client's own state through [`Entity::tick_local`].
    pub fn local_player(id: EntityId, uuid: Uuid, pos: DVec3, yaw: f32, pitch: f32) -> Entity {
        // The player's own entity type index, when the entity registry table is loaded.
        let kind = crate::registry::entity_type_names()
            .and_then(|names| {
                names
                    .iter()
                    .find(|(_, name)| **name == "minecraft:player")
                    .map(|(k, _)| *k)
            })
            .unwrap_or(RegistryIndex(0));
        let mut e = Entity::new(
            &haste_core::entity::EntitySpawn {
                id,
                uuid,
                kind,
                pos,
                yaw,
                pitch,
                head_yaw: yaw,
                data: 0,
                velocity: DVec3::ZERO,
            },
            &EntityDimensions::default(),
        );
        e.is_player_type = true;
        e
    }

    /// The local player's per-tick animation step, in vanilla's order: `Player.aiStep`'s head
    /// yaw (the look yaw), `LivingEntity.calculateEntityAnimation`'s walk (from the exact
    /// per-tick movement, no packet lerp), `LivingEntity.tick`'s body yaw (`tickHeadTurn`, fed
    /// `yBodyRotT` from the movement like the store's) and fall-flying counter, and
    /// `updateSwimAmount`. The swing is mirrored from the client's own hand swing (one
    /// `LivingEntity.SwingState` serves both hands), and the hurt/death counters come from the
    /// client's (`hurt_strength`/`animation::death_time` read them exactly as for a tracked
    /// entity).
    pub fn tick_local(&mut self, tick: LocalTick) {
        self.tick_count += 1;
        self.prev_body_yaw = self.body_yaw;
        self.prev_head_yaw = self.head_yaw;
        self.prev_pos = tick.prev_pos;
        self.pos = tick.pos;
        self.prev_yaw = tick.prev_look.0;
        self.yaw = tick.look.0;
        self.prev_pitch = tick.prev_look.1;
        self.pitch = tick.look.1;
        // `Player.aiStep`: the head faces the look directly (remote players ease it over the
        // `rotate_head` packet's three ticks; ours is exact).
        self.head_yaw = tick.look.0;
        self.pose = tick.pose;
        // The shared flags the render path reads off a player's metadata (`isCrouching` drives
        // the model's crouch branch, `isPassenger` the riding pose, `isFallFlying` the glide
        // rotations): the client's own physics, not synced entity data.
        self.metadata.shared.crouching = tick.pose == Pose::Crouching;
        self.metadata.shared.riding = tick.passenger;
        self.metadata.shared.elytra_flying = tick.pose == Pose::FallFlying;
        let d = self.pos - self.prev_pos;
        let dist = f32::sqrt((d.x * d.x + d.z * d.z) as f32);
        // `LivingEntity.calculateEntityAnimation`: passengers and the dead stop walking.
        if !tick.passenger && !tick.dead {
            self.walk.update((dist * 4.0).min(1.0), 0.4, 1.0);
        } else {
            self.walk.stop();
        }
        crate::store::update_body_yaw(self, d);
        // `LivingEntity.updateSwimAmount` (smoothed 0.09 per tick).
        self.swim_amount_o = self.swim_amount;
        if self.pose == Pose::Swimming {
            self.swim_amount = (self.swim_amount + 0.09).min(1.0);
        } else {
            self.swim_amount = (self.swim_amount - 0.09).max(0.0);
        }
        self.swing.mirror(tick.swing);
        // The hurt flash and death tilt: the client's own counters (a tracked entity counts
        // them down itself; here they are inputs).
        if tick.dead {
            self.dead = true;
            self.hurt_ticks = DEATH_TICKS.saturating_sub(tick.death_ticks.min(DEATH_TICKS));
        } else {
            self.dead = false;
            self.hurt_ticks = tick.hurt_ticks;
        }
        // `LivingEntity.tick`: fallFlyTicks counts while the fall-flying flag is set.
        if self.metadata.shared.elytra_flying {
            self.fall_flying_ticks = self.fall_flying_ticks.saturating_add(1);
        } else {
            self.fall_flying_ticks = 0;
        }
    }
}

impl SwingState {
    /// The local player's swing, mirrored from the client's own hand swing (`haste-client`'s
    /// `Hand`, which ticks the same `LivingEntity.SwingState` for the first-person hand):
    /// instead of `swing_animation` packets and `SwingState.tick`, the client's swing IS this
    /// state, so both hands stay in step. `old`/`progress` are the pair `Hand::attack` gives
    /// (unwrapped; `SwingState::animation` re-wraps a falling end itself).
    pub fn mirror(&mut self, swing: MirroredSwing) {
        self.active = swing.swinging;
        self.hand = swing.hand;
        self.duration = swing.duration;
        self.old_progress = swing.old;
        self.progress = swing.progress;
    }
}

/// What the client feeds [`Entity::tick_local`] each tick for its own model: its movement and
/// look bracketing the tick (the look's yaw drives the head, its pitch the model's pitch),
/// the physics pose, whether it rides, its hurt/death counters and its hand's swing.
#[derive(Clone, Copy)]
pub struct LocalTick {
    /// Feet position at the start of the tick.
    pub prev_pos: DVec3,
    /// Feet position after the tick.
    pub pos: DVec3,
    /// Look (yaw, pitch) at the start and end of the tick, degrees.
    pub prev_look: (f32, f32),
    pub look: (f32, f32),
    /// The physics pose (standing, crouching, swimming, fall-flying).
    pub pose: Pose,
    /// Whether the client rides a vehicle (`set_passengers`).
    pub passenger: bool,
    /// Hurt ticks left (`hurtTime` runs to 10).
    pub hurt_ticks: u32,
    /// Whether the player is dead (`LivingEntity.isDeadOrDying`).
    pub dead: bool,
    /// Ticks since the player died (vanilla `deathTime`), when dead.
    pub death_ticks: u32,
    /// The hand's swing, mirrored (see [`SwingState::mirror`]).
    pub swing: MirroredSwing,
}

/// The local player's swing as [`SwingState::mirror`] takes it: the client's own
/// first-person-hand swing state, which is the same `LivingEntity.SwingState`.
#[derive(Clone, Copy, Debug)]
pub struct MirroredSwing {
    /// `LivingEntity.isSwinging`.
    pub swinging: bool,
    /// The swinging hand (0 main, 1 off).
    pub hand: i32,
    /// The swing's duration in ticks (`SwingAnimation.DEFAULT`: 6).
    pub duration: i32,
    /// The swing progress at the end of the previous tick.
    pub old: f32,
    /// The swing progress at the end of the tick.
    pub progress: f32,
}

/// The hands, equipment layers and custom head item for worn stacks by their wire slots: the
/// item in each hand (main hand -> right, off hand -> left), every armour piece whose item's
/// `equippable` component names the same slot (vanilla `HumanoidArmorLayer.shouldRender`), and
/// the head-slot item when it is not armour (vanilla `CustomHeadLayer`: pumpkins, heads). The
/// equipment asset id comes from the item's default `equippable` component; the dye from the
/// stack's `dyed_color` patch. Tracked entities get theirs from `set_equipment` packets; the
/// local player's own inventory is fed in the same shape by the client.
pub fn equipment_by_slot(
    stacks: &[(u8, ItemStackData)],
    components: &haste_data::ItemComponents,
) -> (
    [Option<RegistryIndex>; 2],
    Vec<haste_core::model::EquipmentPiece>,
    Option<RegistryIndex>,
) {
    use haste_core::model::{EquipmentPiece, EquipmentSlot};
    let mut hands = [None, None];
    let mut equipment = Vec::new();
    let mut head_item = None;
    for (wire, stack) in stacks {
        if stack.is_empty() {
            continue;
        }
        let Some(item) = stack.item else { continue };
        match EquipmentSlot::from_wire(*wire) {
            Some(EquipmentSlot::MainHand) => hands[0] = Some(item),
            Some(EquipmentSlot::OffHand) => hands[1] = Some(item),
            Some(slot) if slot.is_armor() => {
                let equippable = components.equippable(item);
                let is_armor = equippable.map(|x| x.slot) == Some(slot);
                if slot == EquipmentSlot::Head && !is_armor {
                    // Not a helmet: `CustomHeadLayer` draws it on the head instead.
                    head_item = Some(item);
                    continue;
                }
                if !is_armor {
                    continue;
                }
                equipment.push(EquipmentPiece {
                    slot,
                    item: Some(item),
                    asset: equippable.and_then(|x| x.asset.clone()),
                    dye: components.dyed_color(&stack.components_raw),
                });
            }
            _ => {}
        }
    }
    (hands, equipment, head_item)
}

/// A tracked entity's synced equipment, resolved through the shared per-slot rules
/// ([`equipment_by_slot`]).
pub(crate) fn equipment_of(
    e: &crate::store::Entity,
    components: &haste_data::ItemComponents,
) -> (
    [Option<RegistryIndex>; 2],
    Vec<haste_core::model::EquipmentPiece>,
    Option<RegistryIndex>,
) {
    equipment_by_slot(&e.equipment, components)
}
