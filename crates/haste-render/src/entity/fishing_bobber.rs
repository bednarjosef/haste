//! The fishing bobber (`FishingHookRenderer`): the hook's sprite as a camera-facing quad from
//! `textures/entity/fishing/fishing_hook.png`, and its line from the owner's rod hand as the
//! 16 segments `fishing_hook` computes. Vanilla draws the line as a `LINES` pass about 2.5
//! pixels wide; the entity pipeline has no line topology, so each segment is a thin
//! camera-facing quad of a fixed world width, coloured black like the vanilla line.
//!
//! The line's points come from the client (`haste_entity::fishing_bobber`), which holds the
//! owner and the math; this crate only draws what it is given.

use super::*;
use haste_core::DVec3;

/// Half the line ribbon's world width (vanilla's 2.5 pixels is about this far at a few blocks).
const LINE_HALF_WIDTH: f32 = 0.01;

/// Half the sprite's side: `FishingHookRenderer.submit` scales its unit quad by 0.5.
const SPRITE_HALF: f32 = 0.25;

/// The line's sample points: 16 segments need 17 fractions from the bobber to the hand.
pub const LINE_POINTS: usize = 17;

/// One bobber to draw: its position (world space), its block and sky light, and while its
/// owner is a player, the line's points from the bobber to the owner's rod hand, relative to
/// `pos` (`FishingHookRenderer.stringVertex` at each fraction).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FishingBobber {
    pub pos: DVec3,
    pub light: (u8, u8),
    pub line: Option<[Vec3; LINE_POINTS]>,
    /// The owner, while it is a player (the owner of the line).
    pub owner: Option<BobberOwner>,
}

/// A bobber's player owner: its entity id and the arm that holds the rod.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BobberOwner {
    pub id: haste_core::EntityId,
    pub holding_right: bool,
}

impl EntityRenderer {
    /// Whether an entity's hand holds a cast rod: `FishingRodCast.get` for the stack in that
    /// hand, true while the entity's bobber is out and the hand is its holding arm.
    pub(super) fn rod_is_cast(&self, entity: haste_core::EntityId, right: bool) -> bool {
        self.fishing_bobbers.iter().any(|b| {
            b.owner
                .is_some_and(|o| o.id == entity && o.holding_right == right)
        })
    }

    /// Every fishing bobber this frame: its sprite, then its line when an owner's hand is known.
    pub(super) fn emit_fishing_bobbers(
        &mut self,
        camera: DVec3,
        orientation: glam::Quat,
        assets: &mut Assets,
    ) {
        if self.fishing_bobbers.is_empty() {
            return;
        }
        static HOOK: std::sync::OnceLock<Identifier> = std::sync::OnceLock::new();
        static WHITE: std::sync::OnceLock<Identifier> = std::sync::OnceLock::new();
        let hook = HOOK.get_or_init(|| Identifier::minecraft("entity/fishing/fishing_hook"));
        let white = WHITE.get_or_init(|| Identifier::new("haste", "white"));
        let hook_rect = self.texture_rect(hook, assets).map(|v| v as f32);
        let white_rect = self.texture_rect(white, assets).map(|v| v as f32);
        let bobbers = std::mem::take(&mut self.fishing_bobbers);
        let mut out = std::mem::take(&mut self.vertices);
        for b in &bobbers {
            let rel = (b.pos - camera).as_vec3();
            let light = b.light;
            let corners = [
                Vec3::new(-SPRITE_HALF, -SPRITE_HALF, 0.0),
                Vec3::new(SPRITE_HALF, -SPRITE_HALF, 0.0),
                Vec3::new(SPRITE_HALF, SPRITE_HALF, 0.0),
                Vec3::new(-SPRITE_HALF, SPRITE_HALF, 0.0),
            ]
            .map(|c| rel + orientation * c);
            let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
            let uv = uv.map(|t| {
                [
                    hook_rect[0] + t[0] * hook_rect[2],
                    hook_rect[1] + t[1] * hook_rect[3],
                ]
            });
            push_quad(&mut out, corners, uv, hook_rect, [1.0; 4], light);
            if let Some(line) = b.line {
                // The ribbon's texel is the white one, so the vertex colour alone is the line.
                let mid = [white_rect[0] + 0.5, white_rect[1] + 0.5];
                for i in 0..LINE_POINTS - 1 {
                    let p0 = rel + line[i];
                    let p1 = rel + line[i + 1];
                    let side = ribbon_side(p1 - p0, (p0 + p1) * 0.5, orientation);
                    // Wound so the front face (CCW, the entity pipeline culls back faces) faces
                    // the camera: the segment crossed with the side points back at it.
                    let corners = [p0 - side, p1 - side, p1 + side, p0 + side];
                    push_quad(
                        &mut out,
                        corners,
                        [mid; 4],
                        white_rect,
                        [0.0, 0.0, 0.0, 1.0],
                        light,
                    );
                }
            }
        }
        self.vertices = out;
        self.fishing_bobbers = bobbers;
    }
}

/// The unit direction across a segment for its camera-facing ribbon: the segment crossed with
/// the view direction (the point's own position, since positions are camera-relative), scaled
/// to the line's half width. A degenerate segment falls back to the camera's right.
fn ribbon_side(dir: Vec3, view: Vec3, orientation: glam::Quat) -> Vec3 {
    let across = dir.cross(view);
    let len = across.length();
    let unit = if len > 1e-6 {
        across / len
    } else {
        orientation * Vec3::X
    };
    unit * LINE_HALF_WIDTH
}

/// Two triangles for a quad whose corners run around its outline; `uv` are per corner.
fn push_quad(
    out: &mut Vec<EntityVertex>,
    corners: [Vec3; 4],
    uv: [[f32; 2]; 4],
    rect: [f32; 4],
    color: [f32; 4],
    light: (u8, u8),
) {
    let r16 = rect.map(|v| v as u16);
    let order: [usize; 6] = [0, 1, 2, 0, 2, 3];
    for i in order {
        out.push(EntityVertex {
            pos: corners[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            light: [u16::from(light.0), u16::from(light.1), 0, 0],
        });
    }
}
