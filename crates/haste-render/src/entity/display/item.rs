//! The item display renderer (`DisplayRenderer.ItemDisplayRenderer`): the item's model
//! through the display's `ItemDisplayContext`, turned half a turn about Y first.

use haste_core::display::ItemDisplayContext;

use super::*;

/// The item renderer's display context for an `ItemDisplayContext`.
pub(crate) fn item_display_for(context: ItemDisplayContext) -> ItemDisplay {
    match context {
        ItemDisplayContext::None => ItemDisplay::None,
        ItemDisplayContext::ThirdPersonLeftHand => ItemDisplay::ThirdPersonLeft,
        ItemDisplayContext::ThirdPersonRightHand => ItemDisplay::ThirdPersonRight,
        ItemDisplayContext::FirstPersonLeftHand => ItemDisplay::FirstPersonLeft,
        ItemDisplayContext::FirstPersonRightHand => ItemDisplay::FirstPerson,
        ItemDisplayContext::Head => ItemDisplay::Head,
        ItemDisplayContext::Gui => ItemDisplay::Gui,
        ItemDisplayContext::Ground => ItemDisplay::Ground,
        ItemDisplayContext::Fixed => ItemDisplay::Fixed,
        ItemDisplayContext::OnShelf => ItemDisplay::OnShelf,
    }
}

/// `ItemDisplayRenderer.submitInner`'s turn before the item: `rotate(Axis.YP, PI)`.
pub(crate) fn item_display_turn() -> Mat4 {
    Mat4::from_rotation_y(std::f32::consts::PI)
}

impl EntityRenderer {
    /// One item display (`ItemDisplayRenderer.submitInner`): nothing for an empty stack,
    /// else the item's model (the missing model for an item without one, like dropped items)
    /// half a turn about Y, through the display context's transform and the item renderer's
    /// usual centring (`ItemStackRenderState.submit`).
    pub(super) fn emit_item_display(
        &mut self,
        pose: Mat4,
        item: Option<haste_core::RegistryIndex>,
        context: ItemDisplayContext,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let Some(item) = item else {
            return;
        };
        let model = self
            .item_model_for(item, assets)
            .unwrap_or_else(missing_item_model);
        let mut out = std::mem::take(&mut self.vertices);
        let root = pose * item_display_turn();
        self.emit_item(
            &mut out,
            &model,
            root,
            light,
            item_display_for(context),
            haste_core::model::FoilType::None,
            assets,
        );
        self.vertices = out;
    }
}
