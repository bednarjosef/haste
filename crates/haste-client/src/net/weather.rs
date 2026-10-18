// The weather extraction: the client's rain/thunder levels (`WeatherState`, ticked and lerped
// by partial tick), the biome precipitation lookup the render thread's per-column scan and the
// rain splash particles share (`precipitation_at_pos`, `column_height`), and the per-tick rain
// sound and particle spawn of `ClientLevel.tickWeatherEffects`.

use glam::DVec3;

use haste_protocol::haste_core::game::DimensionInfo;
use haste_protocol::haste_core::BlockPos;
use haste_world::World;

/// The client's weather state (vanilla `Level`'s rain/thunder levels plus
/// `ClientLevel.tickWeatherEffects`): the levels bracket the tick so the render thread can
/// lerp by partial tick (`Level.getRainLevel(a)`), and the tick spawns the rain splash
/// particles and plays the rain sound every few ticks.
#[derive(Clone, Debug)]
pub struct WeatherState {
    /// `Level.oRainLevel`: the rain level at the start of the tick.
    pub rain_level_o: f32,
    /// `Level.rainLevel`.
    pub rain_level: f32,
    /// `Level.oThunderLevel` (before the rain scaling `getThunderLevel` applies).
    pub thunder_level_o: f32,
    /// `Level.thunderLevel`.
    pub thunder_level: f32,
    /// `ClientLevel.rainSoundTime`: the rain sound's counter (`random.nextInt(3) <
    /// rainSoundTime++` picks the tick it plays on).
    rain_sound_time: i32,
    /// The RNG of `tickWeatherEffects` (a thread-local random in vanilla; a fixed seed
    /// keeps Haste's runs reproducible, and the sequence is not player-visible).
    rng: u64,
}

impl Default for WeatherState {
    fn default() -> Self {
        Self {
            rain_level_o: 0.0,
            rain_level: 0.0,
            thunder_level_o: 0.0,
            thunder_level: 0.0,
            rain_sound_time: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
        }
    }
}

impl WeatherState {
    /// Uniform f32 in [0, 1) (the xorshift of `LightmapState`'s flicker RNG).
    fn next_float(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / 16_777_216.0
    }

    /// `Level.setRainLevel`: clamps and writes both the bracketed level and the target.
    pub fn set_rain_level(&mut self, level: f32) {
        let l = level.clamp(0.0, 1.0);
        self.rain_level_o = l;
        self.rain_level = l;
    }

    /// `Level.setThunderLevel`.
    pub fn set_thunder_level(&mut self, level: f32) {
        let l = level.clamp(0.0, 1.0);
        self.thunder_level_o = l;
        self.thunder_level = l;
    }

    /// One tick: shifts the bracket (`ClientLevel.tick` moves the old level to the target
    /// each tick; the packets set both, so the lerp only shows between the server's
    /// `rain_level_change` steps).
    pub fn tick(&mut self) {
        self.rain_level_o = self.rain_level;
        self.thunder_level_o = self.thunder_level;
    }

    /// `Level.getRainLevel(a)`: lerped by partial tick.
    pub fn rain_level(&self, partial: f32) -> f32 {
        self.rain_level_o + (self.rain_level - self.rain_level_o) * partial
    }

    /// Whether the dimension can have weather at all (`Level.canHaveWeather`: skylight, no
    /// ceiling, not the End). `DimensionInfo` carries no ceiling flag, so the known
    /// ceiling dimensions are excluded by name (the Nether); custom skyless dimensions are
    /// already excluded by the skylight flag.
    pub fn can_have_weather(dimension: &DimensionInfo) -> bool {
        dimension.has_skylight
            && !is_end(dimension)
            && dimension.dimension_type.path() != "the_nether"
            && dimension.name.path() != "the_nether"
    }

    /// `ClientLevel.tickWeatherEffects`: spawns the rain splash particles around the
    /// camera and plays the rain sound every few ticks. The particle spawns and the sound
    /// request are pushed onto the caller's reused vectors (the game thread routes them to
    /// the particle pool and the sound output); `weather_radius` is the `weatherRadius`
    /// option, `climates` the biome climate table. Called by the game thread after
    /// `end_tick` (vanilla runs it inside `ClientLevel.tick`, after the player's tick).
    #[allow(clippy::too_many_arguments)]
    pub fn tick_effects(
        &mut self,
        camera: DVec3,
        weather_radius: i32,
        world: &World,
        climates: &[haste_render::BiomeClimate],
        sea_level: i32,
        min_y: i32,
        particles: &mut Vec<haste_render::ParticleSpawn>,
        sounds: &mut Vec<crate::sound::SoundRequest>,
    ) {
        let rain_level = self.rain_level(1.0);
        if rain_level <= 0.0 {
            return;
        }
        let weather_diameter = 2 * weather_radius + 1;
        let weather_area = weather_diameter * weather_diameter;
        // Vanilla: (int)(0.225F * weatherArea * rainLevel^2) (particle status ALL; the
        // decreased option halves it, minimal takes the first hit only — Haste runs all).
        let rain_particles = (0.225_f32 * weather_area as f32 * rain_level * rain_level) as i32;
        let camera_block = BlockPos::new(
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        );
        let mut rain_particle_position: Option<BlockPos> = None;
        for _ in 0..rain_particles {
            let x = (self.next_float() * weather_diameter as f32) as i32 - weather_radius;
            let z = (self.next_float() * weather_diameter as f32) as i32 - weather_radius;
            let column = camera_block.add(x, 0, z);
            let Some(heightmap_position) = heightmap_pos(world, &column, min_y) else {
                continue;
            };
            if heightmap_position.y > min_y
                && heightmap_position.y <= camera_block.y + 10
                && heightmap_position.y >= camera_block.y - 10
                && precipitation_at(world, climates, &heightmap_position, sea_level)
                    == Precipitation::Rain
            {
                let pos = heightmap_position.add(0, -1, 0);
                rain_particle_position = Some(pos);
                let block_x = self.next_float() as f64;
                let block_z = self.next_float() as f64;
                // The particle's y sits on the block's (or the fluid's) top surface; without
                // the shape lookup here it is the block's own top (vanilla
                // `VoxelShape.max`/`FluidState.getHeight` at the random x/z).
                let particle_y = pos.y as f64 + 1.0;
                particles.push(rain_particle(pos, block_x, particle_y, block_z));
            }
        }
        // The rain sound: `random.nextInt(3) < rainSoundTime++` (the counter grows every
        // tick it rains, so after a few ticks the sound fires and resets), at the last
        // particle's position, above the camera when there is a roof over the player. The
        // short-circuit means the random is only drawn when a particle landed.
        let fires = rain_particle_position.is_some_and(|_| {
            let next = (self.next_float() * 3.0) as i32;
            let old = self.rain_sound_time;
            self.rain_sound_time = old + 1;
            next < old
        });
        if fires {
            self.rain_sound_time = 0;
            let pos = rain_particle_position.expect("checked above");
            let above = pos.y > camera_block.y + 1
                && heightmap_pos(world, &camera_block, min_y)
                    .is_some_and(|h| h.y > camera.y.floor() as i32);
            let (event, volume, pitch) = if above {
                ("minecraft:weather.rain.above", 0.1, 0.5)
            } else {
                ("minecraft:weather.rain", 0.2, 1.0)
            };
            sounds.push(crate::sound::SoundRequest::Play(
                crate::sound::PlayRequest {
                    sound: crate::sound::SoundRef::Inline(event.to_string()),
                    // `SoundSource.WEATHER` (the packet's category order: master 0, music 1,
                    // records 2, weather 3).
                    category: 3,
                    pos: crate::sound::SoundPos::Fixed(DVec3::new(
                        pos.x as f64,
                        pos.y as f64,
                        pos.z as f64,
                    )),
                    volume,
                    pitch,
                    seed: 0,
                },
            ));
        }
    }
}

/// A biome's precipitation at a position (vanilla `Biome.Precipitation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precipitation {
    None,
    Rain,
    Snow,
}

/// The weather extraction's public lookup: the biome precipitation at a position (the
/// render thread's per-column scan; see `precipitation_at`).
pub fn precipitation_at_pos(
    world: &World,
    climates: &[haste_render::BiomeClimate],
    pos: &BlockPos,
    sea_level: i32,
) -> Precipitation {
    precipitation_at(world, climates, pos, sea_level)
}

/// The weather extraction's public lookup: a column's MOTION_BLOCKING height (0 below the
/// world when the chunk is not loaded, matching `Level.getHeight`'s empty fallback).
pub fn column_height(world: &World, x: i32, z: i32) -> i32 {
    let column = BlockPos::new(x, 0, z);
    let min_y = world.dimension().min_y;
    heightmap_pos(world, &column, min_y)
        .map(|p| p.y)
        .unwrap_or(min_y)
}

/// Whether the dimension is the End (`Level.canHaveWeather` excludes it by id).
fn is_end(dim: &DimensionInfo) -> bool {
    dim.name.path() == "the_end" || dim.dimension_type.path() == "the_end"
}

/// `ClientLevel.getHeightmapPos` for the MOTION_BLOCKING heightmap (wire type 4): the
/// heightmap the chunk sent, else a linear scan of the column (`Chunk.height_at`).
fn heightmap_pos(world: &World, column: &BlockPos, min_y: i32) -> Option<BlockPos> {
    let chunk = world.chunk(column.chunk())?;
    let local_x = (column.x & 15) as u8;
    let local_z = (column.z & 15) as u8;
    let h = chunk
        .heightmap(4)
        .map(|h| h.get(local_x, local_z) + min_y)
        .or_else(|| chunk.height_at(local_x, local_z).map(|y| y + 1))?;
    Some(BlockPos::new(column.x, h, column.z))
}

/// `Biome.getPrecipitationAt` at a position: no precipitation where the biome has none,
/// else snow when the height-adjusted temperature is below 0.15
/// (`Biome.coldEnoughToSnow`/`warmEnoughToRain`). The height adjustment above the snow
/// line (sea level + 17) subtracts `(noise + y - snowLevel) * 0.05 / 40`; the temperature
/// noise (`Biome.TEMPERATURE_NOISE`, simplex seeded 1234) is approximated by a smooth
/// hash of the column — its exact value differs from vanilla's, so the snow line wobbles
/// slightly differently. Returns `None` when the column's chunk is not loaded (vanilla
/// `getPrecipitationAt` returns NONE then).
fn precipitation_at(
    world: &World,
    climates: &[haste_render::BiomeClimate],
    pos: &BlockPos,
    sea_level: i32,
) -> Precipitation {
    use haste_render::BiomeSource as _;
    let biome = crate::app::biomes::WorldBiomes(world).biome(*pos).0 as usize;
    let Some(climate) = climates.get(biome) else {
        return Precipitation::None;
    };
    if !climate.has_precipitation {
        return Precipitation::None;
    }
    let snow_level = sea_level + 17;
    let mut temperature = climate.temperature;
    if pos.y > snow_level {
        // Approximation of TEMPERATURE_NOISE.get(x/8, z/8) * 8: a deterministic value in
        // about -8..8 from the column (see the doc comment).
        let v = temperature_noise(pos.x, pos.z);
        temperature -= (v + (pos.y - snow_level) as f32) * 0.05 / 40.0;
    }
    if temperature >= 0.15 {
        Precipitation::Rain
    } else {
        Precipitation::Snow
    }
}

/// The temperature noise stand-in: a fixed pseudo-random value in about -8..8 per column,
/// seeded from the coordinates (vanilla's simplex value at x/8, z/8).
fn temperature_noise(x: i32, z: i32) -> f32 {
    let mut h = (x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u32 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 29;
    ((h >> 40) as f32 / (1u32 << 24) as f32) * 16.0 - 8.0
}

/// One rain splash particle (`ParticleTypes.RAIN`, the `WaterDropParticle` the particle
/// pool knows: gravity 0.06, lifetime 8/(r*0.8+0.2)).
fn rain_particle(
    pos: BlockPos,
    block_x: f64,
    particle_y: f64,
    block_z: f64,
) -> haste_render::ParticleSpawn {
    haste_render::ParticleSpawn {
        kind: haste_core::Identifier::minecraft("rain"),
        pos: DVec3::new(pos.x as f64 + block_x, particle_y, pos.z as f64 + block_z),
        offset: [0.0; 3],
        max_speed: [0.0; 3],
        count: 0,
        randomization: 0,
        override_limiter: false,
        dust: None,
        block_state: None,
    }
}
