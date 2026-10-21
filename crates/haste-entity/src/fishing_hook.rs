//! Fishing bobbers (`minecraft:fishing_bobber`): the pure parts of vanilla's
//! `FishingHookRenderer` — the fishing line's 16-segment curve, the hand the line leaves from
//! (third person and first person), the arm a rod is held in, the swing factor that lifts the
//! hand, and the rod's `fishing_rod/cast` item-model choice (`FishingRodCast.get`).
//!
//! The renderer (`haste-render`) draws the bobber sprite and the line from these functions;
//! nothing here touches the GPU.

use haste_core::{DVec3, Vec3};

/// The line is drawn as this many segments (`FishingHookRenderer.submit`'s `steps`).
pub const STRING_SEGMENTS: u32 = 16;

/// `FishingHookRenderer.fraction`: the `i`-th of `steps` even fractions in `[0, 1]`.
pub fn fraction(i: u32, steps: u32) -> f32 {
    i as f32 / steps as f32
}

/// `FishingHookRenderer.getHoldingArm`: a fishing rod in the main hand is held in the main
/// arm; anything else makes the hook's owner cast with the off arm. `main_is_right` is the
/// owner's main arm (`Player.getMainArm` is right-handed for `true`).
pub fn holding_arm_is_right(main_is_right: bool, main_hand_is_rod: bool) -> bool {
    if main_hand_is_rod {
        main_is_right
    } else {
        !main_is_right
    }
}

/// `FishingRodCast.get`: the rod's `fishing_rod/cast` model applies while its owner player
/// has a hook out and the item in the arm the rod is held in is this very stack.
pub fn cast_model_selected(owner_has_hook: bool, held_by_arm_is_this_stack: bool) -> bool {
    owner_has_hook && held_by_arm_is_this_stack
}

/// `Window.getAppropriateLineWidth`: 2.5 pixels, growing with the window past 1920 wide.
pub fn appropriate_line_width(window_width: u32) -> f32 {
    f32::max(2.5, window_width as f32 / 1920.0 * 2.5)
}

/// The swing factor `FishingHookRenderer.extractRenderState` takes from the owner's swing
/// animation: `sin(sqrt(swing) * pi)`, which lifts the hand while the rod is cast.
pub fn swing_factor(swing: f32) -> f32 {
    (swing.sqrt() * std::f32::consts::PI).sin()
}

/// The owner's hand, third person (`FishingHookRenderer.getPlayerHandPos`, the else branch):
/// the eye position moved by the body yaw's right and forward offsets, scaled by the owner's
/// scale and dropped by the crouch and the arm's height. `body_yaw` is the lerped
/// `yBodyRot` in degrees; `main_is_right` decides which side the rod hand is on.
pub fn hand_pos_third_person(
    eye: DVec3,
    body_yaw: f32,
    player_scale: f32,
    right_hand: bool,
    crouching: bool,
) -> DVec3 {
    let invert = if right_hand { 1.0 } else { -1.0 };
    let yaw = body_yaw * (std::f32::consts::PI / 180.0);
    let sin = f64::from(yaw.sin());
    let cos = f64::from(yaw.cos());
    let scale = f64::from(player_scale);
    let right_offset = invert * 0.35 * scale;
    let forward_offset = 0.8 * scale;
    let y_offset = if crouching { -0.1875 } else { 0.0 };
    eye + DVec3::new(
        -cos * right_offset - sin * forward_offset,
        y_offset - 0.45 * scale,
        -sin * right_offset + cos * forward_offset,
    )
}

/// The camera's near plane, as `Camera.getNearPlane` builds it: the view's forward, up and
/// left at `z_near`, the up and left spans sized from the field of view (degrees) and aspect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearPlane {
    pub forward: DVec3,
    pub up: DVec3,
    pub left: DVec3,
}

impl NearPlane {
    /// `Camera.getNearPlane(fov)`. The vectors are the camera's unit forward, up and left.
    pub fn new(forward: DVec3, up: DVec3, left: DVec3, fov: f32, aspect: f64, z_near: f64) -> Self {
        let plane_height = f64::from(fov * (std::f32::consts::PI / 180.0) / 2.0).tan() * z_near;
        let plane_width = plane_height * aspect;
        Self {
            forward: forward * z_near,
            up: up * plane_height,
            left: left * plane_width,
        }
    }

    /// `Camera.NearPlane.getPointOnPlane`: forward + up·y − left·x.
    pub fn point_on_plane(&self, x: f32, y: f32) -> DVec3 {
        self.forward + self.up * f64::from(y) - self.left * f64::from(x)
    }
}

/// `Vec3.yRot` then `Vec3.xRot` in vanilla's order, as the first-person hand applies them.
fn y_rot(v: DVec3, radians: f32) -> DVec3 {
    let cos = f64::from(radians.cos());
    let sin = f64::from(radians.sin());
    DVec3::new(v.x * cos + v.z * sin, v.y, v.z * cos - v.x * sin)
}

fn x_rot(v: DVec3, radians: f32) -> DVec3 {
    let cos = f64::from(radians.cos());
    let sin = f64::from(radians.sin());
    DVec3::new(v.x, v.y * cos + v.z * sin, v.z * cos - v.y * sin)
}

/// The owner's hand, first person (`FishingHookRenderer.getPlayerHandPos`, the first branch):
/// a point on the near plane (the rod side picked by `invert`), pushed out by the view-bobbing
/// scale `960 / fov` and swung by the swing factor from the eye. `invert` is +1 for the right
/// arm and -1 for the left.
pub fn hand_pos_first_person(
    eye: DVec3,
    near: &NearPlane,
    fov: f32,
    right_hand: bool,
    swing: f32,
) -> DVec3 {
    let invert = if right_hand { 1.0 } else { -1.0 };
    let bobbing = VIEW_BOBBING_SCALE / f64::from(fov);
    let view = near.point_on_plane(invert * 0.525, -0.1) * bobbing;
    let view = y_rot(view, swing * 0.5);
    let view = x_rot(view, -swing * 0.7);
    eye + view
}

/// `FishingHookRenderer.VIEW_BOBBING_SCALE`.
pub const VIEW_BOBBING_SCALE: f64 = 960.0;

/// The line's origin relative to the bobber (`FishingHookRenderer.extractRenderState`): the
/// hand position minus the hook's interpolated position lifted by 0.25. A bobber without an
/// owner has no line (`Vec3.ZERO`).
pub fn line_origin_offset(hand: Option<DVec3>, hook_pos: DVec3) -> DVec3 {
    match hand {
        Some(hand) => hand - (hook_pos + DVec3::new(0.0, 0.25, 0.0)),
        None => DVec3::ZERO,
    }
}

/// `FishingHookRenderer.stringVertex`'s position for the fraction `aa` along the line from the
/// bobber (origin `(0, 0.25, 0)`): x and z scale linearly, y follows the sagging parabola
/// `y = ya·(aa² + aa)/2 + 0.25`.
pub fn string_point(line: DVec3, aa: f32) -> Vec3 {
    let aa = f64::from(aa);
    Vec3::new(
        (line.x * aa) as f32,
        (line.y * (aa * aa + aa) * 0.5 + 0.25) as f32,
        (line.z * aa) as f32,
    )
}

/// `FishingHookRenderer.stringVertex`: the line vertex at `aa` and its normal, the unit
/// direction towards the vertex at `nexta` (zero for a degenerate segment, never NaN).
pub fn string_vertex(line: DVec3, aa: f32, nexta: f32) -> (Vec3, Vec3) {
    let p = string_point(line, aa);
    let q = string_point(line, nexta);
    let d = q - p;
    let len = d.length();
    let normal = if len > 0.0 { d / len } else { Vec3::ZERO };
    (p, normal)
}

/// The line's vertex pairs in vanilla's submit order: for segment `i`, the vertex at `a0`
/// towards `a1`, then the vertex at `a1` towards `a0`, so each segment is two line endpoints.
pub fn string_segment_vertices(line: DVec3, i: u32) -> [(Vec3, Vec3); 2] {
    let a0 = fraction(i, STRING_SEGMENTS);
    let a1 = fraction(i + 1, STRING_SEGMENTS);
    [string_vertex(line, a0, a1), string_vertex(line, a1, a0)]
}
