//! Shared GUI command helpers of the overlays (fills, full-screen textures, ARGB).
use super::Frame;
use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;
/// A rectangle in GUI pixels, filled with `tint` (vanilla's `ARGB`).
pub(super) fn fill(x1: i32, y1: i32, x2: i32, y2: i32, tint: u32) -> GuiCommand {
    GuiCommand::Fill {
        x1,
        y1,
        x2,
        y2,
        color: tint,
    }
}

/// A texture over the whole screen, stretched, with `tint` (`Hud.extractTextureOverlay`).
pub(super) fn full_screen_texture(
    texture: Identifier,
    width: u32,
    height: u32,
    tint: u32,
    blend: GuiBlend,
) -> GuiCommand {
    GuiCommand::Texture {
        texture,
        x: 0,
        y: 0,
        w: width as i32,
        h: height as i32,
        u: 0.0,
        v: 0.0,
        uw: 1.0,
        vh: 1.0,
        tex_w: 1,
        tex_h: 1,
        tint,
        blend,
    }
}

impl Frame<'_> {
    /// A texture over the whole screen, stretched, with `tint` (`Hud.extractTextureOverlay`).
    pub(super) fn full_screen(
        &self,
        texture: Identifier,
        tint: u32,
        blend: GuiBlend,
    ) -> GuiCommand {
        full_screen_texture(texture, self.width, self.height, tint, blend)
    }
}

/// Vanilla's `ARGB.colorFromFloat` (each channel `floor(value * 255)`), packed like vanilla.
pub(super) fn argb(alpha: f32, red: f32, green: f32, blue: f32) -> u32 {
    let c = |v: f32| ((v * 255.0).floor() as u32).min(255);
    c(alpha) << 24 | c(red) << 16 | c(green) << 8 | c(blue)
}

/// Vanilla's `ARGB.white(alpha)`.
pub(super) fn white(alpha: f32) -> u32 {
    argb(alpha, 1.0, 1.0, 1.0)
}
