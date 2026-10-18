//! Item display transforms: vanilla's `ItemDisplayContext` matrices (held, frame, painting,
//! head) and the model-space bounding box through a display transform.

use super::*;

/// An ARGB colour (armour dye / undyed fallback) as a linear-RGB vertex multiplier, the same
/// conversion the GUI item tints use (`sky::linear`).
pub(crate) fn argb_linear(c: u32) -> [f32; 3] {
    let f = |s: u32| crate::sky::linear([((c >> s) & 0xFF) as f32 / 255.0; 3])[0];
    [f(16), f(8), f(0)]
}

/// A held item's model matrix: its `display.<context>` transform and composite
/// `transformation`, then the recentring vanilla's `ItemTransform.apply` ends with
/// (the model's unit cube is drawn around the origin). `left` applies vanilla's left-hand fix
/// (`ItemTransform.apply(true, ...)`: translation.x and rotation.y/z negated).
pub(crate) fn item_display_matrix(
    part: &haste_assets::ItemModelPart,
    display: &haste_assets::DisplayTransform,
    left: bool,
) -> Mat4 {
    let display = if left {
        haste_assets::DisplayTransform {
            rotation: [
                display.rotation[0],
                -display.rotation[1],
                -display.rotation[2],
            ],
            translation: [
                -display.translation[0],
                display.translation[1],
                display.translation[2],
            ],
            scale: display.scale,
        }
    } else {
        *display
    };
    crate::gui::display_matrix(&display)
        * crate::gui::part_matrix(&part.transformation)
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// `ItemFrameRenderer.submit`'s frame transform: translate to `direction * 0.46875`, then the
/// rotations that turn the block model to face `direction` (horizontal: yaw `180 - toYRot`;
/// vertical: pitch `-90 * axisStep` then yaw 180).
pub(crate) fn item_frame_transform(facing: haste_core::Direction) -> Mat4 {
    let (fx, fy, fz) = (
        facing.step_x() as f32,
        facing.step_y() as f32,
        facing.step_z() as f32,
    );
    let (x_rot, y_rot) = if facing.is_horizontal() {
        (0.0, 180.0 - facing.to_y_rot())
    } else {
        (-90.0 * facing.axis_direction_step() as f32, 180.0)
    };
    Mat4::from_translation(Vec3::new(fx, fy, fz) * 0.46875)
        * Mat4::from_rotation_x(x_rot.to_radians())
        * Mat4::from_rotation_y(y_rot.to_radians())
}

/// `ItemFrameRenderer.submit`'s item transform: forward to the frame's face, `rotation * 45`
/// degrees around Z, then scale 0.5.
pub(crate) fn item_frame_item_transform(rotation: u8) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, 0.0, 0.4375))
        * Mat4::from_rotation_z((rotation as f32 * 45.0).to_radians())
        * Mat4::from_scale(Vec3::splat(0.5))
}

/// `PaintingRenderer.submit`'s rotation: yaw `180 - direction.get2DDataValue() * 90`.
pub(crate) fn painting_transform(facing: haste_core::Direction) -> Mat4 {
    Mat4::from_rotation_y((180.0 - facing.to_2d_data_value() as f32 * 90.0).to_radians())
}

/// The model's bounding box in the space `emit_item` draws it through the given display
/// transform (block units, after the -0.5 centring `ItemTransform.apply` ends with): the
/// transformed element or extruded-sprite positions. Vanilla's
/// `ItemStackRenderState.getModelBoundingBox` (which drives `ItemEntityRenderer.submit`'s
/// lift, the stack-copy layout and `ShelfRenderer`'s vertical offset) computes exactly this;
/// a model with no geometry gets vanilla's empty box (a point at the origin).
pub(super) fn item_model_bbox(
    model: &haste_assets::ItemModel,
    display: &ItemDisplay,
) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    let mut see = |p: Vec3| {
        min = min.min(p);
        max = max.max(p);
        any = true;
    };
    for part in &model.parts {
        // The composition `item_display_matrix` applies: the display transform, the model's
        // composite transformation, then the -0.5 recentring (no left-hand fix here: block
        // entities never draw an item in the left hand).
        let display = match display {
            ItemDisplay::FirstPerson => &part.firstperson,
            ItemDisplay::ThirdPersonRight | ItemDisplay::ThirdPersonLeft => &part.thirdperson,
            ItemDisplay::Head => &part.head,
            ItemDisplay::Fixed => &part.fixed,
            ItemDisplay::Ground => &part.ground,
            ItemDisplay::OnShelf => &part.on_shelf,
        };
        let m = crate::gui::display_matrix(display)
            * crate::gui::part_matrix(&part.transformation)
            * Mat4::from_translation(Vec3::splat(-0.5));
        match &part.geometry {
            haste_assets::ItemGeometry::None => {}
            haste_assets::ItemGeometry::Generated { layers } => {
                for layer in layers {
                    for q in &layer.quads {
                        for p in q.positions {
                            see(m.transform_point3(Vec3::from(p)));
                        }
                    }
                }
            }
            haste_assets::ItemGeometry::Elements(quads) => {
                for q in quads {
                    for p in q.positions {
                        see(m.transform_point3(Vec3::from(p)));
                    }
                }
            }
        }
    }
    if any {
        (min, max)
    } else {
        (Vec3::ZERO, Vec3::ZERO)
    }
}
