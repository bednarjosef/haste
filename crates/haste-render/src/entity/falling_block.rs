//! The falling block renderer (`FallingBlockRenderer`): the entity's block state drawn as
//! the block model, centred on the entity and lit at the block above its bounding box.

use super::*;

impl EntityRenderer {
    /// One falling block, following vanilla `FallingBlockRenderer.submit`: the pose stack
    /// sits at the entity's (interpolated) feet, the model is translated by
    /// (-0.5, 0, -0.5) so the unit block spans the entity's bounding box footprint. The
    /// `shouldRender` override (nothing once the world's block at the entity's position is
    /// the falling state — it has landed) arrives as [`FallingBlockDecoration::landed`],
    /// set by the client where the world is visible.
    pub(super) fn emit_falling_block(
        &mut self,
        e: &EntityInstance,
        d: &FallingBlockDecoration,
        camera: glam::DVec3,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        if d.landed {
            return;
        }
        let m = Mat4::from_translation((e.pos - camera).as_vec3())
            * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5));
        self.emit_block_model(m, d.block_state, light, false, assets);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_spans_the_entitys_bounding_box_footprint() {
        // Vanilla `submit`: translate(-0.5, 0, -0.5) on top of the entity pose, so the
        // model's [0,1]^3 spans [-0.5, 0.5] in x/z and [0, 1] in y around the entity.
        let m = Mat4::from_translation(Vec3::new(10.0, 64.0, -5.0))
            * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5));
        let corner = |x, y, z| m.transform_point3(Vec3::new(x, y, z));
        assert_eq!(corner(0.0, 0.0, 0.0), Vec3::new(9.5, 64.0, -5.5));
        assert_eq!(corner(1.0, 1.0, 1.0), Vec3::new(10.5, 65.0, -4.5));
    }
}
