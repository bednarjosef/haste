//! The local player's own third-person model: one `haste-entity` model state ([`Entity`]),
//! ticked per tick from the client's own state (the server never spawns us as a tracked
//! entity) and drawn through the same `setupAnim`/`setupRotations` path a remote player
//! takes — the walk swing, body and head yaw, the attack swing, the crouch, swimming and
//! elytra-flight poses all come from the ported vanilla code, never hand-written poses.

use haste_core::entity::Pose;
use haste_core::math::Vec3;
use haste_core::model::{EntityInstance, PoseOverrides};
use haste_core::EntityId;

use super::shared::Shared;
use super::view::ViewState;
use crate::net::Session;

/// The local player's per-tick model tick, the tick's second-to-last step (after the physics
/// and the hand's tick, before the view publishes what the render thread needs): everything
/// [`haste_entity::Entity::tick_local`] reads is fed from the client's own state, in vanilla's
/// order (`LivingEntity.tick`'s walk/body-yaw/swim/fall-fly steps after `aiStep`'s movement,
/// `Player.aiStep`'s head yaw, and the hand's swing — vanilla has one
/// `LivingEntity.SwingState` for both hands, so the first-person hand's is mirrored in).
pub(super) fn tick(s: &Session, shared: &Shared, prev_look: (f32, f32), look: (f32, f32)) {
    // The feet bracket the tick like the eye does (the eye height can change within it), so
    // the previous position is the last view's, not the eye minus its height. Short lock,
    // taken and dropped before the model's: the render thread holds them the other way round.
    let (prev_pos, placed) = {
        let view = shared.view.lock();
        (view.feet, view.has_position)
    };
    let prev_pos = if placed { prev_pos } else { s.player.state.pos };
    // The swing mirrored from the hand: `Hand::attack`'s pair is (old, progress-or-wrapped);
    // the wrap comes back off here (`SwingState::animation` re-wraps a falling end itself).
    let (swing_old, swing_progress) = s.player.hand.attack();
    let swing_progress = if swing_progress > 1.0 {
        swing_progress - 1.0
    } else {
        swing_progress
    };
    let mut model = shared.local_model.lock();
    model.tick_local(haste_entity::LocalTick {
        prev_pos,
        pos: s.player.state.pos,
        prev_look,
        look,
        pose: super::view::player_pose(s).unwrap_or(Pose::Standing),
        passenger: s.player.vehicle.is_some(),
        hurt_ticks: s.hurt_ticks,
        dead: s.player.health <= 0.0,
        death_ticks: s.death_ticks,
        swing: haste_entity::MirroredSwing {
            swinging: s.player.hand.is_swinging(),
            hand: 0,
            duration: crate::hand::SWING_DURATION as i32,
            old: swing_old,
            progress: swing_progress,
        },
    });
    // The worn equipment in the tracked entities' shape: the held stack and the four armour
    // pieces by their wire slots (`set_equipment`'s numbering), read per tick by the render
    // thread through `equipment_by_slot` exactly like a remote player's. This is what draws
    // the armour (and an elytra's `WingsLayer`) on our own model.
    model.equipment.clear();
    if !s.player.main_hand().is_empty() {
        model.equipment.push((0, s.player.main_hand().clone()));
    }
    if !s.player.int.offhand.is_empty() {
        model.equipment.push((1, s.player.int.offhand.clone()));
    }
    for (wire, armor) in [(5u8, 0usize), (4, 1), (3, 2), (2, 3)] {
        if !s.player.int.armor[armor].is_empty() {
            model
                .equipment
                .push((wire, s.player.int.armor[armor].clone()));
        }
    }
    // The use state for the arm poses (`AvatarRenderer.getArmPose` reads `isUsingItem`):
    // mirrored from the local use. The off-hand bit stays false (which hand a local use
    // started on is not tracked here), so an off-hand use poses the main arm.
    model.metadata.living.using_item = s.player.is_using_item();
}

/// Pushes the local player's own instance onto the render list (the render thread's step for
/// the own model: one short lock on the shared model state, like the store read beside it).
/// Nothing while a spectator views from another entity (vanilla `LevelExtractor
/// .extractVisibleEntities` only draws a `LocalPlayer` that is the camera entity).
pub fn push_local_instance(
    instances: &mut Vec<EntityInstance>,
    shared: &Shared,
    local: (EntityId, u128),
    view: &ViewState,
    partial: f32,
    skin: &haste_entity::skin::PlayerSkin,
) {
    if !super::camera_entity::draws_local_player(view) {
        return;
    }
    let model = shared.local_model.lock();
    instances.push(player_instance(
        local.0,
        &model,
        view,
        partial,
        skin,
        &shared.data.item_components,
    ));
}

/// The local player's own render instance for third person (and the GUI's inventory preview,
/// which the caller draws even in first person): the humanoid model with the same
/// `setupAnim`/`setupRotations` a remote player's takes, dressed with our skin and cape by
/// [`super::skins::dress_local`]. `components` is the item default-components table the
/// equipment layers resolve their assets through, like the entity store does.
fn player_instance(
    id: EntityId,
    model: &haste_entity::Entity,
    view: &ViewState,
    partial: f32,
    skin: &haste_entity::skin::PlayerSkin,
    components: &haste_data::ItemComponents,
) -> EntityInstance {
    let humanoid = haste_entity::models::humanoid(false);
    let (hands, equipment, head_item) =
        haste_entity::equipment_by_slot(&model.equipment, components);
    // `AvatarRenderer` passes isPassenger from the shared riding flag we mirror each tick.
    let is_passenger = model.metadata.shared.riding;
    let pose: PoseOverrides =
        haste_entity::animation::setup_anim(model, &humanoid, true, partial, is_passenger);
    let body = haste_entity::animation::body_transform(model, partial, false);
    let mut inst = EntityInstance {
        id,
        model: humanoid,
        pos: model.prev_pos.lerp(model.pos, partial as f64),
        yaw: model.body_yaw_at(partial),
        pitch: model.pitch_at(partial),
        head_yaw: model.head_yaw_at(partial),
        scale: 1.0,
        pose,
        body,
        body_scale: Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        // `AvatarRenderer` passes shadowRadius 0.5 (LivingEntityRenderer, player scale 1).
        shadow_radius: 0.5,
        shadow_strength: 1.0,
        invisible: false,
        // `Entity.displayFireAnimation`: on fire and not a spectator.
        display_fire: view.screen.on_fire && !view.spectator,
        width: view.bb.0,
        height: view.bb.1,
        is_discrete: model.metadata.shared.crouching,
        // `EntityAttachment.NAME_TAG` AT_HEIGHT fallback = the player's 1.8 bb height.
        name_tag_y: 1.8,
        // Our own model flashes red and tips over in third person like any entity
        // (`hurt_strength`, and the death tilt inside `body_transform`).
        hurt: haste_entity::store::hurt_strength(model),
        // Full sky until the frame pass samples the world (`EntityInstance::sample_light`).
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: None,
        hands,
        equipment,
        head_item,
    };
    super::skins::dress_local(&mut inst, skin, view, partial);
    inst
}
