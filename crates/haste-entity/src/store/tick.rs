//! One entity's 20 Hz tick: the body every stored entity runs (`Entity.commonTick` and the
//! per-class `tick` steps), shared by the root entities and the passengers that
//! `ClientLevel.tickPassenger` ticks through their vehicle ([`crate::ride_tick`]).
use haste_core::entity::Pose;

use super::{Entity, CREEPER_MAX_SWELL};
use crate::arrow::{ArrowParticle, ArrowWorld};
use crate::dims::EntityDimensions;
use crate::item_entity::ItemWorld;
use crate::lerp::LerpTarget;
use crate::store::rot_lerp;
use crate::using_item::UseTick;

/// What one tick reads and writes besides the entity itself: the world views, the sizes of the
/// entity types, and the buffers the tick fills (reused, so a tick allocates nothing).
pub(crate) struct TickEnv<'a> {
    pub world: &'a dyn ItemWorld,
    pub arrows: &'a dyn ArrowWorld,
    pub arrow_particles: &'a mut Vec<ArrowParticle>,
    pub dims: &'a EntityDimensions,
    pub use_ticks: &'a mut Vec<UseTick>,
}

/// One entity's tick. `passenger` is whether it rides a vehicle: a passenger does not walk
/// (`LivingEntity.calculateEntityAnimation`) and its position is set by its vehicle afterwards.
pub(crate) fn tick_entity(slot: &mut Entity, env: &mut TickEnv<'_>, passenger: bool) {
    slot.tick_count += 1;
    slot.prev_body_yaw = slot.body_yaw;
    // `LivingEntity.baseTick` records `yHeadRotO`, then `aiStep` applies one
    // `lerpHeadRotationStep` (rotLerp by 1/steps toward the packet's head yaw).
    slot.prev_head_yaw = slot.head_yaw;
    if slot.lerp_head_steps > 0 {
        slot.head_yaw = rot_lerp(
            1.0 / f32::from(slot.lerp_head_steps),
            slot.head_yaw,
            slot.lerp_head_yaw,
        );
        slot.lerp_head_steps -= 1;
    }
    // `AbstractClientPlayer.tick`: the cloak follows the position before this tick's
    // movement (the interpolation step below).
    if slot.is_player_type {
        slot.cloak.tick(slot.pos);
    }
    slot.lerp.tick(&mut LerpTarget {
        pos: &mut slot.pos,
        prev_pos: &mut slot.prev_pos,
        yaw: &mut slot.yaw,
        prev_yaw: &mut slot.prev_yaw,
        pitch: &mut slot.pitch,
        prev_pitch: &mut slot.prev_pitch,
    });
    // `Display.tick`: the render state rebuilt from the synched data that changed.
    if let Some(display) = slot.display.as_mut() {
        display.tick(&slot.metadata, slot.tick_count);
    }
    // `AbstractMinecart.tick` -> `NewMinecartBehavior.tick`: the track walk.
    crate::new_minecart_behavior::tick(slot);
    // `ItemEntity.tick`: gravity, move, drag, between position packets.
    if slot.is_item {
        crate::item_entity::tick(slot, env.world);
    }
    // `AbstractArrow.tick`: the client-side flight simulation, between position packets
    // (arrows are `SERVER_AND_CLIENT` simulated like items).
    if crate::arrow::is_abstract_arrow(slot.kind) {
        crate::arrow::tick(slot, env.arrows, env.arrow_particles);
    }
    // `PrimedTnt.tick` runs on the client too: the fuse counts down locally between the
    // server's entity-data syncs (which overwrite it in lockstep).
    if slot.is_tnt {
        slot.metadata.tnt_fuse = slot.metadata.tnt_fuse.saturating_sub(1);
    }
    // `AbstractBoat.tick`'s client-side state: the paddle sweeps and the bubble-column rocking,
    // from the synced flags and bubble time.
    if slot.is_boat {
        let eye_height = env.dims.eye_height(slot.kind, false);
        crate::boat::tick(slot, env.world, eye_height);
    }
    // `LivingEntity.tick`'s use step (`updatingUsingItem`), after `super.tick()` like vanilla:
    // the tick is recorded for the client's effects.
    crate::using_item::step(slot, env.dims, env.use_ticks);
    let d = slot.pos - slot.prev_pos;
    let dist = f32::sqrt((d.x * d.x + d.z * d.z) as f32);
    // `LivingEntity.calculateEntityAnimation`: passengers and the dead stop walking.
    if !passenger && !slot.dead {
        let target = (dist * 4.0).min(1.0);
        // `LivingEntity.updateWalkAnimation`: babies swing at three times the position scale
        // (`isBaby() ? 3.0F : 1.0F`).
        let baby = !slot.is_player_type && crate::registry::is_baby(&slot.metadata);
        slot.walk.update(target, 0.4, if baby { 3.0 } else { 1.0 });
    } else {
        slot.walk.stop();
    }
    crate::walk_swing::update_body_yaw(slot, d);
    // `LivingEntity.updateSwimAmount` (smoothed 0.09 per tick).
    slot.swim_amount_o = slot.swim_amount;
    if slot.pose == Pose::Swimming {
        slot.swim_amount = (slot.swim_amount + 0.09).min(1.0);
    } else {
        slot.swim_amount = (slot.swim_amount - 0.09).max(0.0);
    }
    // `Chicken.aiStep`'s flap timers (harmless for every other model).
    slot.o_flap = slot.flap;
    slot.o_flap_speed = slot.flap_speed;
    slot.flap_speed += (if slot.on_ground { -1.0 } else { 4.0 }) * 0.3;
    slot.flap_speed = slot.flap_speed.clamp(0.0, 1.0);
    if !slot.on_ground && slot.flapping < 1.0 {
        slot.flapping = 1.0;
    }
    slot.flapping *= 0.9;
    slot.flap += slot.flapping * 2.0;
    slot.swing.tick();
    slot.hurt_ticks = slot.hurt_ticks.saturating_sub(1);
    // `Creeper.tick`: the client advances its own swell count from the synced direction (an
    // ignited creeper forces it to 1), clamped to [0, maxSwell].
    slot.old_swell = slot.swell;
    if slot.metadata.creeper_ignited {
        slot.metadata.creeper_swell_dir = 1;
    }
    slot.swell = (slot.swell + slot.metadata.creeper_swell_dir).clamp(0, CREEPER_MAX_SWELL);
    // `IronGolem.tick`: the attack animation tick counts down.
    slot.attack_ticks = slot.attack_ticks.saturating_sub(1);
    // `EvokerFangs.tick`: the bite's `lifeTicks` counts down once it has started.
    slot.fangs.tick();
    // `AbstractMinecart.tick`'s countdowns (hurt time, damage, the TNT cart's fuse); read only
    // by minecarts, harmless elsewhere.
    crate::minecart::tick(&mut slot.metadata);
    // `LivingEntity.tick`: fallFlyTicks counts while the fall-flying flag is set.
    if slot.metadata.shared.elytra_flying {
        slot.fall_flying_ticks = slot.fall_flying_ticks.saturating_add(1);
    } else {
        slot.fall_flying_ticks = 0;
    }
    slot.age = slot.age.saturating_add(1);
}
