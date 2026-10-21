//! The humanoid equipment layers: held items (`ItemInHandLayer`), armour
//! (`HumanoidArmorLayer`), elytra wings and the custom head layer.

use super::*;

/// The bridge from the model's pixel space to the block units vanilla's layer transforms use:
/// `root` ends in `scale(s / 16)` (model pixels -> blocks), so the arm part's pivot comes out in
/// blocks, but everything `ItemInHandLayer.submitArmWithItem` and `CustomHeadLayer
/// .translateToHead` push afterwards (the hand offset `(±1, 2, -10) / 16`, the head's
/// `(0, -0.25, 0)` and its `(0.625, -0.625, -0.625)` scale) and the item's own 0..1-block
/// geometry are in block units — vanilla's `ModelPart.translateAndRotate` divides only the
/// pivot by 16. Scaling by 16 right after the arm matrix undoes the model's pixel scale for
/// the layer transform and the item, leaving the pivot converted.
pub(crate) fn pixel_units() -> Mat4 {
    Mat4::from_scale(Vec3::splat(16.0))
}

/// `EquipmentLayerRenderer.getColorForLayer` as a tint: a dyeable layer takes the stack's dye
/// (made opaque, `DyedItemColor.getOrDefault`), else its undyed colour, and is not drawn at all
/// (`None`, vanilla's colour 0) when it has neither; any other layer draws untinted.
pub(crate) fn layer_tint(
    layer: &haste_assets::EquipmentLayer,
    dye: Option<u32>,
) -> Option<[f32; 3]> {
    match layer.dyeable {
        Some(d) => dye
            .map(|c| c | 0xFF00_0000)
            .or(d.color_when_undyed)
            .map(argb_linear),
        None => Some([1.0; 3]),
    }
}

impl EntityRenderer {
    /// Draws an entity's equipment on top of its model (already emitted at `root`): the held
    /// items (`ItemInHandLayer`) and armour (`HumanoidArmorLayer`). Unknown items and equipment
    /// draw nothing. The vertex buffer is reused (taken out and put back) so nothing allocates
    /// per frame.
    pub(super) fn emit_equipment(
        &mut self,
        e: &EntityInstance,
        root: Mat4,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        // Held items: vanilla draws the right hand first, then the left.
        for (i, item) in e.hands.iter().enumerate() {
            let Some(idx) = *item else { continue };
            let Some(model) = self.item_model_for(idx, assets) else {
                continue;
            };
            let left = i == 1;
            let arm = if left {
                parts::LEFT_ARM
            } else {
                parts::RIGHT_ARM
            };
            let Some(arm_m) = part_matrix(&e.model.root, &e.pose, Mat4::IDENTITY, arm) else {
                continue;
            };
            // A small stand's arm carries the small layer's 1/2 part scale (vanilla's pose
            // scale on the arm's frame), so its held item scales with it.
            let arm_scale = if armor_stand::small_of(&e.model) == Some(true) {
                Mat4::from_scale(Vec3::splat(0.5))
            } else {
                Mat4::IDENTITY
            };
            let item_root = root * arm_m * arm_scale * pixel_units() * held_item_layer_matrix(left);
            let display = if left {
                ItemDisplay::ThirdPersonLeft
            } else {
                ItemDisplay::ThirdPersonRight
            };
            let mut out = std::mem::take(&mut self.vertices);
            self.emit_item(&mut out, &model, item_root, light, display, assets);
            self.vertices = out;
        }
        // Armour, in vanilla's order (`HumanoidArmorLayer.submit`: chest, legs, feet, head).
        // The armour draws with its own model — the humanoid shapes for players and mobs,
        // the armour stand's (`ArmorStandArmorModel`) for stands, small or big like their
        // body model — and with the body pose's rotations and pivot offsets but none of its
        // hidden parts: vanilla's armour models are separate, so parts the body model hides
        // (a player's hidden skin layers, a stand's hidden arms) still draw their armour
        // (the stand's floating chestplate shoulders are vanilla behaviour).
        let armor_pose = pose_without_hidden(&e.model, &e.pose);
        let stand_small = armor_stand::small_of(&e.model);
        for slot in [
            haste_core::model::EquipmentSlot::Chest,
            haste_core::model::EquipmentSlot::Legs,
            haste_core::model::EquipmentSlot::Feet,
            haste_core::model::EquipmentSlot::Head,
        ] {
            let Some(piece) = e.equipment.iter().find(|p| p.slot == slot) else {
                continue;
            };
            let Some(asset) = &piece.asset else { continue };
            let Some(info) = self.equipment_info(asset, assets) else {
                continue;
            };
            // `HumanoidArmorLayer`: leggings use the inner (leggings) layer, the rest the
            // humanoid one (babies are not modelled).
            let layer_type = if slot == haste_core::model::EquipmentSlot::Legs {
                "humanoid_leggings"
            } else {
                "humanoid"
            };
            let model = match stand_small {
                Some(small) => armor_stand::armor_for(slot, small),
                None => humanoid_armor(slot),
            };
            let mut out = std::mem::take(&mut self.vertices);
            for layer in info.layers_of(layer_type) {
                let Some(tint) = layer_tint(layer, piece.dye) else {
                    continue;
                };
                let rect = self.texture_rect(&layer.texture, assets);
                // A small stand's armour goes through the small layer's per-part scale (the
                // armour model is authored with big cubes and small pivots, like the body).
                match stand_small {
                    Some(_) => {
                        for child in &model.root.children {
                            emit_part_tinted(
                                &mut out,
                                &model,
                                child,
                                &armor_pose,
                                root * armor_stand::small_part_scale(child),
                                rect,
                                light,
                                e.hurt,
                                tint,
                            );
                        }
                    }
                    None => emit_part_tinted(
                        &mut out,
                        &model,
                        &model.root,
                        &armor_pose,
                        root,
                        rect,
                        light,
                        e.hurt,
                        tint,
                    ),
                }
            }
            // Wings (`WingsLayer`): an elytra on the chest draws the WINGS layers on the elytra
            // model, pushed 0.125 blocks back (the wings' flap animation is not modelled yet).
            if slot == haste_core::model::EquipmentSlot::Chest {
                let elytra = elytra_model();
                let wings_root = root * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.125));
                for layer in info.layers_of("wings") {
                    let Some(tint) = layer_tint(layer, piece.dye) else {
                        continue;
                    };
                    let texture = cape::player_elytra_texture(e)
                        .filter(|_| layer.use_player_texture)
                        .unwrap_or(&layer.texture);
                    let rect = self.texture_rect(texture, assets);
                    emit_part_tinted(
                        &mut out,
                        &elytra,
                        &elytra.root,
                        &e.pose,
                        wings_root,
                        rect,
                        light,
                        e.hurt,
                        tint,
                    );
                }
            }
            self.vertices = out;
        }
        // Custom head (vanilla `CustomHeadLayer`): the head-slot item when it is not armour
        // (pumpkins, heads), through the head part's animated transform.
        if let Some(idx) = e.head_item {
            if let Some(model) = self.item_model_for(idx, assets) {
                if let Some(head_m) =
                    part_matrix(&e.model.root, &e.pose, Mat4::IDENTITY, parts::HEAD)
                {
                    // A small stand's head carries the small layer's 3/4 part scale.
                    let head_scale = if armor_stand::small_of(&e.model) == Some(true) {
                        Mat4::from_scale(Vec3::splat(0.75))
                    } else {
                        Mat4::IDENTITY
                    };
                    let head_root =
                        root * head_m * head_scale * pixel_units() * custom_head_layer_matrix();
                    let mut out = std::mem::take(&mut self.vertices);
                    self.emit_item(
                        &mut out,
                        &model,
                        head_root,
                        light,
                        ItemDisplay::Head,
                        assets,
                    );
                    self.vertices = out;
                }
            }
        }
    }
    /// The equipment asset info, loaded once and cached (`None` for an unknown asset, never a
    /// panic and never a re-read).
    pub(super) fn equipment_info(
        &mut self,
        asset: &Identifier,
        assets: &mut Assets,
    ) -> Option<Arc<haste_assets::EquipmentInfo>> {
        if let Some(info) = self.equipment_infos.get(asset) {
            return info.clone();
        }
        let info = assets.equipment_info(asset).ok().map(Arc::new);
        if info.is_none() {
            tracing::warn!("equipment asset {asset} not found");
        }
        self.equipment_infos.insert(asset.clone(), info.clone());
        info
    }
}
