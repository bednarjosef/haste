//! The display renderer (`DisplayRenderer`): what the block, item and text displays share —
//! the distance and culling-box checks, the brightness override, and the pose (the billboard
//! orientation, then the interpolated transformation) each subclass draws its content under.
//! One module per subclass: `BlockDisplayRenderer`, `ItemDisplayRenderer` and
//! `TextDisplayRenderer`.

mod block;
mod item;
#[cfg(test)]
mod tests;
mod text;
mod text_pass;

use haste_core::display::{Billboard, DisplayContent, DisplayDecoration};
pub(super) use text_pass::TextPass;

use super::*;

/// The display renderers' state kept across frames: the camera rotation the billboards
/// follow, the text passes and their line caches, and the text displays waiting for the
/// back-to-front emission. Everything is reused; a frame allocates nothing once warm.
pub(super) struct Displays {
    /// `Camera.yRot()` / `xRot()` this frame (degrees).
    camera_rot: (f32, f32),
    pub(super) text: text_pass::TextPass,
    layouts: text::LayoutCache,
    /// (squared distance, instance index) of this frame's text displays.
    pending_text: Vec<(f64, usize)>,
    /// The atlas key of the white texel text backgrounds and effects use.
    pub(super) white_key: Identifier,
}

impl Displays {
    pub(super) fn new(text: text_pass::TextPass) -> Displays {
        Displays {
            camera_rot: (0.0, 0.0),
            text,
            layouts: text::LayoutCache::default(),
            pending_text: Vec::new(),
            white_key: Identifier::new("haste", "white"),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending_text.clear();
        self.text.clear();
    }
}

/// `DisplayRenderer.calculateOrientation`: the rotation the content turns by before its
/// transformation, `rotationYXZ(-yRot, xRot, 0)` (degrees here) with the camera's rotation
/// standing in for the entity's on the axes the billboard frees (`transformYRot`:
/// `cameraYRot - 180`; `transformXRot`: `-cameraXRot`).
pub fn orientation(
    billboard: Billboard,
    entity_rot: (f32, f32),
    camera_rot: (f32, f32),
) -> glam::Quat {
    let (entity_y, entity_x) = entity_rot;
    let (camera_y, camera_x) = (camera_rot.0 - 180.0, -camera_rot.1);
    let (y, x) = match billboard {
        Billboard::Fixed => (entity_y, entity_x),
        Billboard::Horizontal => (entity_y, camera_x),
        Billboard::Vertical => (camera_y, entity_x),
        Billboard::Center => (camera_y, camera_x),
    };
    glam::Quat::from_euler(glam::EulerRot::YXZ, -y.to_radians(), x.to_radians(), 0.0)
}

/// `EntityRenderer.shouldRender` for a display: `Display.shouldRenderAtSqrDistance` (within
/// `viewRange * 64 * viewScale` blocks), then, when the culling box has a size
/// (`affectedByCulling`), that box inflated by 0.5 against the frustum. `rel` is the entity's
/// position relative to the camera.
pub(super) fn should_render(
    d: &DisplayDecoration,
    rel: glam::DVec3,
    dist2: f64,
    frustum: &[glam::Vec4; 6],
    view_scale: f32,
) -> bool {
    let range = f64::from(d.view_range) * 64.0 * f64::from(view_scale);
    if dist2 >= range * range {
        return false;
    }
    if d.width == 0.0 || d.height == 0.0 {
        return true;
    }
    // `Display.updateCulling`: the box spans `width` around the entity and `height` up.
    let half = d.width / 2.0;
    let rel = rel.as_vec3();
    let min = rel + Vec3::new(-half, 0.0, -half) - 0.5;
    let max = rel + Vec3::new(half, d.height, half) + 0.5;
    should_render::box_in_frustum(frustum, min, max)
}

impl EntityRenderer {
    /// The camera rotation billboards follow (`Camera.yRot()` / `xRot()`, degrees), set once
    /// per frame before `prepare`.
    pub fn set_camera_rotation(&mut self, y_rot: f32, x_rot: f32) {
        self.displays.camera_rot = (y_rot, x_rot);
    }

    /// The pose a display's content is drawn under (`DisplayRenderer.submit`): at the entity
    /// (camera-relative), turned by the billboard orientation, then the interpolated
    /// transformation.
    fn display_pose(&self, e: &EntityInstance, d: &DisplayDecoration, camera: glam::DVec3) -> Mat4 {
        let rotation = orientation(
            d.billboard,
            (d.entity_y_rot, d.entity_x_rot),
            self.displays.camera_rot,
        );
        Mat4::from_translation((e.pos - camera).as_vec3())
            * Mat4::from_quat(rotation)
            * d.transformation
    }

    /// One display (`DisplayRenderer.submit` -> `submitInner`). `light` is the light sampled
    /// at the entity (block, sky); the brightness override replaces it
    /// (`getBlockLightLevel` / `getSkyLightLevel`). Text displays are queued and drawn back to
    /// front after every other entity (`emit_text_displays`), like vanilla's translucent text.
    pub(super) fn emit_display(
        &mut self,
        index: usize,
        e: &EntityInstance,
        d: &DisplayDecoration,
        camera: glam::DVec3,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let light = d.brightness.unwrap_or(light);
        match &d.content {
            DisplayContent::Block {
                state,
                light_emission,
            } => {
                let pose = self.display_pose(e, d, camera);
                self.emit_block_display(pose, *state, *light_emission, light, assets);
            }
            DisplayContent::Item { item, context } => {
                let pose = self.display_pose(e, d, camera);
                self.emit_item_display(pose, *item, *context, light, assets);
            }
            DisplayContent::Text(_) => {
                let dist2 = (e.pos - camera).length_squared();
                self.displays.pending_text.push((dist2, index));
            }
        }
    }

    /// The queued text displays, farthest first (vanilla sorts its translucent text
    /// submissions by distance), into the text passes.
    pub(super) fn emit_text_displays(&mut self, entities: &[EntityInstance], camera: glam::DVec3) {
        let mut pending = std::mem::take(&mut self.displays.pending_text);
        pending.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        for &(_, index) in &pending {
            let e = &entities[index];
            let Some(Decoration::Display(d)) = &e.decoration else {
                continue;
            };
            let DisplayContent::Text(t) = &d.content else {
                continue;
            };
            let (sky, block) = e.light;
            let light = d.brightness.unwrap_or((block, sky));
            let pose = self.display_pose(e, d, camera);
            self.emit_text_display(e.id, pose, t, light);
        }
        self.displays.pending_text = pending;
        self.displays.layouts.end_frame();
        self.displays.text.upload(&self.device, &self.queue);
    }
}
