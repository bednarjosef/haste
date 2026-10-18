//! `Hud.extractSpyglassOverlay`: the scope and the black around it.
use super::draw::fill;
use super::Frame;
use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;
/// Vanilla's `Hud.SPYGLASS_SCOPE_LOCATION`.
fn spyglass_scope() -> Identifier {
    Identifier::minecraft("textures/misc/spyglass_scope.png")
}

/// The spyglass item's name (`Player.isScoping`: `getUseItem().is(Items.SPYGLASS)`).
pub fn is_spyglass(item: Option<&Identifier>) -> bool {
    item.is_some_and(|i| i.path() == "spyglass")
}

impl Frame<'_> {
    /// `Hud.extractSpyglassOverlay`: the scope texture over a centred square, black around it.
    pub(super) fn spyglass_overlay(&self) -> [GuiCommand; 5] {
        let (w, h) = (self.width as f32, self.height as f32);
        let src = w.min(h);
        // `min(w / src, h / src)` is 1 on any window (src is the smaller side), so the ratio is
        // the scope scale itself.
        let ratio = (w / src).min(h / src) * self.scope_scale;
        let size = (src * ratio).floor() as i32;
        let left = (self.width as i32 - size) / 2;
        let top = (self.height as i32 - size) / 2;
        let scope = GuiCommand::Texture {
            texture: spyglass_scope(),
            x: left,
            y: top,
            w: size,
            h: size,
            u: 0.0,
            v: 0.0,
            uw: 1.0,
            vh: 1.0,
            tex_w: 1,
            tex_h: 1,
            tint: 0xFFFF_FFFF,
            blend: GuiBlend::Normal,
        };
        let (w, h) = (self.width as i32, self.height as i32);
        let (right, bottom) = (left + size, top + size);
        [
            scope,
            fill(0, bottom, w, h, BLACK),
            fill(0, 0, w, top, BLACK),
            fill(0, top, left, bottom, BLACK),
            fill(right, top, w, bottom, BLACK),
        ]
    }
}

/// Vanilla's spyglass black (`fill(..., -16777216)`).
const BLACK: u32 = 0xFF00_0000;
