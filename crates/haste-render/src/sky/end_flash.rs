// The End dimension's sky flash (vanilla `EndFlashState` ticked by `ClientLevel.tick` with
// the dimension clock's time, drawn by `SkyRenderer.renderEndFlash`): one flash every 600
// ticks of the End clock, its offset, duration and direction drawn from a seeded
// `SingleThreadedRandomSource` (the Java LCG), the brightness a half-sine over the duration.

use glam::{Mat4, Vec3};

use crate::java_random::JavaRandom;

/// One flash interval of the End clock (`EndFlashState.FLASH_INTERVAL_IN_TICKS`).
const FLASH_INTERVAL: i64 = 600;
/// The latest into an interval a flash can start (`MAX_FLASH_OFFSET_IN_TICKS`).
const MAX_OFFSET: i32 = 200;
/// The shortest a flash lasts (`MIN_FLASH_DURATION_IN_TICKS`).
const MIN_DURATION: i32 = 100;
/// The longest a flash lasts (`MAX_FLASH_DURATION_IN_TICKS`).
const MAX_DURATION: i32 = 380;
/// The flash quad's height (`SkyRenderer.END_FLASH_HEIGHT`).
const FLASH_HEIGHT: f32 = 100.0;
/// The flash quad's half-size (`SkyRenderer.END_FLASH_SCALE`).
const FLASH_SCALE: f32 = 60.0;

/// One interval's flash parameters (`EndFlashState.calculateFlashParameters`): a
/// `SingleThreadedRandomSource` seeded with `clockTime / 600`, its first `nextFloat`
/// discarded, then the offset 0..=200, the duration 100..=min(380, 600 - offset) and the
/// two direction angles in degrees (x -60..10, y -180..180, `Mth.randomBetween`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct FlashParams {
    offset: i32,
    duration: i32,
    x_angle: f32,
    y_angle: f32,
}

fn flash_params(seed: i64) -> FlashParams {
    let mut random = JavaRandom::new(seed);
    random.next_float();
    let offset = random.next_int(MAX_OFFSET + 1);
    let max_duration = MAX_DURATION.min((FLASH_INTERVAL as i32) - offset);
    let duration = MIN_DURATION + random.next_int(max_duration - MIN_DURATION + 1);
    let x_angle = random.next_float() * (10.0 - -60.0) + -60.0;
    let y_angle = random.next_float() * (180.0 - -180.0) + -180.0;
    FlashParams {
        offset,
        duration,
        x_angle,
        y_angle,
    }
}

/// The flash brightness at a clock time (`EndFlashState.calculateIntensity`): a half-sine
/// from `offset` to `offset + duration` within the interval, 0 outside it.
fn intensity_at(clock_time: i64, params: &FlashParams) -> f32 {
    let t = clock_time % FLASH_INTERVAL;
    let offset = params.offset as i64;
    let end = offset + params.duration as i64;
    if t >= offset && t <= end {
        ((t - offset) as f32 * std::f32::consts::PI / params.duration as f32).sin()
    } else {
        0.0
    }
}

/// Vanilla `EndFlashState`: ticked once per game tick with the End clock's time (the
/// dimension's default clock, `Level.getDefaultClockTime`); the renderer reads the
/// interpolated intensity and the flash's direction angles per frame.
#[derive(Clone, Debug)]
pub struct EndFlashState {
    /// The interval the parameters belong to (`clockTime / 600`); starts at 0 like Java's
    /// field default, so a first tick inside interval 0 keeps the zero parameters.
    flash_seed: i64,
    params: FlashParams,
    intensity: f32,
    old_intensity: f32,
}

impl Default for EndFlashState {
    fn default() -> Self {
        Self::new()
    }
}

impl EndFlashState {
    /// The state a fresh level starts with: no flash, the zero parameters (Java's field
    /// defaults, which hold until the clock reaches its second interval).
    pub fn new() -> Self {
        EndFlashState {
            flash_seed: 0,
            params: FlashParams {
                offset: 0,
                duration: 0,
                x_angle: 0.0,
                y_angle: 0.0,
            },
            intensity: 0.0,
            old_intensity: 0.0,
        }
    }

    /// One game tick (`EndFlashState.tick`): refresh the parameters when the interval
    /// rolled over, then bracket the intensity for the frame interpolation.
    pub fn tick(&mut self, clock_time: i64) {
        let new_seed = clock_time / FLASH_INTERVAL;
        if new_seed != self.flash_seed {
            self.params = flash_params(new_seed);
            self.flash_seed = new_seed;
        }
        self.old_intensity = self.intensity;
        self.intensity = intensity_at(clock_time, &self.params);
    }

    /// The brightness this frame (`getIntensity`), lerped between the last two ticks.
    pub fn intensity(&self, partial_ticks: f32) -> f32 {
        self.old_intensity + (self.intensity - self.old_intensity) * partial_ticks
    }

    /// The flash's x direction angle in degrees (`getXAngle`).
    pub fn x_angle(&self) -> f32 {
        self.params.x_angle
    }

    /// The flash's y direction angle in degrees (`getYAngle`).
    pub fn y_angle(&self) -> f32 {
        self.params.y_angle
    }

    /// Whether the flash just became visible (`flashStartedThisTick`); vanilla plays the
    /// end-flash sound when this turns true.
    pub fn flash_started_this_tick(&self) -> bool {
        self.intensity > 0.0 && self.old_intensity <= 0.0
    }
}

/// The flash quad's pose (`SkyRenderer.renderEndFlash`): `rotateDegrees(Axis.YP,
/// 180 - yAngle)` then `rotateDegrees(Axis.XP, -90 - xAngle)`, and
/// `applyCelestialBodyTransform`'s translate to height 100 and scale to half-size 60.
pub fn pose(x_angle: f32, y_angle: f32) -> Mat4 {
    Mat4::from_rotation_y((180.0 - y_angle).to_radians())
        * Mat4::from_rotation_x((-90.0 - x_angle).to_radians())
        * Mat4::from_translation(Vec3::new(0.0, FLASH_HEIGHT, 0.0))
        * Mat4::from_scale(Vec3::new(FLASH_SCALE, 1.0, FLASH_SCALE))
}

#[cfg(test)]
mod tests;
