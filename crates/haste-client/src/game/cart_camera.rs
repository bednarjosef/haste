//! Riding a minecart on the new behaviour: vanilla `Camera.alignWithEntity`'s minecart branch
//! (the camera follows the cart's track walk at the frame's partial tick, not the straight
//! line between the tick positions) and `EntityRenderer.extractRenderState`'s
//! `passengerOffset` for our own third-person model. The walk is `haste-entity`'s
//! `new_minecart_behavior`; the tick publishes what the frame needs into [`ViewState`].

use glam::DVec3;
use haste_core::EntityId;
use haste_entity::minecart::{variant_for_kind, Variant};
use haste_entity::{Entity, EntityStore};

use super::shared::Shared;
use super::view::ViewState;

/// The cart we ride, as the tick left it: `Camera.alignWithEntity` takes the camera from the
/// cart's walk plus our seat offset (`getPassengerRidingPosition - position -
/// getVehicleAttachmentPoint`, which is where our feet sit relative to the cart this tick).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CartCamera {
    /// The cart's entity id.
    pub cart: EntityId,
    /// The cart's tick count when the view was published: a frame drawn after the store has
    /// ticked again but before the next publish falls back to the plain eye.
    pub cart_tick: u64,
    /// Our feet before and after the tick (`LocalPlayer::camera`'s own bracket).
    pub prev_feet: DVec3,
    pub feet: DVec3,
    /// Our feet relative to the cart's position this tick.
    pub offset: DVec3,
}

/// The tick's [`CartCamera`]: `Some` while we ride a `Minecart` (vanilla's check is
/// `instanceof Minecart`, the rideable cart) that runs `NewMinecartBehavior`.
pub(super) fn extract(s: &crate::net::Session, shared: &Shared) -> Option<CartCamera> {
    let vehicle = s.player.vehicle()?;
    let store = shared.entities.read();
    let cart = store.get(vehicle)?;
    if variant_for_kind(cart.kind) != Some(Variant::Plain) || cart.new_minecart.is_none() {
        return None;
    }
    Some(CartCamera {
        cart: vehicle,
        cart_tick: cart.tick_count,
        prev_feet: s.player.prev_pos,
        feet: s.player.state.pos,
        offset: s.player.state.pos - cart.pos,
    })
}

/// The cart of `view`'s [`CartCamera`] when it is still the tick the view was published for.
fn cart<'a>(view: &ViewState, store: &'a EntityStore) -> Option<(&'a Entity, CartCamera)> {
    let c = view.cart_camera?;
    let cart = store.get(c.cart)?;
    (cart.tick_count == c.cart_tick).then_some((cart, c))
}

/// The camera's eye at `partial` (`Camera.alignWithEntity`): while we ride a cart walking its
/// track steps, the walk's point plus our seat offset plus the camera's eye height; otherwise
/// the eye interpolated between the tick's two (`LocalPlayer::camera`).
pub fn camera_eye(view: &ViewState, shared: &Shared, partial: f32) -> DVec3 {
    let linear = super::interpolate(view.prev_eye, view.eye, partial);
    if view.cart_camera.is_none() {
        return linear;
    }
    let store = shared.entities.read();
    let Some((cart, c)) = cart(view, &store) else {
        return linear;
    };
    let Some(walk) = cart
        .new_minecart
        .as_deref()
        .filter(|b| b.cart_has_pos_rot_lerp())
        .map(|b| b.cart_lerp_position(partial))
    else {
        return linear;
    };
    riding_eye(linear, walk, &c, partial)
}

/// `Camera.alignWithEntity`'s riding position: the cart's walk at `partial`, plus our seat
/// offset, plus the eye height the camera smooths (`eyeHeightOld`/`eyeHeight`), which is the
/// plain interpolated eye (`linear`) minus the interpolated feet.
fn riding_eye(linear: DVec3, walk: DVec3, c: &CartCamera, partial: f32) -> DVec3 {
    let eye_height = linear - super::interpolate(c.prev_feet, c.feet, partial);
    walk + c.offset + eye_height
}

/// Our own model's `passengerOffset` at `partial` while we ride a cart walking its track
/// steps (`None` otherwise): it is drawn moved with the walk like any passenger.
pub(super) fn passenger_offset(view: &ViewState, shared: &Shared, partial: f32) -> Option<DVec3> {
    view.cart_camera?;
    let store = shared.entities.read();
    let (cart, _) = cart(view, &store)?;
    haste_entity::new_minecart_behavior::passenger_offset(cart, partial)
}

#[cfg(test)]
mod tests;
