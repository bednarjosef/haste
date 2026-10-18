//! The first-person hand: the player arm model and vanilla's `FirstPersonHandsAndItemsRenderer`
//! transforms (arm, item, layer and lag matrices).

use super::*;

/// The arm model of a first-person hand: the player model's `right_arm` with its
/// `right_sleeve` child (vanilla `HumanoidModel.createMesh` + `PlayerModel.createMesh`), in
/// model pixels. `slim` picks the 3 px arm. Built once and cached, so the hand pass allocates
/// nothing per frame.
pub(crate) fn arm_model(slim: bool, sleeve: bool) -> CubeModel {
    // Wide: (-3, -2, -2) + (4, 12, 4); slim: (-2, -2, -2) + (3, 12, 4).
    let origin_x = if slim { -2.0 } else { -3.0 };
    let w = if slim { 3.0 } else { 4.0 };
    let cube = |uv: [u32; 2], inflate: f32| haste_core::model::Cube {
        origin: Vec3::new(origin_x, -2.0, -2.0),
        size: Vec3::new(w, 12.0, 4.0),
        inflate,
        uv,
        mirror: false,
    };
    CubeModel {
        texture: Identifier::minecraft("entity/player/wide/steve"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            // `PartPose.offset(-5, 2, 0)` of the model's right_arm.
            name: "right_arm".into(),
            id: parts::RIGHT_ARM,
            pivot: Vec3::new(-5.0, 2.0, 0.0),
            // `AvatarRenderer.renderHand`: `rightArm.zRot = 0.1`.
            rotation: Vec3::new(0.0, 0.0, 0.1),
            visible: true,
            cubes: vec![cube([40, 16], 0.0)],
            // `AvatarRenderer.renderHand`: the sleeve shows with the right-sleeve part.
            children: vec![ModelPart {
                name: "right_sleeve".into(),
                id: parts::RIGHT_SLEEVE,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: sleeve,
                cubes: vec![cube([40, 32], 0.25)],
                children: vec![],
            }],
        },
    }
}

/// `ItemInHandLayer.submitArmWithItem`'s transform after `ArmedModel.translateToHand`: rotate X
/// -90 degrees, rotate Y 180 degrees, then the hand offset (`(±1, 2, -10)/16` for adults; a
/// left-hand item is offset to -1). The item then draws through `thirdperson_righthand`.
pub(crate) fn held_item_layer_matrix(left: bool) -> Mat4 {
    Mat4::from_rotation_x((-90.0f32).to_radians())
        * Mat4::from_rotation_y(180.0f32.to_radians())
        * Mat4::from_translation(Vec3::new(
            if left { -1.0 } else { 1.0 } / 16.0,
            2.0 / 16.0,
            -10.0 / 16.0,
        ))
}

/// `CustomHeadLayer.translateToHead` (the static one, after the head part's own transform):
/// translate `(0, -0.25, 0)`, rotate Y 180 degrees, scale `(0.625, -0.625, -0.625)`. The item
/// then draws through `display.head`.
pub(crate) fn custom_head_layer_matrix() -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
        * Mat4::from_rotation_y(180.0f32.to_radians())
        * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
}

/// `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`'s outer transforms, in view
/// space: the hand's lag behind the camera turn (`(viewXRot - xBob) * 0.1`,
/// `(viewYRot - yBob) * 0.1`). The walk bob is NOT here: vanilla applies `bobHurt` and
/// `bobView` to the whole hand pose stack in `GameRenderer.renderItemInHand` (the renderer
/// passes them as the prefix matrix), once, like the world's projection carries them.
pub(crate) fn hand_root_matrix(fp: &FirstPerson, view: (f32, f32), partial: f32) -> Mat4 {
    Mat4::from_rotation_x(((view.1 - fp.x_bob.at(partial)) * 0.1).to_radians())
        * Mat4::from_rotation_y(((view.0 - fp.y_bob.at(partial)) * 0.1).to_radians())
}

/// `FirstPersonHandsAndItemsRenderer.renderPlayerArm` for the empty hand: the arm's swing
/// translation and rotations, ending at the arm model's own transform. `inverse_arm_height` is
/// 0 when the item is fully held and 1 while it switches.
pub(crate) fn arm_matrix(fp: &FirstPerson, partial: f32) -> Mat4 {
    const ARM_SWING_X_POS_SCALE: f32 = -0.3;
    const ARM_SWING_Y_POS_SCALE: f32 = 0.4;
    const ARM_SWING_Z_POS_SCALE: f32 = -0.4;
    const ARM_SWING_Y_ROT_AMOUNT: f32 = 70.0;
    const ARM_SWING_Z_ROT_AMOUNT: f32 = -20.0;
    let attack = fp.attack.at(partial);
    let inverse_arm_height = 1.0 - fp.equip.at(partial);
    // Only the right hand is rendered (a left main hand is not modelled yet), so the arm's
    // `invert` factor is 1 throughout.
    let sqrt_attack = attack.max(0.0).sqrt();
    let x = ARM_SWING_X_POS_SCALE * (sqrt_attack * std::f32::consts::PI).sin();
    let y = ARM_SWING_Y_POS_SCALE * (sqrt_attack * std::f32::consts::PI * 2.0).sin();
    let z = ARM_SWING_Z_POS_SCALE * (attack * std::f32::consts::PI).sin();
    let mut m = Mat4::from_translation(Vec3::new(
        // `0.64000005` and `-0.71999997` are vanilla's own literals.
        x + 0.64000005,
        y - 0.6 + inverse_arm_height * -0.6,
        z - 0.71999997,
    ));
    m *= Mat4::from_rotation_y(45.0f32.to_radians());
    let z_swing_rotation = (attack * attack * std::f32::consts::PI).sin();
    let y_swing_rotation = (sqrt_attack * std::f32::consts::PI).sin();
    m *= Mat4::from_rotation_y((y_swing_rotation * ARM_SWING_Y_ROT_AMOUNT).to_radians());
    m *= Mat4::from_rotation_z((z_swing_rotation * ARM_SWING_Z_ROT_AMOUNT).to_radians());
    m *= Mat4::from_translation(Vec3::new(-1.0, 3.6, 3.5));
    m *= Mat4::from_rotation_z(120.0f32.to_radians());
    m *= Mat4::from_rotation_x(200.0f32.to_radians());
    m *= Mat4::from_rotation_y((-135.0f32).to_radians());
    m * Mat4::from_translation(Vec3::new(5.6, 0.0, 0.0))
}

/// `FirstPersonHandsAndItemsRenderer.applyItemArmTransform` + `swingArm` for a held item: the
/// item's place in view space (`0.56, -0.52 + inverseArmHeight * -0.6, -0.72`) with the swing's
/// translation and `applyItemArmAttackTransform`'s rotations.
pub(crate) fn item_arm_matrix(fp: &FirstPerson, partial: f32) -> Mat4 {
    const ITEM_SWING_X_POS_SCALE: f32 = -0.4;
    const ITEM_SWING_Y_POS_SCALE: f32 = 0.2;
    const ITEM_SWING_Z_POS_SCALE: f32 = -0.2;
    let attack = fp.attack.at(partial);
    let inverse_arm_height = 1.0 - fp.equip.at(partial);
    let mut m = Mat4::from_translation(Vec3::new(0.56, -0.52 + inverse_arm_height * -0.6, -0.72));
    // `swingArm`.
    let sqrt_attack = attack.max(0.0).sqrt();
    m *= Mat4::from_translation(Vec3::new(
        ITEM_SWING_X_POS_SCALE * (sqrt_attack * std::f32::consts::PI).sin(),
        ITEM_SWING_Y_POS_SCALE * (sqrt_attack * std::f32::consts::PI * 2.0).sin(),
        ITEM_SWING_Z_POS_SCALE * (attack * std::f32::consts::PI).sin(),
    ));
    // `applyItemArmAttackTransform`.
    let y_swing_rotation = (attack * attack * std::f32::consts::PI).sin();
    m *= Mat4::from_rotation_y((45.0 + y_swing_rotation * -20.0).to_radians());
    let xz_swing_rotation = (sqrt_attack * std::f32::consts::PI).sin();
    m *= Mat4::from_rotation_z((xz_swing_rotation * -20.0).to_radians());
    m *= Mat4::from_rotation_x((xz_swing_rotation * -80.0).to_radians());
    m * Mat4::from_rotation_y((-45.0f32).to_radians())
}

impl EntityRenderer {
    /// The empty hand's arm model for the skin's arm width, built once (no per-frame work).
    fn arm_model(&mut self, slim: bool, sleeve: bool) -> Arc<CubeModel> {
        let slot = &mut self.arm_models[usize::from(slim) * 2 + usize::from(sleeve)];
        slot.get_or_insert_with(|| Arc::new(arm_model(slim, sleeve)))
            .clone()
    }

    /// Builds the first-person arm or held item, following vanilla's
    /// `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`: the hurt tilt and walk bob
    /// (`GameRenderer.renderItemInHand`'s pose stack) and the hand's lag behind the camera
    /// turn are applied to the whole hand in view space, then per hand vanilla's own
    /// transform, then the model.
    ///
    /// `prefix` is the pose stack the hand is submitted under (view space stays view space:
    /// x right, y up, -z forward), `view` is the camera's look (yaw, pitch) in degrees and
    /// `partial` the partial tick; the per-tick animation values live in `fp` and are only
    /// lerped here.
    ///
    /// Nothing is allocated per frame: the arm model is cached by `slim` and the vertices go
    /// into the reused hand buffer.
    pub fn prepare_hand(
        &mut self,
        fp: Option<&FirstPerson>,
        prefix: Mat4,
        view: (f32, f32),
        partial: f32,
        assets: &mut Assets,
    ) {
        self.hand.clear();
        let Some(fp) = fp else {
            return;
        };
        // Hand normals are world-oriented: vanilla puts the inverse view rotation on the
        // hand's pose stack (`GameRenderer.renderItemInHand`) and the view rotation on the
        // model-view, so the fixed world light directions shade the arm/item as you look
        // around. Emit with the inverse rotation in front (world-oriented normals, so the
        // baked shade is vanilla's), then rotate the positions back below.
        let inv_view = hand_normal_rotation(view.0, view.1);
        // `FirstPerson::light` is (sky, block); the vertex wants (block, sky).
        let light = (fp.light.1, fp.light.0);
        let item = fp.item.and_then(|i| self.item_model_for(i, assets));
        let mut out = std::mem::take(&mut self.hand);
        let root = Mat4::from_mat3(inv_view) * prefix * hand_root_matrix(fp, view, partial);
        match item {
            Some(model) => {
                let root = root * item_arm_matrix(fp, partial);
                self.emit_item(
                    &mut out,
                    &model,
                    root,
                    light,
                    ItemDisplay::FirstPerson,
                    assets,
                );
            }
            None => {
                let steve = Identifier::minecraft("entity/player/wide/steve");
                let skin = fp.skin.as_ref().unwrap_or(&steve);
                let rect = self.texture_rect(skin, assets);
                let model = self.arm_model(fp.slim, fp.sleeve);
                // Vanilla's `ModelPart` scales its pivot by 1/16 (the model is in pixels), and
                // `AvatarRenderer.renderHand` gives the right arm its 0.1 rad z tilt.
                let m = root * arm_matrix(fp, partial) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
                emit_part_tinted(
                    &mut out,
                    &model,
                    &model.root,
                    &PoseOverrides::default(),
                    m,
                    rect,
                    light,
                    0.0,
                    [1.0; 3],
                );
            }
        }
        let to_view = inv_view.transpose();
        for v in &mut out {
            v.pos = (to_view * Vec3::from(v.pos)).into();
        }
        self.hand = out;
        if self.hand.len() > self.hand_capacity {
            self.hand_capacity = self.hand.len().next_power_of_two();
            self.hand_buffer = make_vertex_buffer(&self.device, self.hand_capacity);
        }
        if !self.hand.is_empty() {
            self.queue
                .write_buffer(&self.hand_buffer, 0, bytemuck::cast_slice(&self.hand));
        }
    }

    /// Draws the first-person arm or item (its own pass, depth cleared).
    pub fn draw_hand<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.hand.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.hand_buffer.slice(..));
        pass.draw(0..self.hand.len() as u32, 0..1);
        1
    }

    pub fn has_hand(&self) -> bool {
        !self.hand.is_empty()
    }
}
