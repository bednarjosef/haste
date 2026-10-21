//! `CapeLayer`: a player's cape is an overlay layer (haste-entity puts the `PlayerCapeModel`
//! layer, textured with the cape, into the instance's overlays), drawn like any overlay except
//! that it is skipped under an elytra and pushed out over a chestplate.
use glam::{Mat4, Vec3};
use haste_assets::Assets;
use haste_core::model::{CubeModel, EntityInstance, EquipmentSlot};

use super::EntityRenderer;

/// `CapeLayer.submit`'s offset over chest armour, (0, -0.053125, 0.06875) blocks in the
/// layer's pose, in model units (1/16 block).
const OVER_ARMOUR: Vec3 = Vec3::new(0.0, -0.053125 * 16.0, 0.06875 * 16.0);

/// Whether an overlay model is the cape layer (its body carries a `cape` part).
fn is_cape_layer(model: &CubeModel) -> bool {
    model
        .root
        .children
        .iter()
        .any(|p| p.name == "body" && p.children.iter().any(|c| c.name == "cape"))
}

/// Whether an overlay model is the riptide spin layer (`SpinAttackEffectLayer`: its root holds
/// the rotating `box0` and `box1`, built by haste-entity's `spin_attack`).
fn is_spin_layer(model: &CubeModel) -> bool {
    model.root.children.iter().any(|p| p.name == "box0")
}

impl EntityRenderer {
    /// The transform an overlay layer draws through and its hurt flash: `root` and the
    /// entity's flash for every layer but the cape, which follows `CapeLayer.submit`: nothing
    /// when the chest item's equipment asset has a wings layer (an elytra), shifted by
    /// [`OVER_ARMOUR`] when it has a humanoid one, and never flashing (`NO_OVERLAY`).
    pub(super) fn overlay_root(
        &mut self,
        e: &EntityInstance,
        model: &CubeModel,
        root: Mat4,
        assets: &mut Assets,
    ) -> Option<(Mat4, f32)> {
        if is_spin_layer(model) {
            return Some((root, 0.0));
        }
        if !is_cape_layer(model) {
            return Some((root, e.hurt));
        }
        let chest = e
            .equipment
            .iter()
            .find(|p| p.slot == EquipmentSlot::Chest)
            .and_then(|p| p.asset.as_ref());
        let Some(info) = chest.and_then(|a| self.equipment_info(a, assets)) else {
            return Some((root, 0.0));
        };
        if info.layers_of("wings").next().is_some() {
            return None;
        }
        if info.layers_of("humanoid").next().is_some() {
            return Some((root * Mat4::from_translation(OVER_ARMOUR), 0.0));
        }
        Some((root, 0.0))
    }
}

/// `WingsLayer.getPlayerElytraTexture`: a player's elytra wears its cape (when the cape part
/// shows: haste-entity only adds the cape layer then) on layers that `use_player_texture`.
/// Profile elytra textures are not downloaded, so the cape is the only override.
pub(super) fn player_elytra_texture(e: &EntityInstance) -> Option<&haste_core::Identifier> {
    e.overlays
        .iter()
        .flatten()
        .find(|o| is_cape_layer(&o.model))
        .map(|o| &o.model.texture)
}
