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

/// Degrees to radians for movement direction: `(deg * pi) / 180` in f32 (mcpk.wiki "Angles").
#[inline]
pub(crate) fn deg_to_rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

/// Degrees to radians for the sprint-jump boost: `deg * 0.017453292` (mcpk.wiki "Angles" notes the
/// sprint jump uses this other constant). Assumption pending traces.
#[inline]
pub(crate) fn deg_to_rad_jump(deg: f32) -> f32 {
    deg * 0.017_453_292
}

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
}
