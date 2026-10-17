use super::*;

fn state(t: i64) -> SkyState {
    SkyState {
        time_of_day: t,
        rain: 0.0,
        dimension_has_sky: true,
        fog_start: 0.0,
        fog_end: 192.0,
        ..Default::default()
    }
}

#[test]
fn celestial_angle_noon_midnight() {
    assert!(celestial_angle(6000).abs() < 1e-6);
    assert!((celestial_angle(18000) - 0.5).abs() < 1e-6);
    assert!((celestial_angle(30000) - 0.0).abs() < 1e-6);
}

#[test]
fn day_is_bright_night_is_dark_with_stars() {
    let noon = sky_colors(&state(6000), SkyKind::Overworld, 12.0);
    assert_eq!(noon.brightness, 1.0);
    assert_eq!(noon.sky, rgb(0x78A7FF));
    assert_eq!(noon.star_brightness, 0.0);
    let night = sky_colors(&state(18000), SkyKind::Overworld, 12.0);
    assert_eq!(night.brightness, 0.0);
    assert!(night.star_brightness > 0.4);
    // Sunrise glow around time 23000-1000, none at noon.
    assert_eq!(noon.sunrise[3], 0.0);
    let dawn = sky_colors(&state(23500), SkyKind::Overworld, 12.0);
    assert!(dawn.sunrise[3] > 0.3, "{dawn:?}");
    assert_eq!(dawn.sunrise_side, 1.0, "dawn glow is in the east");
    let dusk = sky_colors(&state(12500), SkyKind::Overworld, 12.0);
    assert_eq!(dusk.sunrise_side, -1.0, "dusk glow is in the west");
}

#[test]
fn rain_greys_the_sky_like_the_weather_attributes() {
    // `WeatherAttributes.RAIN` over SKY_COLOR: lerp(0.75, colour, grey * 0.6) applied
    // by the rain level (rain minus thunder). At noon (0x78A7FF) with full rain and no
    // thunder: grey = 0.3*0.4706 + 0.59*0.6549 + 0.11*1.0 = 0.6427.
    let mut s = state(6000);
    s.rain = 1.0;
    let noon = sky_colors(&s, SkyKind::Overworld, 12.0);
    let grey = 0.3 * (0x78 as f32 / 255.0) + 0.59 * (0xA7 as f32 / 255.0) + 0.11 * 1.0;
    let target = grey * 0.6;
    let expect: [f32; 3] = [
        0.4706 + (target - 0.4706) * 0.75,
        0.6549 + (target - 0.6549) * 0.75,
        1.0 + (target - 1.0) * 0.75,
    ];
    for (i, (got, want)) in noon.sky.iter().zip(expect).enumerate() {
        assert!((got - want).abs() < 1e-4, "channel {i}: {got} vs {want}");
    }
    // Stars are gone with any weather.
    assert_eq!(noon.star_brightness, 0.0);
}

#[test]
fn thunder_darkens_the_sky_further() {
    // `WeatherAttributes.THUNDER`: lerp(0.94, colour, grey * 0.24) by the thunder level
    // (which the rain level scales). Full thunder implies rain 1: the rain layer first
    // greys by the rain-minus-thunder level (0 at full thunder), then the thunder layer
    // takes over. The thunder sky is darker than the rain sky.
    let mut rain = state(6000);
    rain.rain = 1.0;
    let rain_sky = sky_colors(&rain, SkyKind::Overworld, 12.0).sky;
    let mut storm = state(6000);
    storm.rain = 1.0;
    storm.thunder = 1.0;
    let storm_sky = sky_colors(&storm, SkyKind::Overworld, 12.0).sky;
    for i in 0..3 {
        assert!(
            storm_sky[i] < rain_sky[i] + 1e-6,
            "thunder darkens channel {i}: {} vs {}",
            storm_sky[i],
            rain_sky[i]
        );
    }
    // Full thunder at noon: the colour is the base blended to the grey times 0.24 by
    // the 0.94 factor (the rain layer's weight is rain minus thunder = 0 here).
    let base = rgb(0x78A7FF);
    let grey = 0.3 * base[0] + 0.59 * base[1] + 0.11 * base[2];
    let target = grey * 0.24;
    for i in 0..3 {
        let expect = base[i] + (target - base[i]) * 0.94;
        assert!(
            (storm_sky[i] - expect).abs() < 1e-4,
            "channel {i}: {} vs {expect}",
            storm_sky[i]
        );
    }
}

#[test]
fn moon_phase_cycles_daily() {
    assert_eq!(
        sky_colors(&state(6000), SkyKind::Overworld, 12.0).moon_phase,
        0
    );
    assert_eq!(
        sky_colors(&state(24000 * 4 + 6000), SkyKind::Overworld, 12.0).moon_phase,
        4
    );
    assert_eq!(
        sky_colors(&state(24000 * 9), SkyKind::Overworld, 12.0).moon_phase,
        1
    );
}

#[test]
fn sun_rises_east_sets_west() {
    let sun = |t: i64| celestial_matrix(celestial_angle(t)).transform_point3(Vec3::Y * 100.0);
    assert!(sun(6000).y > 99.0);
    assert!(sun(0).x > 50.0, "morning sun in the east: {:?}", sun(0));
    assert!(
        sun(12000).x < -50.0,
        "evening sun in the west: {:?}",
        sun(12000)
    );
}

#[test]
fn dimensions_pick_their_sky() {
    let mut d = DimensionInfo::overworld();
    assert_eq!(SkyKind::of(&d), SkyKind::Overworld);
    d.dimension_type = Identifier::minecraft("the_nether");
    assert_eq!(SkyKind::of(&d), SkyKind::None);
    d.dimension_type = Identifier::minecraft("the_end");
    assert_eq!(SkyKind::of(&d), SkyKind::End);
}

/// Vanilla's moon (`SkyRenderer.buildMoonPhases` corner (-1, 0, -1) takes (U1, V1), then the
/// moon angle turns the quad 180 degrees about X): in our frame the corner at (-h, -100, -h)
/// shows (U1, V0) and (h, -100, -h) shows (U0, V0). The sun keeps U0 at -x.
#[test]
fn moon_uvs_follow_vanilla_mirroring() {
    let rect = [32, 0, 32, 32];
    let mut moon = Vec::new();
    push_quad(&mut moon, -100.0, 20.0, rect, true, [1.0; 4]);
    let uv_at = |v: &[SkyVertex], x: f32, z: f32| {
        v.iter()
            .find(|p| p.pos[0] == x && p.pos[2] == z)
            .map(|p| p.uv)
            .unwrap()
    };
    assert_eq!(uv_at(&moon, -20.0, -20.0), [64.0, 0.0]);
    assert_eq!(uv_at(&moon, 20.0, -20.0), [32.0, 0.0]);
    assert_eq!(uv_at(&moon, 20.0, 20.0), [32.0, 32.0]);
    assert_eq!(uv_at(&moon, -20.0, 20.0), [64.0, 32.0]);
    let mut sun = Vec::new();
    push_quad(&mut sun, 100.0, 30.0, [0, 0, 32, 32], false, [1.0; 4]);
    assert_eq!(uv_at(&sun, -30.0, -30.0), [0.0, 0.0]);
    assert_eq!(uv_at(&sun, 30.0, 30.0), [32.0, 32.0]);
}

#[test]
fn dark_disc_below_the_horizon_out_of_water() {
    use super::dark_disc::*;
    assert_eq!(horizon_height(false, -64), 63.0);
    assert_eq!(horizon_height(true, -64), -64.0);
    assert!(should_render_dark_disc(62.9, 63.0, false));
    assert!(!should_render_dark_disc(63.0, 63.0, false));
    assert!(!should_render_dark_disc(40.0, 63.0, true));
    let fan = dark_disc_positions();
    assert!(fan.iter().all(|p| p[1] == -4.0));
    // The ring starts at a = -180: x = 512, z = 0.
    assert!((fan[1][0] - 512.0).abs() < 1e-3 && fan[1][2].abs() < 1e-3);
    assert_eq!(fan[0], [0.0, -4.0, 0.0]);
}
