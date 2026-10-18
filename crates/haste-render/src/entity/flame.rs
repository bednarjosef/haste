//! `EntityRenderDispatcher`'s fire feature (`FlameFeatureRenderer`): the `fire_0`/`fire_1`
//! block-atlas sprites drawn as camera-facing quads stacked up the entity's bounding box.

use super::*;

use std::sync::OnceLock;

/// Core geometry: emits the flame quads for a bounding box of `width` x `height`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_geometry(
    out: &mut Vec<EntityVertex>,
    origin: Vec3,
    width: f32,
    height: f32,
    rotation: glam::Quat,
    light: (u8, u8),
    rect0: [u32; 4],
    rect1: [u32; 4],
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let s = width * 1.4;
    let mut h = height / s;
    let int_h = h as i32;
    let zoff = 0.3 - int_h as f32 * 0.02;
    let mut yo = 0.0_f32;
    let mut r = 0.5_f32;
    let mut zo = 0.0_f32;
    let mut ss = 0_i32;
    while h > 0.0 {
        let rect = if ss % 2 == 0 { rect0 } else { rect1 };
        let (mut u0, mut u1) = (0.0_f32, 1.0_f32);
        let (v0, v1) = (0.0_f32, 1.0_f32);
        if ss / 2 % 2 == 0 {
            std::mem::swap(&mut u0, &mut u1);
        }
        let m = Mat4::from_translation(origin)
            * Mat4::from_scale(Vec3::splat(s))
            * Mat4::from_quat(rotation)
            * Mat4::from_translation(Vec3::new(0.0, 0.0, zoff));
        let nm = Mat3::from_mat4(m).inverse().transpose();
        let p = [
            Vec3::new(-r, 0.0 - yo, zo),
            Vec3::new(r, 0.0 - yo, zo),
            Vec3::new(r, 1.4 - yo, zo),
            Vec3::new(-r, 1.4 - yo, zo),
        ];
        let uv = [[u1, v1], [u0, v1], [u0, v0], [u1, v0]];
        push_world_quad(out, m, nm, p, uv, Vec3::Y, [1.0; 3], light, rect);
        h -= 0.45;
        yo -= 0.45;
        r *= 0.9;
        zo -= 0.03;
        ss += 1;
    }
}

static FIRE_0: OnceLock<Identifier> = OnceLock::new();
static FIRE_1: OnceLock<Identifier> = OnceLock::new();

fn fire_0_id() -> &'static Identifier {
    FIRE_0.get_or_init(|| Identifier::minecraft("block/fire_0"))
}
fn fire_1_id() -> &'static Identifier {
    FIRE_1.get_or_init(|| Identifier::minecraft("block/fire_1"))
}

impl EntityRenderer {
    /// `FlameFeatureRenderer`: the flame quads of a burning entity, turned to face the camera
    /// around the vertical axis only (`Mth.rotationAroundAxis(Y_AXIS, camera.orientation)`).
    pub(super) fn emit_flame(
        &mut self,
        e: &EntityInstance,
        origin: Vec3,
        orientation: glam::Quat,
        assets: &mut Assets,
    ) {
        if !e.display_fire {
            return;
        }
        let rect0 = self.texture_rect(fire_0_id(), assets);
        let rect1 = self.texture_rect(fire_1_id(), assets);
        // `LightCoordsUtil.withBlock(state.lightCoords, 15)`: sky light kept, block light forced to 15.
        let light = (15u8, e.light.0);
        let (yaw, _, _) = orientation.to_euler(glam::EulerRot::YXZ);
        let rotation = glam::Quat::from_rotation_y(yaw);
        emit_geometry(
            &mut self.vertices,
            origin,
            e.width,
            e.height,
            rotation,
            light,
            rect0,
            rect1,
        );
    }
}
