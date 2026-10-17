//! Name tags in GUI space: world tags projected to the screen and their text lines.

use glam::Mat4;
use haste_assets::Assets;
use haste_core::gui::TextStyle;

use super::*;

/// A name tag in GUI space: anchor (top centre of the text) and GUI pixels per text pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenNametag {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub text: String,
    /// `!state.isDiscrete`: also draw the opaque solid pass above the see-through one.
    pub see_through: bool,
    /// The scoreboard below-name line, drawn one text line under the name.
    pub below_name: Option<String>,
}

/// Projects a world name tag to GUI space (vanilla draws tags at 0.025 blocks per text pixel).
/// None when it is behind the camera or would be smaller than a pixel.
pub fn project_nametag(
    view_proj: &Mat4,
    camera: &crate::Camera,
    tag: &crate::entity::Nametag,
    target: (u32, u32),
    gui_scale: f32,
) -> Option<ScreenNametag> {
    // Relative to the camera in f64 first, so far coordinates keep precision.
    let p = view_proj.mul_vec4(glam::Vec4::new(
        tag.anchor.x as f32,
        tag.anchor.y as f32,
        tag.anchor.z as f32,
        1.0,
    ));
    if p.w <= 0.05 {
        return None;
    }
    let ndc = p.truncate() / p.w;
    if !(-1.5..=1.5).contains(&ndc.x) || !(-1.5..=1.5).contains(&ndc.y) {
        return None;
    }
    let (w, h) = (target.0 as f32, target.1 as f32);
    let fx = (ndc.x + 1.0) * 0.5 * w / gui_scale;
    let fy = (1.0 - ndc.y) * 0.5 * h / gui_scale;
    let focal = h / (2.0 * (camera.fov_deg.to_radians() * 0.5).tan());
    let scale = 0.025 * focal / p.w / gui_scale;
    if scale * 8.0 < 1.0 / gui_scale {
        return None;
    }
    Some(ScreenNametag {
        x: fx,
        y: fy,
        scale,
        text: tag.text.clone(),
        see_through: tag.see_through,
        below_name: tag.below_name.clone(),
    })
}

impl GuiRenderer {
    /// One centred name tag line at local `(x, y)` in text pixels: the background box (when
    /// `bg` has alpha) and the text. The caller places and scales the vertices.
    pub(super) fn nametag_line(
        &mut self,
        o: [f32; 2],
        y: f32,
        text: &str,
        color: u32,
        bg: [u8; 4],
        assets: &mut Assets,
    ) {
        let w = self.font.width_f32(text);
        let x = -(w / 2.0).floor();
        if bg[3] != 0 {
            self.solid(o, [x - 1.0, y - 1.0, x + w, y + 8.0], bg, bg);
        }
        self.text(o, x, y, text, color, false, TextStyle::default(), assets);
    }
}
