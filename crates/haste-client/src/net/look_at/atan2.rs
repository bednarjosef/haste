//! `Mth.atan2`: vanilla's table approximation of the angle, not `f64::atan2`. Its arcsine and
//! cosine tables are built once from 257 steps of 1/256, and a fast inverse square root scales
//! the operands, so the angle matches the client's value to the bit.

use std::f64::consts::PI;
use std::sync::OnceLock;

/// `Mth.FRAC_BIAS`: 2^44. Adding y in [0, 1] keeps y's 1/256 steps in the low mantissa bits.
const FRAC_BIAS: f64 = f64::from_bits(4805340802404319232);

/// The magic constant of `Mth.fastInvSqrt`.
const INV_SQRT_MAGIC: i64 = 6910469410427058090;

/// `Mth.ASIN_TAB` and `Mth.COS_TAB`: `asin(i / 256)` and its cosine for i in 0..=256.
struct Tables {
    asin: [f64; 257],
    cos: [f64; 257],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut asin = [0.0; 257];
        let mut cos = [0.0; 257];
        for (ind, (a, c)) in asin.iter_mut().zip(cos.iter_mut()).enumerate() {
            let asinv = (ind as f64 / 256.0).asin();
            *c = asinv.cos();
            *a = asinv;
        }
        Tables { asin, cos }
    })
}

/// `Mth.fastInvSqrt`: one Newton step from the bit-hack guess.
fn fast_inv_sqrt(x: f64) -> f64 {
    let xhalf = 0.5 * x;
    let i = INV_SQRT_MAGIC - ((x.to_bits() as i64) >> 1);
    let x = f64::from_bits(i as u64);
    x * (1.5 - xhalf * x * x)
}

/// `Mth.atan2(y, x)`: the angle of the point (x, y) in radians, in (-PI, PI].
pub(super) fn atan2(y: f64, x: f64) -> f64 {
    let d2 = x * x + y * y;
    if d2.is_nan() {
        return f64::NAN;
    }

    let neg_y = y < 0.0;
    let mut y = if neg_y { -y } else { y };
    let neg_x = x < 0.0;
    let mut x = if neg_x { -x } else { x };

    let steep = y > x;
    if steep {
        std::mem::swap(&mut x, &mut y);
    }

    let rinv = fast_inv_sqrt(d2);
    x *= rinv;
    y *= rinv;
    let yp = FRAC_BIAS + y;
    // The low 32 bits of yp's bits are the table index (Java's `(int)` of the raw long).
    let index = (yp.to_bits() as u32) as usize;
    let tables = tables();
    let phi = tables.asin[index];
    let c_phi = tables.cos[index];
    let s_phi = yp - FRAC_BIAS;
    let sd = y * c_phi - x * s_phi;
    let d = (6.0 + sd * sd) * sd * 0.16666666666666666;
    let mut theta = phi + d;

    if steep {
        theta = PI / 2.0 - theta;
    }
    if neg_x {
        theta = PI - theta;
    }
    if neg_y {
        theta = -theta;
    }
    theta
}
