//! The tick's weather step (vanilla `ClientLevel.tick` -> `tickWeatherEffects`) and the
//! rain-fog helpers the view update reads (`AtmosphericFogEnvironment.updateRainFogState`).

use glam::DVec3;
use haste_protocol::haste_core::BlockPos;
use haste_world::World;

use super::shared::Shared;
use crate::net::NetDriver;

/// `AtmosphericFogEnvironment.updateRainFogState`'s sky-light factor: the sky light at the
/// camera's block, clamped `(light - 8) / 7`.
pub(super) fn rain_fog_sky_light(world: &World, eye: DVec3) -> f32 {
    use haste_core::view::BlockView as _;
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    ((world.sky_light(p) as f32) - 8.0) / 7.0
}

/// Whether the biome at the eye has precipitation (`updateRainFogState` halves the rain
/// fog when it does not).
pub(super) fn biome_has_precipitation(
    world: &World,
    climates: &[haste_render::BiomeClimate],
    eye: DVec3,
) -> bool {
    use haste_render::BiomeSource as _;
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    let biome = crate::app::biomes::WorldBiomes(world).biome(p).0 as usize;
    climates.get(biome).is_some_and(|c| c.has_precipitation)
}

/// The weather effects' tick (`ClientLevel.tick` -> `tickWeatherEffects`, after the
/// entities): the rain splash particles join this tick's spawn list and the rain
/// sound joins the sound requests below. The levels' bracket shifts here too (the
/// render thread lerps it by partial tick).
pub(super) fn tick(driver: &mut NetDriver, world: &World, shared: &Shared) {
    driver.session.weather.tick();
    if crate::net::WeatherState::can_have_weather(&driver.session.dimension) {
        let camera = driver.session.camera_pose(1.0).0;
        driver.session.weather.tick_effects(
            camera,
            haste_render::weather::WEATHER_RADIUS,
            world,
            &shared.biome_climates,
            driver.session.sea_level,
            driver.session.dimension.min_y,
            &mut driver.session.particle_spawns,
            &mut driver.session.sound_requests,
        );
    }
}
