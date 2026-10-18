//! `Hud.extractConfusionOverlay`: the nausea texture over the screen.
use super::draw::argb;
use super::{lerp, Frame};
use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;
/// Vanilla's `Hud.NAUSEA_LOCATION`.
fn confusion() -> Identifier {
    Identifier::minecraft("textures/misc/nausea.png")
}

impl Frame<'_> {
    /// `Hud.extractConfusionOverlay`: the nausea texture zoomed about the screen centre by
    /// `lerp(strength, 2, 1)` and added to the screen in a green tint.
    pub(super) fn confusion_overlay(&self, strength: f32) -> GuiCommand {
        let size = lerp(strength, 2.0, 1.0);
        let tint = argb(1.0, 0.2 * strength, 0.4 * strength, 0.2 * strength);
        // The pose scales about the centre by `size` and blits the whole texture over the
        // scaled quad, so the screen sees the middle `1 / size` of the texture: the uv window
        // is `1 / size` wide, centred (from `(1 - 1 / size) / 2`).
        let span = 1.0 / size;
        let uv = (1.0 - span) / 2.0;
        GuiCommand::Texture {
            texture: confusion(),
            x: 0,
            y: 0,
            w: self.width as i32,
            h: self.height as i32,
            u: uv,
            v: uv,
            uw: span,
            vh: span,
            tex_w: 1,
            tex_h: 1,
            tint,
            blend: GuiBlend::Additive,
        }
    }
}
