//! Sine/cosine as the game computes them for movement: a 65536-entry f32 table, index from the
//! angle in radians (mcpk.wiki "Angles": 65536 significant angles, cos = sin shifted by a quarter
//! turn). Assumption pending traces; switch `USE_TABLE` off to compare with exact f32 trig.
use std::sync::OnceLock;

const USE_TABLE: bool = true;
const SIZE: usize = 65536;

fn table() -> &'static [f32] {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        (0..SIZE)
            .map(|i| (i as f64 * std::f64::consts::PI * 2.0 / SIZE as f64).sin() as f32)
            .collect()
    })
}

/// Table units per radian, computed in f32 (= 65536 / 2pi).
#[inline]
fn units_per_radian() -> f32 {
    SIZE as f32 / std::f32::consts::TAU
}

#[inline]
pub(crate) fn sin(rad: f32) -> f32 {
    if USE_TABLE {
        table()[((rad * units_per_radian()) as i32 & (SIZE as i32 - 1)) as usize]
    } else {
        rad.sin()
    }
}

#[inline]
pub(crate) fn cos(rad: f32) -> f32 {
    if USE_TABLE {
        table()
            [((rad * units_per_radian() + (SIZE / 4) as f32) as i32 & (SIZE as i32 - 1)) as usize]
    } else {
        rad.cos()
    }
}

/// Degrees to radians for movement direction and the sprint-jump boost: `(deg * pi) / 180` in f32
/// (mcpk.wiki "Angles"; vanilla `LivingEntity.jumpFromGround` uses the same `(float)(PI / 180.0)`
/// conversion as the movement code).
#[inline]
pub(crate) fn deg_to_rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

// MERGE(jump-sprint side):
// MERGE(main side):
/// `(float)(Math.PI / 180.0)`, the f32 constant vanilla multiplies its angles by.
pub(crate) const DEG_TO_RAD_F32: f32 = (std::f64::consts::PI / 180.0) as f32;

/// Degrees to radians as vanilla spells it for the view vector: `deg * DEG_TO_RAD_F32` in f32.
#[inline]
pub(crate) fn deg_to_rad_f32(deg: f32) -> f32 {
    deg * DEG_TO_RAD_F32
}

/// Vanilla's sine-table scale (a double), and the mask and cosine offset it indexes with.
const SIN_SCALE: f64 = 10430.378350470453;
const SIN_MASK: i64 = (SIZE as i64) - 1;
const COS_OFFSET: f64 = 16384.0;

/// Vanilla `Mth.sin(double)`: table entry `(rad * SIN_SCALE) as long & 65535`.
#[inline]
pub(crate) fn sin_d(rad: f64) -> f32 {
    table()[((rad * SIN_SCALE) as i64 & SIN_MASK) as usize]
}

/// Vanilla `Mth.cos(double)`: the same index plus the quarter-turn offset, added before the cast.
#[inline]
pub(crate) fn cos_d(rad: f64) -> f32 {
    table()[((rad * SIN_SCALE + COS_OFFSET) as i64 & SIN_MASK) as usize]
}

/// Degrees to radians for the sprint-jump boost: `deg * 0.017453292` (mcpk.wiki "Angles" notes the
/// sprint jump uses this other constant). Assumption pending traces.
#[inline]
pub(crate) fn deg_to_rad_jump(deg: f32) -> f32 {
    deg * 0.017_453_292
}

// MERGE end
#[cfg(test)]
mod tests {
    #[test]
    fn units_per_radian_is_the_documented_value() {
        assert_eq!(super::units_per_radian(), 10430.378_f32);
    }
    #[test]
    fn table_close_to_exact() {
        for d in [0.0f32, 30.0, 45.0, 90.0, 180.0, 270.0, -45.0, 359.0] {
            let r = super::deg_to_rad(d);
            assert!((super::sin(r) - r.sin()).abs() < 1e-3, "{d}");
            assert!((super::cos(r) - r.cos()).abs() < 1e-3, "{d}");
        }
    }

    #[test]
    fn double_indexed_lookups_match_the_f32_ones_for_exact_angles() {
        // The double scale and the f32 scale are the same number; the lookups agree wherever both
        // land in the same entry (0, a quarter turn, a half turn).
        assert_eq!(super::sin_d(0.0), 0.0);
        assert_eq!(super::cos_d(0.0), 1.0);
        assert_eq!(super::sin_d(std::f64::consts::FRAC_PI_2), 1.0);
        assert_eq!(super::cos_d(std::f64::consts::PI), -1.0);
    }

    #[test]
    fn deg_to_rad_f32_is_vanillas_constant() {
        assert_eq!(super::DEG_TO_RAD_F32, 0.017_453_292_f32);
        // 60 degrees of it: the pitch in radians vanilla's look vector works with.
        let sixty = super::deg_to_rad_f32(60.0);
        assert!(
            (sixty - std::f32::consts::FRAC_PI_3).abs() < 1e-6,
            "{sixty}"
        );
        assert_eq!(super::deg_to_rad_f32(0.0), 0.0);
    }
}
