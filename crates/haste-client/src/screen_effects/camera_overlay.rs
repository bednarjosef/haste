//! The worn item's camera overlay (`Hud.extractCameraOverlays`: the carved pumpkin).
use haste_protocol::haste_core::Identifier;
/// The camera overlay texture of a worn item: vanilla reads the equippable component's
/// `camera_overlay` (`Hud.extractCameraOverlays`), and `Items.CARVED_PUMPKIN` is the only
/// vanilla item that sets one (`Blocks.CARVED_PUMPKIN` -> `textures/misc/pumpkinblur.png`).
/// Ids we do not know draw nothing.
pub fn camera_overlay(item: &Identifier) -> Option<Identifier> {
    (item.path() == "carved_pumpkin")
        .then(|| Identifier::minecraft("textures/misc/pumpkinblur.png"))
}
