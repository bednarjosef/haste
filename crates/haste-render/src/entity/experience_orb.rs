//! The experience orb renderer (`ExperienceOrbRenderer`): a camera-facing quad with a
//! value-tiered icon from `textures/entity/experience/experience_orb.png`, a colour cycle
//! from the orb's age, full-bright-ish light (block light + 7) and translucent blending.

use super::*;

/// Icon tier from `ExperienceOrb.getIcon`'s value thresholds.
pub fn experience_orb_icon(value: i32) -> i32 {
    if value >= 2477 {
        10
    } else if value >= 1237 {
        9
    } else if value >= 617 {
        8
    } else if value >= 307 {
        7
    } else if value >= 149 {
        6
    } else if value >= 73 {
        5
    } else if value >= 37 {
        4
    } else if value >= 17 {
        3
    } else if value >= 7 {
        2
    } else if value >= 3 {
        1
    } else {
        0
    }
}

/// Colour cycle `(red, green, blue)` as byte values (0..=255) for a given age in ticks.
pub fn experience_orb_color(age: f32) -> (u8, u8, u8) {
    let rr = age / 2.0;
    let rc = ((rr.sin() + 1.0) * 0.5 * 255.0) as u8;
    let gc: u8 = 255;
    let bc = (((rr + std::f32::consts::PI * 4.0 / 3.0).sin() + 1.0) * 0.1 * 255.0) as u8;
    (rc, gc, bc)
}

impl EntityRenderer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_experience_orb(
        &mut self,
        e: &EntityInstance,
        value: i32,
        age: f32,
        camera: glam::DVec3,
        light: (u8, u8),
        orientation: glam::Quat,
        assets: &mut Assets,
    ) {
        let icon = experience_orb_icon(value);
        let texture = Identifier::minecraft("entity/experience/experience_orb");
        let rect = self.texture_rect(&texture, assets);
        let (rc, _gc, bc) = experience_orb_color(age);
        let u0 = (icon % 4 * 16) as f32 / 64.0;
        let u1 = u0 + 16.0 / 64.0;
        let v0 = (icon / 4 * 16) as f32 / 64.0;
        let v1 = v0 + 16.0 / 64.0;
        // Vanilla: block light + 7, clamped.
        let block = (light.0 as i32 + 7).clamp(0, 15) as u8;
        let light = (block, light.1);
        let center = (e.pos - camera).as_vec3() + Vec3::new(0.0, 0.1, 0.0);
        let s = 0.3_f32;
        let corners: [(f32, f32); 4] = [(-0.5, -0.25), (0.5, -0.25), (0.5, 0.75), (-0.5, 0.75)];
        let uvs: [(f32, f32); 4] = [(u0, v1), (u1, v1), (u1, v0), (u0, v0)];
        let mut pts = [Vec3::ZERO; 4];
        for i in 0..4 {
            let local = Vec3::new(corners[i].0, corners[i].1, 0.0) * s;
            pts[i] = center + orientation * local;
        }
        let uv = uvs.map(|t| {
            [
                rect[0] as f32 + t.0 * rect[2] as f32,
                rect[1] as f32 + t.1 * rect[3] as f32,
            ]
        });
        let r16 = rect.map(|v| v as u16);
        let color = [rc as f32 / 255.0, 1.0, bc as f32 / 255.0, 128.0 / 255.0];
        let order: [usize; 6] = [0, 1, 2, 0, 2, 3];
        for i in order {
            self.vertices.push(EntityVertex {
                pos: pts[i].to_array(),
                uv: uv[i],
                color,
                rect: r16,
                light: [light.0 as u16, light.1 as u16, 0, 0],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_tiers() {
        assert_eq!(experience_orb_icon(0), 0);
        assert_eq!(experience_orb_icon(2), 0);
        assert_eq!(experience_orb_icon(3), 1);
        assert_eq!(experience_orb_icon(7), 2);
        assert_eq!(experience_orb_icon(17), 3);
        assert_eq!(experience_orb_icon(37), 4);
        assert_eq!(experience_orb_icon(73), 5);
        assert_eq!(experience_orb_icon(149), 6);
        assert_eq!(experience_orb_icon(307), 7);
        assert_eq!(experience_orb_icon(617), 8);
        assert_eq!(experience_orb_icon(1237), 9);
        assert_eq!(experience_orb_icon(2477), 10);
    }

    #[test]
    fn colour_at_two_ages() {
        let a = experience_orb_color(0.0);
        let b = experience_orb_color(10.0);
        // red cycles: age 0 -> (sin0+1)/2 = 0.5 -> 127
        assert_eq!(a.0, 127);
        assert_eq!(a.2, 3);
        // Just ensure differs
        assert_ne!(a, b);
    }
}
