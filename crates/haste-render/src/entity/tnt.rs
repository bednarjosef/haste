//! The primed TNT renderer (`TntRenderer`): the TNT block model scaled up over the last
//! ten fuse ticks, with the white flash overlay every other five-tick band.

use super::*;

/// `TntRenderer.getSwellAmount`: `(1 - fuse/10)` clamped to 0..1, to the fourth power,
/// times 0.3 — the extra scale the block swells by in its last ten ticks.
pub(crate) fn swell_amount(fuse: f32) -> f32 {
    let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
    g * g * g * g * 0.3
}

/// `TntRenderer.isLit`: the flash is on in the even five-tick bands of the fuse (the
/// `(int)(fuse / 5) % 2 == 0` of vanilla), off once the fuse has run out.
pub(crate) fn flash_lit(fuse: f32) -> bool {
    if fuse < 0.0 {
        return false;
    }
    (fuse / 5.0) as i32 % 2 == 0
}

impl EntityRenderer {
    /// One primed TNT, following vanilla `TntRenderer.submit`: lift by half a block, swell
    /// in the last ten ticks, then the -90/+90 degree Y rotation pair around the
    /// (-0.5, -0.5, 0.5) translation — conjugating a translation by rotations about the
    /// same axis, the pair nets out to a pure translation of (-0.5, -0.5, -0.5), so the
    /// block stays axis-aligned and centred on the entity. `getSwellAmount` scales about
    /// the entity's mid-height. The flash overlay follows `isLit` on the same
    /// `fuseRemainingInTicks` the state carries.
    pub(super) fn emit_primed_tnt(
        &mut self,
        e: &EntityInstance,
        d: &PrimedTntDecoration,
        camera: glam::DVec3,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let scale = if d.fuse < 10.0 {
            1.0 + swell_amount(d.fuse)
        } else {
            1.0
        };
        let m = Mat4::from_translation((e.pos - camera).as_vec3())
            * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0))
            * Mat4::from_scale(Vec3::splat(scale))
            * Mat4::from_translation(Vec3::new(-0.5, -0.5, -0.5));
        self.emit_block_model(m, d.block_state, light, flash_lit(d.fuse), assets);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length_squared() < 1e-10
    }

    #[test]
    fn swell_grows_to_1_3_over_the_last_ten_ticks() {
        // Ten or more ticks left: no swell at all.
        assert_eq!(swell_amount(10.0), 0.0);
        assert_eq!(swell_amount(80.0), 0.0);
        // Halfway: (1 - 0.5)^4 * 0.3.
        assert!((swell_amount(5.0) - 0.3 * 0.0625).abs() < 1e-6);
        // The last moment before detonation swells the full 30 %.
        assert!((swell_amount(0.0) - 0.3).abs() < 1e-6);
        // Past the end stays clamped.
        assert!((swell_amount(-3.0) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn the_flash_bands_every_five_ticks() {
        // fuse = getFuse() - partial + 1: a fresh 80-tick TNT starts at 81. The bands are
        // `(int)(fuse / 5) % 2`: 81 sits in 16 (lit), 76 in 15 (dark), 74 and 71 in 14
        // (lit), 66 in 13 (dark).
        assert!(flash_lit(81.0));
        assert!(!flash_lit(76.0));
        assert!(flash_lit(74.0));
        assert!(flash_lit(71.0));
        assert!(!flash_lit(66.0));
        // The last band before the detonation stays lit.
        assert!(flash_lit(1.0));
        // Past the end of the fuse the flash is off.
        assert!(!flash_lit(-0.5));
    }

    #[test]
    fn the_rotation_pair_is_a_pure_centring_translation() {
        // Vanilla's `rotateDegrees(YP, -90)`, `translate(-0.5, -0.5, 0.5)`,
        // `rotateDegrees(YP, 90)`: the rotations conjugate the translation into
        // (-0.5, -0.5, -0.5), leaving the block axis-aligned and spanning the entity's
        // bounding box footprint at scale 1.
        let rotations = Mat4::from_rotation_y((-90.0f32).to_radians())
            * Mat4::from_translation(Vec3::new(-0.5, -0.5, 0.5))
            * Mat4::from_rotation_y(90.0f32.to_radians());
        let expected = Mat4::from_translation(Vec3::new(-0.5, -0.5, -0.5));
        for p in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        ] {
            assert!(close(
                rotations.transform_point3(p),
                expected.transform_point3(p)
            ));
        }
    }

    #[test]
    fn the_swell_scales_about_the_entitys_mid_height() {
        let m = Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0))
            * Mat4::from_scale(Vec3::splat(1.3))
            * Mat4::from_translation(Vec3::new(-0.5, -0.5, -0.5));
        // The block's centre (0.5, 0.5, 0.5) stays at the entity's mid-height point.
        assert!(close(
            m.transform_point3(Vec3::new(0.5, 0.5, 0.5)),
            Vec3::new(0.0, 0.5, 0.0)
        ));
        // Its corners reach 0.65 out from the centre: 1.3 * 0.5.
        assert!(close(
            m.transform_point3(Vec3::new(0.0, 0.0, 0.0)),
            Vec3::new(-0.65, -0.15, -0.65)
        ));
    }
}
