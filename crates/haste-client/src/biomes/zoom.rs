//! Vanilla's biome zoom fuzz (`BiomeManager.getBiome`): the biome at a block is not its own
//! 4x4x4 cell but the nearest of the 8 surrounding cell corners, each pushed by a seeded
//! fiddle, so colour borders wobble instead of stepping in straight 4-block lines.

/// The quart cell `BiomeManager.getBiome` resolves a block position to (its noise-biome
/// coordinates): the corner of the 2x2x2 neighbourhood of quart cells around the block that
/// wins the fiddled-distance comparison.
pub fn zoomed_cell(seed: i64, x: i32, y: i32, z: i32) -> (i32, i32, i32) {
    // Java's `int` arithmetic wraps; the -2 and the corner +1 must too.
    let (ax, ay, az) = (x.wrapping_sub(2), y.wrapping_sub(2), z.wrapping_sub(2));
    let (px, py, pz) = (ax >> 2, ay >> 2, az >> 2);
    let fx = (ax & 3) as f64 / 4.0;
    let fy = (ay & 3) as f64 / 4.0;
    let fz = (az & 3) as f64 / 4.0;
    let mut best_i = 0;
    let mut best = f64::INFINITY;
    for i in 0..8 {
        let x_even = (i & 4) == 0;
        let y_even = (i & 2) == 0;
        let z_even = (i & 1) == 0;
        let corner_x = if x_even { px } else { px.wrapping_add(1) };
        let corner_y = if y_even { py } else { py.wrapping_add(1) };
        let corner_z = if z_even { pz } else { pz.wrapping_add(1) };
        let dx = if x_even { fx } else { fx - 1.0 };
        let dy = if y_even { fy } else { fy - 1.0 };
        let dz = if z_even { fz } else { fz - 1.0 };
        let next = fiddled_distance(seed, corner_x, corner_y, corner_z, dx, dy, dz);
        if best > next {
            best_i = i;
            best = next;
        }
    }
    (
        if best_i & 4 == 0 {
            px
        } else {
            px.wrapping_add(1)
        },
        if best_i & 2 == 0 {
            py
        } else {
            py.wrapping_add(1)
        },
        if best_i & 1 == 0 {
            pz
        } else {
            pz.wrapping_add(1)
        },
    )
}

/// `LinearCongruentialGenerator.next`: `rval * (rval * multiplier + increment) + c`, all
/// wrapping like Java's `long`.
fn lcg_next(rval: i64, c: i64) -> i64 {
    rval.wrapping_mul(
        rval.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407),
    )
    .wrapping_add(c)
}

/// `BiomeManager.getFiddledDistance`: the squared distance from the block to a corner, each
/// axis stretched by a per-corner fiddle derived from the seed.
fn fiddled_distance(seed: i64, cx: i32, cy: i32, cz: i32, dx: f64, dy: f64, dz: f64) -> f64 {
    let mut rval = seed;
    rval = lcg_next(rval, cx as i64);
    rval = lcg_next(rval, cy as i64);
    rval = lcg_next(rval, cz as i64);
    rval = lcg_next(rval, cx as i64);
    rval = lcg_next(rval, cy as i64);
    rval = lcg_next(rval, cz as i64);
    let fiddle_x = fiddle(rval);
    rval = lcg_next(rval, seed);
    let fiddle_y = fiddle(rval);
    rval = lcg_next(rval, seed);
    let fiddle_z = fiddle(rval);
    (dz + fiddle_z) * (dz + fiddle_z)
        + (dy + fiddle_y) * (dy + fiddle_y)
        + (dx + fiddle_x) * (dx + fiddle_x)
}

/// `BiomeManager.getFiddle`: a corner axis offset in -0.45..0.45 from the LCG state.
fn fiddle(rval: i64) -> f64 {
    let uniform = (rval >> 24).rem_euclid(1024) as f64 / 1024.0;
    (uniform - 0.5) * 0.9
}

#[cfg(test)]
#[path = "zoom/tests.rs"]
mod tests;
