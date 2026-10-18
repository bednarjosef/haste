//! `ScreenEffectRenderer.submitWater`: the underwater texture, lit by the eye's light.
use super::draw::argb;
use super::{hud_fov, lerp, Frame};
use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;
/// Vanilla's `ScreenEffectRenderer.UNDERWATER_LOCATION`.
fn underwater() -> Identifier {
    Identifier::minecraft("textures/misc/underwater.png")
}

/// Vanilla's `Lightmap.getBrightness`: the light curve `v / (4 - 3v)` lifted towards 1 by the
/// dimension's ambient light (the overworld's is 0, so the curve itself).
pub fn light_brightness(level: u8, ambient_light: f32) -> f32 {
    let v = f32::from(level) / 15.0;
    lerp(ambient_light, v / (4.0 - 3.0 * v), 1.0)
}

impl Frame<'_> {
    /// `ScreenEffectRenderer.submitWater` over `LevelExtractor.extractPlayerState`'s overlay:
    /// the underwater texture over the whole screen, alpha 0.1 and the lightmap brightness at
    /// the eye as the colour, its uv `(u0+4, v0+4)..(u0, v0)` scrolled by the look
    /// (`uOffset = -yRot / 64`, `vOffset = xRot / 64`) and repeated (the vanilla sampler is
    /// `REPEAT`). Haste's GUI atlas is texel-fetched (no sampler), so the same REPEAT is drawn
    /// as one quad per uv period: each tile samples the whole texture once, and the tiles
    /// together cover exactly the uv window vanilla's single quad would.
    pub(super) fn water_overlay(&self, out: &mut Vec<GuiCommand>) {
        let (w, h) = (self.width as f32, self.height as f32);
        // Pixels per view unit at z = -0.5: the HUD projection maps view x to `x * h / tan(fov/2)`.
        let s = h / (hud_fov(self.effects.eye_in_water).to_radians() * 0.5).tan();
        // One uv period spans half the quad's 2 view units, i.e. `s / 2` pixels.
        let tile = s * 0.5;
        let u_off = -self.look.0 / 64.0;
        let v_off = self.look.1 / 64.0;
        let brightness = light_brightness(self.effects.eye_light, 0.0);
        let tint = argb(0.1, brightness, brightness, brightness);
        // Screen pixel where the uv `base + slope * (pixel - centre) / tile` crosses an integer.
        let edges = |size: f32, centre: f32, base: f32, slope: f32, out: &mut [f32; 24]| -> usize {
            let mut n = 1;
            out[0] = 0.0;
            let near = base + slope * (0.0 - centre) / tile;
            let far = base + slope * (size - centre) / tile;
            let (lo, hi) = (near.min(far), near.max(far));
            let mut k = lo.ceil();
            while k <= hi && n < out.len() - 1 {
                let p = centre + (k - base) * tile / slope;
                if p > 0.0 && p < size {
                    out[n] = p;
                    n += 1;
                }
                k += 1.0;
            }
            out[n] = size;
            n += 1;
            out[..n].sort_by(|a, b| a.partial_cmp(b).unwrap());
            n
        };
        let mut xs = [0.0f32; 24];
        let mut ys = [0.0f32; 24];
        // u decreases as x grows (vanilla samples u1 at the left), v increases downward.
        let nx = edges(w, w * 0.5, u_off + 2.0, -1.0, &mut xs);
        let ny = edges(h, h * 0.5, v_off + 2.0, 1.0, &mut ys);
        // The texture coordinate of a boundary: `frac_hi` keeps an exact integer at the end of
        // a period (1.0) so a full-period tile runs 1..0, `frac_lo` puts the start at 0.
        let frac_lo = |v: f32| v - v.floor();
        let frac_hi = |v: f32| v - (v.ceil() - 1.0);
        let u_at = |px: f32| u_off + 2.0 - (px - w * 0.5) / tile;
        let v_at = |py: f32| v_off + 2.0 + (py - h * 0.5) / tile;
        // All tiles are the same texture and blend, so they batch into one draw call. They are
        // prepended back to front, but never overlap, so their order among themselves is free.
        for yi in (0..ny - 1).rev() {
            for xi in (0..nx - 1).rev() {
                let (x0, x1) = (xs[xi], xs[xi + 1]);
                let (y0, y1) = (ys[yi], ys[yi + 1]);
                let u_left = frac_hi(u_at(x0));
                let u_right = frac_lo(u_at(x1));
                let v_top = frac_lo(v_at(y0));
                let v_bottom = frac_hi(v_at(y1));
                out.insert(
                    0,
                    GuiCommand::Texture {
                        texture: underwater(),
                        x: x0.round() as i32,
                        y: y0.round() as i32,
                        w: (x1 - x0).round() as i32,
                        h: (y1 - y0).round() as i32,
                        u: u_left,
                        v: v_top,
                        uw: u_right - u_left,
                        vh: v_bottom - v_top,
                        tex_w: 1,
                        tex_h: 1,
                        tint,
                        blend: GuiBlend::Normal,
                    },
                );
            }
        }
    }
}
