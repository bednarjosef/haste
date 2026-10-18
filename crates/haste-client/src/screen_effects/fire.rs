//! `ScreenEffectRenderer.submitFire`: the two fire quads, and which blocks set us burning.
use super::{hud_fov, Frame};
use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;
/// Vanilla's `ModelBakery.FIRE_1`: the fire sprite of the block atlas (`fire_1`).
fn fire_sprite() -> Identifier {
    Identifier::minecraft("block/fire_1")
}
/// Vanilla's `ScreenEffectRenderer.buildFireQuad` tint (`-436207617` as ARGB).
const FIRE_TINT: u32 = 0xE5FF_FFFF;

/// Whether `block` sets the player on fire when inside it: a `BaseFireBlock` (fire, soul fire)
/// or lava. Both apply the fire inside-block effect (`FIRE_IGNITE` / `LAVA_IGNITE`), which is
/// what makes the local player's `isOnFire` true on the client.
pub(super) fn is_fire_block(
    block: &Identifier,
    blocks: &haste_data::BlockStates,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> bool {
    if block.namespace() == "minecraft" && matches!(block.path(), "fire" | "soul_fire" | "lava") {
        return true;
    }
    match (tags, blocks.block_index(block)) {
        (Some(tags), Some(index)) => {
            tags.contains(&Identifier::minecraft("fire"), index.0)
                || tags.contains(&Identifier::minecraft("lava"), index.0)
        }
        _ => false,
    }
}

impl Frame<'_> {
    /// `ScreenEffectRenderer.submitFire`: two 1x1 quads at z = -0.5, translated to
    /// (±0.24, -0.3) and rotated ±10° about Y, both sampling the whole fire sprite.
    ///
    /// The level pass projects them with the HUD projection, which maps the quad at z = -0.5
    /// to the screen by a uniform `height / tan(fov / 2)` pixels per unit in x and y (a
    /// rotation about Y keeps z constant, so it only shifts and squeezes the quad in x).
    pub(super) fn fire_overlay(&self) -> [GuiCommand; 2] {
        let (w, h) = (self.width as f32, self.height as f32);
        let s = h / (hud_fov(self.effects.eye_in_water).to_radians() * 0.5).tan();
        let (cx, cy) = (w / 2.0, h / 2.0);
        let turn = (10.0f32).to_radians();
        // A corner (x, y, -0.5) lands at x * cos - 0.5 * sin shifted by ±0.24 in view units,
        // its y unchanged; screen y grows downwards, so view y inverts.
        let half = 0.5;
        let z = -0.5;
        let quad = |shift: f32, turn: f32| -> GuiCommand {
            let (cos, sin) = (turn.cos(), turn.sin());
            let left = shift + half * cos + z * sin;
            let right = shift - half * cos + z * sin;
            let x0 = (cx + left.min(right) * s).round() as i32;
            let x1 = (cx + left.max(right) * s).round() as i32;
            // `buildSpriteQuad` passes (u1, v1) at the quad's bottom-left corner and (u0, v0) at
            // the top-right, so u runs high-to-low left-to-right and v low-to-high top-to-bottom
            // (the sprite is mirrored in x only, not flipped in y).
            GuiCommand::Texture {
                texture: fire_sprite(),
                x: x0,
                y: (cy - 0.2 * s).round() as i32,
                w: x1 - x0,
                h: (1.0 * s).round() as i32,
                u: 1.0,
                v: 0.0,
                uw: -1.0,
                vh: 1.0,
                tex_w: 1,
                tex_h: 1,
                tint: FIRE_TINT,
                blend: GuiBlend::Normal,
            }
        };
        // `submitFire`: the quad shifted right turns left (-10°) and the one shifted left
        // turns right (+10°), so the pair is mirrored about the screen centre.
        [quad(0.24, -turn), quad(-0.24, turn)]
    }
}
