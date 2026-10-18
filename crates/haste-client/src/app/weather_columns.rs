//! The weather extraction (vanilla `WeatherEffectRenderer.extractRenderState`): the per-column
//! scan around the camera, its per-tick cache of heights/precipitation/light, and the seeded
//! Java LCG the column instances' texture offsets come from.

use glam::DVec3;
use haste_protocol::haste_core::BlockPos;
use haste_world::World;

/// Vanilla `RandomSource` (the single-threaded variant `WeatherEffectRenderer` uses): the
/// 48-bit Java LCG. `setSeed` xors the multiplier, `next(bits)` advances the seed.
struct JavaRandom(u64);

/// The Java LCG's multiplier, 25214903917 (0x5DEECE66D).
const LCG_MULTIPLIER: u64 = 25_214_903_917;
/// The Java LCG's increment.
const LCG_INCREMENT: u64 = 11;
/// The Java LCG's 48-bit mask.
const LCG_MASK: u64 = (1u64 << 48) - 1;

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ LCG_MULTIPLIER) & LCG_MASK)
    }

    fn next_i32(&mut self, bits: u32) -> i32 {
        self.0 = (self
            .0
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT))
            & LCG_MASK;
        (self.0 >> (48 - bits)) as i32
    }

    /// `nextFloat`.
    fn next_float(&mut self) -> f32 {
        self.next_i32(24) as f32 * 5.960_464_5E-8
    }

    /// `nextDouble`.
    fn next_double(&mut self) -> f64 {
        let upper = self.next_i32(26) as u64;
        let lower = self.next_i32(27) as u64;
        ((upper << 27) + lower) as f64 * 1.110_223E-16
    }

    /// `nextGaussian` (Marsaglia polar).
    fn next_gaussian(&mut self) -> f64 {
        loop {
            let x = 2.0 * self.next_double() - 1.0;
            let y = 2.0 * self.next_double() - 1.0;
            let r2 = x * x + y * y;
            if r2 < 1.0 && r2 > 0.0 {
                let m = (-2.0 * r2.ln() / r2).sqrt();
                return x * m;
            }
        }
    }
}

/// One cached weather column: (x, z, terrain height, precipitation, light at the sample
/// position) — the parts of the extraction that only change when the camera leaves its
/// block cell or the world changes.
type CachedWeatherColumn = (i32, i32, i32, crate::net::Precipitation, (u8, u8));

/// The per-tick part of the weather extraction: a column's terrain height, its precipitation
/// and its light sample, which only change when the camera leaves its block cell or the world
/// changes (the world version bumps on every dirty-section publish). The frame's scroll
/// offsets are recomputed every frame; these are not.
#[derive(Default)]
pub(crate) struct WeatherColumnCache {
    /// The camera's block cell the cache was built for.
    center: (i32, i32, i32),
    /// The world version it was built at.
    world_version: u64,
    /// One entry per scanned column, in scan order.
    columns: Vec<CachedWeatherColumn>,
}

impl WeatherColumnCache {
    /// The cached columns for `camera`'s block cell, rebuilding only when the cell or the
    /// world changed since the last call.
    fn columns(
        &mut self,
        world: &World,
        climates: &[haste_render::BiomeClimate],
        camera: DVec3,
        world_version: u64,
        sea_level: i32,
    ) -> &[CachedWeatherColumn] {
        use haste_core::view::BlockView as _;
        let radius = haste_render::weather::WEATHER_RADIUS;
        let cbx = camera.x.floor() as i32;
        let cby = camera.y.floor() as i32;
        let cbz = camera.z.floor() as i32;
        if self.center == (cbx, cby, cbz) && self.world_version == world_version {
            return &self.columns;
        }
        self.center = (cbx, cby, cbz);
        self.world_version = world_version;
        self.columns.clear();
        for z in cbz - radius..=cbz + radius {
            for x in cbx - radius..=cbx + radius {
                // `Level.getHeight(MOTION_BLOCKING, x, z)` (0 below the world when no chunk:
                // the column then degenerates and is skipped by the y test, like vanilla's).
                let terrain_height = crate::net::column_height(world, x, z);
                // The biome precipitation at the camera's y (vanilla probes at the camera).
                let probe = BlockPos::new(x, cby, z);
                let precipitation =
                    crate::net::precipitation_at_pos(world, climates, &probe, sea_level);
                let light_sample_y = cby.max(terrain_height);
                let light_pos = BlockPos::new(x, light_sample_y, z);
                let light = (world.block_light(light_pos), world.sky_light(light_pos));
                self.columns
                    .push((x, z, terrain_height, precipitation, light));
            }
        }
        &self.columns
    }
}

/// Vanilla `WeatherEffectRenderer.extractRenderState` + the per-column instances: scans the
/// weather radius around the camera, clips each column's y range to the heightmap, sorts it
/// into rain or snow by the biome's precipitation at the camera's y, and fills the per-column
/// random's texture offsets. Fills the caller's reused frame buffer; the heights,
/// precipitation and light come from `cache` (rebuilt only when the camera's block cell or
/// the world changed), only the scroll offsets are recomputed per frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_weather(
    frame: &mut haste_render::WeatherFrame,
    cache: &mut WeatherColumnCache,
    world: &World,
    climates: &[haste_render::BiomeClimate],
    camera: DVec3,
    intensity: f32,
    game_time: i64,
    sea_level: i32,
    partial_tick: f32,
    world_version: u64,
) {
    let radius = haste_render::weather::WEATHER_RADIUS;
    frame.intensity = intensity;
    frame.radius = radius;
    frame.rain.clear();
    frame.snow.clear();
    let cby = camera.y.floor() as i32;
    // One-time diagnostics: the camera, sea level, one column's height and the frame's
    // rain/snow counts, so an invisible weather pass is debuggable from the log.
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let mut terrain_of_first: Option<i32> = None;
    let columns = cache.columns(world, climates, camera, world_version, sea_level);
    for &(x, z, terrain_height, precipitation, light) in columns {
        let y0 = (cby - radius).max(terrain_height);
        let y1 = (cby + radius).max(terrain_height);
        if y1 - y0 == 0 {
            continue;
        }
        if terrain_of_first.is_none() {
            terrain_of_first = Some(terrain_height);
        }
        let seed = haste_render::weather::column_seed(x, z);
        let mut rng = JavaRandom::new(seed as i64);
        let column = haste_render::WeatherColumn {
            x,
            z,
            bottom_y: y0,
            top_y: y1,
            u_offset: 0.0,
            v_offset: 0.0,
            light,
            seed,
        };
        match precipitation {
            crate::net::Precipitation::Rain => {
                // `createRainColumnInstance`: the scroll speed 3 + rand, the wrapped
                // tick offset and the v offset it scrolls to.
                let wrapped_ticks = (game_time & 131071) as f32;
                let tick_offset = (x
                    .wrapping_mul(x)
                    .wrapping_mul(3121)
                    .wrapping_add(x.wrapping_mul(45238971))
                    .wrapping_add(z.wrapping_mul(z).wrapping_mul(418711))
                    .wrapping_add(z.wrapping_mul(13761)))
                    & 0xFF;
                let speed = 3.0 + rng.next_float();
                let texture_offset =
                    -(wrapped_ticks + tick_offset as f32 + partial_tick) / 32.0 * speed;
                frame.rain.push(haste_render::WeatherColumn {
                    v_offset: texture_offset % 32.0,
                    ..column
                });
            }
            crate::net::Precipitation::Snow => {
                // `createSnowColumnInstance`: the drift u/v from the seeded gaussian
                // pair, the slow fall, and the brightened light.
                let wrapped_ticks = (game_time & 131071) as f32;
                let time = wrapped_ticks as f64 + partial_tick as f64;
                let u = (rng.next_double() + time * 0.01 * rng.next_gaussian()) as f32;
                let v = (rng.next_double() + time * rng.next_gaussian() * 0.001) as f32;
                let v_offset = -((game_time & 511) as f32 + partial_tick) / 512.0 + v;
                let brightened = (
                    ((light.0 as i32 * 3 + 15) / 4).clamp(0, 15) as u8,
                    ((light.1 as i32 * 3 + 15) / 4).clamp(0, 15) as u8,
                );
                frame.snow.push(haste_render::WeatherColumn {
                    u_offset: u,
                    v_offset,
                    light: brightened,
                    ..column
                });
            }
            crate::net::Precipitation::None => {}
        }
    }
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::info!(
            "weather extraction: camera ({:.1}, {:.1}, {:.1}), sea level {sea_level}, first \
             column height {:?}, {} rain + {} snow columns, intensity {:.3}",
            camera.x,
            camera.y,
            camera.z,
            terrain_of_first,
            frame.rain.len(),
            frame.snow.len(),
            intensity
        );
    }
}

#[cfg(test)]
mod tests;
