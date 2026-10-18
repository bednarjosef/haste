//! The pickup particles' plumbing between the game and render threads: the game thread
//! publishes the session's particles once per tick, and the render thread appends one
//! entity-renderer instance per particle at its partial-tick eased position (vanilla
//! `ItemPickupParticle` + `ItemPickupParticleGroup`), drawn through the dropped-item renderer.

use std::sync::{Arc, OnceLock};

use glam::Vec3;
use haste_core::model::{
    BodyTransform, CubeModel, Decoration, DroppedItem, EntityInstance, ExperienceOrbDecoration,
    PoseOverrides,
};

use super::shared::Shared;
use crate::net::pickup::{PickupParticle, PickupVisual};

/// The placeholder model the item/orb instances carry: it is ignored by the dropped-item and
/// orb renderers, and only its extents feed the frustum/distance cull. A dropped item entity's
/// box is 0.25 (vanilla `ItemEntity` dimensions); orbs are near enough.
fn pickup_model() -> Arc<CubeModel> {
    static MODEL: OnceLock<Arc<CubeModel>> = OnceLock::new();
    MODEL
        .get_or_init(|| haste_entity::models::box_for(0.25, 0.25))
        .clone()
}

/// Copies the session's pickup particles into the shared buffer the render thread reads
/// (once per tick; the buffer's capacity is reused, no per-frame work).
pub(super) fn publish(driver: &crate::net::NetDriver, shared: &Shared) {
    let mut out = shared.pickup_particles.lock();
    out.clear();
    out.extend(driver.session.pickup_particles.iter().cloned());
}

/// Appends this frame's pickup instances at `partial` to the entity instance list.
pub(crate) fn instances(shared: &Shared, partial: f32, out: &mut Vec<EntityInstance>) {
    let particles = shared.pickup_particles.lock();
    for p in particles.iter() {
        out.push(instance(p, partial));
    }
}

/// One pickup particle as a renderer instance (vanilla hands the collected entity's render
/// state to the particle, so an item draws as the item and an orb as the orb).
fn instance(p: &PickupParticle, partial: f32) -> EntityInstance {
    let pos = crate::net::pickup::pickup_position(p, partial);
    let (item, decoration) = match p.visual {
        PickupVisual::Item { item, count } => (
            Some(DroppedItem {
                item,
                count,
                age: p.age,
            }),
            None,
        ),
        PickupVisual::Orb { value } => (
            None,
            Some(Decoration::ExperienceOrb(ExperienceOrbDecoration {
                value,
                age: p.age,
            })),
        ),
    };
    EntityInstance {
        id: p.id,
        model: pickup_model(),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: PoseOverrides::default(),
        body: BodyTransform::default(),
        body_scale: Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.0,
        shadow_strength: 0.75,
        invisible: false,
        is_discrete: false,
        name_tag_y: 0.0,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item,
        decoration,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}
