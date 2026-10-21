//! `Hud.extractVignette`: the vignette texture over the screen, tinted toward red as the world
//! border's warning rises.
use super::draw::argb;
use super::Frame;
use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;

/// Vanilla's `Hud.VIGNETTE_LOCATION`.
fn vignette() -> Identifier {
    Identifier::minecraft("textures/misc/vignette.png")
}

/// Vanilla's `Options.vignetteBrightness` default (`Hud.vignetteBrightness`, 1.0).
const VIGNETTE_BRIGHTNESS: f32 = 1.0;

impl Frame<'_> {
    /// `Hud.extractVignette`: the vignette multiplied over the screen (`VIGNETTE` blends
    /// `dst * (1 - src)`). At `warning` 0 the tint is the brightness's grey; as it rises the
    /// red channel falls and green and blue stay lit, so the edges turn red.
    pub(super) fn vignette_overlay(&self, warning: f32) -> GuiCommand {
        let strength = warning.clamp(0.0, 1.0);
        let red = VIGNETTE_BRIGHTNESS * (1.0 - strength);
        let green_blue = VIGNETTE_BRIGHTNESS + (1.0 - VIGNETTE_BRIGHTNESS) * strength;
        self.full_screen(
            vignette(),
            argb(1.0, red, green_blue, green_blue),
            GuiBlend::Multiply,
        )
    }
}
