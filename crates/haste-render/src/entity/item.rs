//! The item entity renderer (`ItemEntityRenderer`): dropped items with their spin, bob,
//! lift and stack-copy layout, and the shared item-model emitter every item draw goes
//! through (`ItemDisplayContext`).

use super::*;

impl EntityRenderer {
    pub(super) fn item_model_for(
        &mut self,
        i: haste_core::RegistryIndex,
        assets: &mut Assets,
    ) -> Option<Arc<haste_assets::ItemModel>> {
        if let Some(m) = self.item_models.get(&i.0) {
            return m.clone();
        }
        let m = self
            .item_names
            .get(i.0 as usize)
            .filter(|n| n.path() != "air")
            .and_then(|n| assets.item_model(n).ok())
            .map(Arc::new);
        self.item_models.insert(i.0, m.clone());
        m
    }
    /// Poses a dropped item entity (`minecraft:item`) into the vertex buffer, following
    /// `ItemEntityRenderer.submit` and `ItemClusterRenderState.submitMultipleFromCount`: the
    /// item's own model (the one the GUI and held items use) through its `display.ground`
    /// transform — which carries vanilla's ground scales, a flat item 0.5 and a block 0.25 —
    /// lifted so its bounding box sits 1/16 above the ground, bobbing and spinning with the
    /// entity's random phase, and a stack's copies spread like vanilla's. An unknown item
    /// draws the missing model (vanilla's missingno cube); nothing panics.
    pub(super) fn emit_dropped_item(
        &mut self,
        e: &EntityInstance,
        dropped: &DroppedItem,
        camera: glam::DVec3,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let model = self
            .item_model_for(dropped.item, assets)
            .unwrap_or_else(missing_item_model);
        // `ItemEntity.bobOffs`: a per-entity random phase in [0, 2pi); ours is a hash of
        // the entity id (the player cannot tell the difference).
        let bob_offs = entity_phase(e.id);
        // The model's bounding box through the ground transform: it decides the lift and
        // whether the copies of a stack spread in 3D or line up flat
        // (`submitMultipleFromCount` branches on the z size).
        let (bbox_min, bbox_max) = item_model_bbox(&model, &ItemDisplay::Ground);
        let lift = item_entity_lift(bbox_min.y);
        let base = Mat4::from_translation(
            (e.pos - camera).as_vec3()
                + Vec3::new(0.0, lift + item_entity_bob(dropped.age, bob_offs), 0.0),
        ) * Mat4::from_rotation_y(item_entity_spin(dropped.age, bob_offs));
        // `ItemClusterRenderState.getSeedForItemStack`: the renderer reseeds its random
        // with the item each frame, so the copy offsets are stable per item type (ours
        // hashes the registry id and stream index; matching Java's Random is not needed).
        let rand = |i: u32| seeded_float(dropped.item.0, i);
        let mut out = std::mem::take(&mut self.vertices);
        self.emit_item_copies(
            &mut out,
            &model,
            base,
            item_entity_copies(dropped.count),
            rand,
            light,
            ItemDisplay::Ground,
            (bbox_min, bbox_max),
            assets,
        );
        self.vertices = out;
    }
    /// The copies of an item stack, laid out like vanilla's
    /// `ItemEntityRenderer.submitMultipleFromCount`: the first copy at `base`, then one per
    /// extra copy of the stack, offset by `+-0.15` in all three axes when the model is thick,
    /// or lined up along z (by 1.5x the model's depth, jittered `+-0.075` in x and y) when
    /// it is flat. `rand` supplies the jitter in [0,1) by stream index; the block entities
    /// that draw a stack (the vault) and the dropped items share this exact layout.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_item_copies(
        &mut self,
        out: &mut Vec<EntityVertex>,
        model: &haste_assets::ItemModel,
        base: Mat4,
        copies: u32,
        rand: impl Fn(u32) -> f32,
        light: (u8, u8),
        display: ItemDisplay,
        bbox: (Vec3, Vec3),
        assets: &mut Assets,
    ) {
        let (bbox_min, bbox_max) = bbox;
        let depth = bbox_max.z - bbox_min.z;
        let flat = depth <= 1.0 / 16.0;
        let step = depth * 1.5;
        for copy in 0..copies {
            let mut off = Vec3::ZERO;
            if copy > 0 {
                if flat {
                    off = Vec3::new(
                        (rand(2 * copy - 2) * 2.0 - 1.0) * 0.075,
                        (rand(2 * copy - 1) * 2.0 - 1.0) * 0.075,
                        step * copy as f32 - step * (copies.saturating_sub(1)) as f32 / 2.0,
                    );
                } else {
                    off = Vec3::new(
                        (rand(3 * copy - 3) * 2.0 - 1.0) * 0.15,
                        (rand(3 * copy - 2) * 2.0 - 1.0) * 0.15,
                        (rand(3 * copy - 1) * 2.0 - 1.0) * 0.15,
                    );
                }
            }
            let m = base * Mat4::from_translation(off);
            self.emit_item(out, model, m, light, display, assets);
        }
    }
    /// Emits an item model's geometry through `root` (model space, 0..1 blocks, through the
    /// part's display transform) into `out`. `display` picks which of vanilla's
    /// `ItemDisplayContext`s the model is drawn through (`firstperson_righthand` for the hand,
    /// `fixed` for item frames, `ground` for dropped items).
    pub(super) fn emit_item(
        &mut self,
        out: &mut Vec<EntityVertex>,
        model: &haste_assets::ItemModel,
        root: Mat4,
        light: (u8, u8),
        display: ItemDisplay,
        assets: &mut Assets,
    ) {
        for part in &model.parts {
            let (display, left) = match display {
                ItemDisplay::FirstPerson => (&part.firstperson, false),
                ItemDisplay::ThirdPersonRight => (&part.thirdperson, false),
                // `ItemTransforms.getTransform(THIRD_PERSON_LEFT_HAND)` is the model's own
                // left entry (its right entry when it names no left one), and the left-hand
                // fix applies on top: `item/handheld`'s left entry mirrors the right exactly
                // so the fix cancels.
                ItemDisplay::ThirdPersonLeft => (&part.thirdperson_left, true),
                ItemDisplay::Head => (&part.head, false),
                ItemDisplay::Fixed => (&part.fixed, false),
                ItemDisplay::Ground => (&part.ground, false),
                ItemDisplay::OnShelf => (&part.on_shelf, false),
            };
            let m = root * item_display_matrix(part, display, left);
            let nm = Mat3::from_mat4(m).inverse().transpose();
            let tint = |i: Option<u32>| -> [f32; 3] {
                i.and_then(|i| part.tints.get(i as usize))
                    .map(|c| {
                        let f =
                            |s: u32| crate::sky::linear([((c >> s) & 0xFF) as f32 / 255.0; 3])[0];
                        [f(16), f(8), f(0)]
                    })
                    .unwrap_or([1.0; 3])
            };
            match &part.geometry {
                haste_assets::ItemGeometry::None => {}
                haste_assets::ItemGeometry::Generated { layers } => {
                    // The extruded sprite: front face, mirrored back face and the 1/16-deep
                    // sides at its opaque edges (vanilla `ItemModelGenerator`).
                    for (i, layer) in layers.iter().enumerate() {
                        let rect = self.texture_rect(&parse_tex(&layer.texture), assets);
                        let t = tint(Some(i as u32));
                        for q in &layer.quads {
                            let o = q.direction.offset();
                            push_world_quad(
                                out,
                                m,
                                nm,
                                q.positions.map(Vec3::from),
                                q.uvs,
                                Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                                t,
                                light,
                                rect,
                            );
                        }
                    }
                }
                haste_assets::ItemGeometry::Elements(quads) => {
                    for q in quads {
                        let rect = self.texture_rect(&parse_tex(&q.texture), assets);
                        let o = q.direction.offset();
                        push_world_quad(
                            out,
                            m,
                            nm,
                            q.positions.map(Vec3::from),
                            q.uvs,
                            Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                            tint(q.tint_index),
                            light,
                            rect,
                        );
                    }
                }
            }
        }
    }

    /// Item registry names by protocol id (for the held item), clearing the model cache.
    pub fn set_item_names(&mut self, names: Vec<Identifier>) {
        self.item_names = names;
        self.item_models.clear();
    }
}
