use super::timeline::argb;
use super::*;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| close(*x, y))
}

fn inputs() -> FogInputs {
    FogInputs::default()
}

#[test]
fn fog_distances_by_render_distance() {
    // rd 32: 512 blocks, span 512/10 = 51.2 -> start 460.8; sky end 512.
    let f = setup_fog(32, &inputs());
    assert_eq!(f.render_distance_end, 512.0);
    assert_eq!(f.render_distance_start, 460.8);
    assert_eq!(f.sky_end, 512.0);
    // rd 12: 192 blocks, span 19.2 -> start 172.8.
    let f = setup_fog(12, &inputs());
    assert_eq!(f.render_distance_end, 192.0);
    assert_eq!(f.render_distance_start, 172.8);
    assert_eq!(f.sky_end, 192.0);
    // rd 2: 32 blocks, span clamped up to 4 -> start 28.
    let f = setup_fog(2, &inputs());
    assert_eq!(f.render_distance_end, 32.0);
    assert_eq!(f.render_distance_start, 28.0);
    assert_eq!(f.sky_end, 32.0);
    // Environmental band is the 0..1024 overworld default everywhere.
    assert_eq!(f.environmental_start, 0.0);
    assert_eq!(f.environmental_end, 1024.0);
}

#[test]
fn fog_value_is_max_of_spherical_and_cylindrical_bands() {
    let f = setup_fog(12, &inputs());
    // On the render distance band: cylindrical distance drives it.
    assert!(close(fog_value(180.0, 185.0, &f), (185.0 - 172.8) / 19.2));
    // Spherical beyond the environmental end saturates.
    assert_eq!(fog_value(1030.0, 0.0, &f), 1.0);
    // The environmental band starts at 0: any spherical distance has slight fog.
    assert!(close(fog_value(10.0, 10.0, &f), 10.0 / 1024.0));
    assert_eq!(fog_value(0.0, 0.0, &f), 0.0);
    // At the render distance end: full fog.
    assert_eq!(fog_value(500.0, 192.0, &f), 1.0);
}

#[test]
fn fog_colour_noon_rd12() {
    let c = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
    // Day: no night multiplier, no sunrise tint; mixed toward sky by the rd-12 factor
    // (1 - (0.25 + 0.75*0.375)^0.25).
    let mix = 1.0 - (0.25 + 0.75f32 * 12.0 / 32.0).powf(0.25);
    let expect: Vec<f32> = (0..3)
        .map(|i| OVERWORLD_FOG_COLOR[i] + (OVERWORLD_SKY_COLOR[i] - OVERWORLD_FOG_COLOR[i]) * mix)
        .collect();
    assert!(close3(c, [expect[0], expect[1], expect[2]]), "{c:?}");
}

#[test]
fn fog_colour_rd_variants_at_noon() {
    let noon = |rd: i32| atmospheric_fog_color(6000, rd, [0.0, 0.0, -1.0], 64.0, -64.0);
    // rd 32: no sky mix at all (sky fog end caps the mix factor at 1).
    let c = noon(32);
    assert!(close3(c, OVERWORLD_FOG_COLOR), "{c:?}");
    // rd 2: more sky mix than rd 12.
    let c2 = noon(2);
    let c12 = noon(12);
    let dist2 = (c2[0] - OVERWORLD_FOG_COLOR[0]).abs();
    let dist12 = (c12[0] - OVERWORLD_FOG_COLOR[0]).abs();
    assert!(dist2 > dist12, "shorter render distance mixes more sky");
}

#[test]
fn fog_colour_midnight_rd12() {
    let c = atmospheric_fog_color(18000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
    // Night: fog 0xC0D8FF * (0.06, 0.06, 0.09), sky black.
    let night_fog = [
        OVERWORLD_FOG_COLOR[0] * 0.06,
        OVERWORLD_FOG_COLOR[1] * 0.06,
        OVERWORLD_FOG_COLOR[2] * 0.09,
    ];
    let mix = 1.0 - (0.25 + 0.75f32 * 12.0 / 32.0).powf(0.25);
    let expect: Vec<f32> = (0..3)
        .map(|i| night_fog[i] + (0.0 - night_fog[i]) * mix)
        .collect();
    assert!(close3(c, [expect[0], expect[1], expect[2]]), "{c:?}");
}

#[test]
fn fog_colour_sunset_look_direction() {
    // t=12300: sunset, glow in the west (-x). Looking west tints; looking east does not.
    let west = atmospheric_fog_color(12300, 12, [-1.0, 0.0, 0.0], 64.0, -64.0);
    let east = atmospheric_fog_color(12300, 12, [1.0, 0.0, 0.0], 64.0, -64.0);
    assert!(
        west[0] > east[0],
        "looking at the sunset is redder: {west:?} {east:?}"
    );
    assert_eq!(sun_glow_side(12300), -1.0);
    assert_eq!(sun_glow_side(300), 1.0);
    // rd < 4: no tint even looking straight at the sun.
    let low = atmospheric_fog_color(12300, 2, [-1.0, 0.0, 0.0], 64.0, -64.0);
    let low_east = atmospheric_fog_color(12300, 2, [1.0, 0.0, 0.0], 64.0, -64.0);
    assert!(close3(low, low_east));
}

#[test]
fn fog_colour_takes_the_weather_layers() {
    // The FOG_COLOR weather layers (WeatherAttributes RAIN/THUNDER, MULTIPLY_RGB):
    // rain (0.5, 0.5, 0.6) by rain minus thunder, thunder (0.25, 0.25, 0.3) by thunder —
    // applied to the base fog colour before the sky mix, exactly where the attribute
    // system delivers them. Full rain, no thunder: fog * (0.5, 0.5, 0.6).
    let rain_fog = |inputs: &FogInputs| {
        compute_fog_color(inputs, 6000, 32, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0)
    };
    let mut inputs = FogInputs::default();
    let clear = rain_fog(&inputs);
    inputs.rain_level = 1.0;
    let rain = rain_fog(&inputs);
    for i in 0..3 {
        let m = [0.5, 0.5, 0.6][i];
        assert!(
            close(rain[i], OVERWORLD_FOG_COLOR[i] * m),
            "channel {i}: {}",
            rain[i]
        );
    }
    // rd 32: no sky mix, so the whole colour is the layered fog. Clear is the base.
    assert!(close3(clear, OVERWORLD_FOG_COLOR));
    // Full thunder (rain 1, thunder 1): the rain layer's weight is 0, the thunder layer
    // multiplies (0.25, 0.25, 0.3) — darker than rain.
    inputs.thunder_level = 1.0;
    let thunder = rain_fog(&inputs);
    for i in 0..3 {
        let m = [0.25, 0.25, 0.3][i];
        assert!(
            close(thunder[i], OVERWORLD_FOG_COLOR[i] * m),
            "channel {i}: {}",
            thunder[i]
        );
        assert!(thunder[i] < rain[i], "thunder darkens channel {i}");
    }
}

#[test]
fn sunrise_colour_timeline_extremes() {
    // Night: alpha 0. Plain day: the 730..11270 plateau.
    let night = sunrise_sunset_color(18000);
    assert_eq!(night[3], 0.0);
    let day = sunrise_sunset_color(6000);
    let plateau = argb(16770355);
    assert!(close3(
        [day[0], day[1], day[2]],
        [plateau[0], plateau[1], plateau[2]]
    ));
    // Peak alpha somewhere in the 12300-13300 evening range.
    let peak = sunrise_sunset_color(12732);
    assert!(peak[3] > 0.9, "{}", peak[3]);
    // Ramps are linear: halfway along the 12243..12358 segment is the tick average.
    let a = sunrise_sunset_color(12243 + 57);
    let b0 = argb((-1310226637i32) as u32);
    let b1 = argb((-857440717i32) as u32);
    for i in 0..4 {
        assert!(close(a[i], b0[i] + (b1[i] - b0[i]) * 57.0 / 115.0), "{a:?}");
    }
}

#[test]
fn timeline_wraps_the_day() {
    // At t=0 (before the first keyframe tick 71): the wrap segment from the last
    // keyframe (23757) to the first (71) one period later.
    let v = sunrise_sunset_color(0);
    let first = argb(1609540403);
    let last = argb((-1310226637i32) as u32);
    let span = 24000 + 71 - 23757;
    for i in 0..4 {
        let expect = last[i] + (first[i] - last[i]) * ((24000 - 23757) as f32 / span as f32);
        assert!(close(v[i], expect), "{v:?}");
    }
    // At the first keyframe's tick: its value.
    let v = sunrise_sunset_color(71);
    assert!(close3([v[0], v[1], v[2]], [first[0], first[1], first[2]]));
    assert!(close(v[3], first[3]));
}

#[test]
fn void_darkness_below_the_world() {
    let above = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
    let below = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], -96.0, -64.0);
    // 32 below min build height: full darkness squared.
    assert!(below.iter().all(|&c| close(c, 0.0)), "{below:?}");
    assert!(!above.iter().all(|&c| close(c, 0.0)));
}

// ---- the remaining environments (vanilla `environment/*FogEnvironment`) ----

#[test]
fn water_fog_distances_and_colour() {
    let mut f = inputs();
    f.fog_type = FogType::Water;
    f.water_fog_start = -8.0;
    f.water_fog_end = 96.0;
    f.water_vision = 0.5;
    let d = setup_fog(12, &f);
    assert_eq!(d.environment, FogEnvironment::Water);
    // `WaterFogEnvironment.setupFog`: end = 96 * max(0.25, 0.5) = 48; sky/cloud = end.
    assert_eq!(d.environmental_start, -8.0);
    assert_eq!(d.environmental_end, 48.0);
    assert_eq!(d.sky_end, 48.0);
    assert_eq!(d.cloud_end, 48.0);
    // Water vision below 0.25 clamps the ramp to a quarter.
    f.water_vision = 0.1;
    assert_eq!(setup_fog(12, &f).environmental_end, 24.0);
    // Colour is the biome water fog colour, brightened by the water vision.
    f.water_vision = 0.0;
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    assert!(close3(c, WATER_FOG_COLOR), "{c:?}");
    // Brightening lerps toward the normalised colour (max channel -> 1).
    f.water_vision = 1.0;
    let bright = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    let scale = 1.0
        / WATER_FOG_COLOR[0]
            .max(WATER_FOG_COLOR[1])
            .max(WATER_FOG_COLOR[2]);
    assert!(close3(
        bright,
        [
            WATER_FOG_COLOR[0] * scale,
            WATER_FOG_COLOR[1] * scale,
            WATER_FOG_COLOR[2] * scale
        ]
    ));
}

#[test]
fn lava_fog_distances_and_colour() {
    let mut f = inputs();
    f.fog_type = FogType::Lava;
    // `LavaFogEnvironment.setupFog` default: 0.25..1.
    let d = setup_fog(12, &f);
    assert_eq!(d.environment, FogEnvironment::Lava);
    assert_eq!(d.environmental_start, 0.25);
    assert_eq!(d.environmental_end, 1.0);
    // Fire resistance: 0..5.
    f.fire_resistance = true;
    let d = setup_fog(12, &f);
    assert_eq!(d.environmental_start, 0.0);
    assert_eq!(d.environmental_end, 5.0);
    // Spectator: -8..render_distance/2.
    f.spectator = true;
    let d = setup_fog(12, &f);
    assert_eq!(d.environmental_start, -8.0);
    assert_eq!(d.environmental_end, 96.0);
    // Colour is the fixed lava colour; the void darkness term is skipped in lava.
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], -96.0, -64.0);
    assert!(close3(c, LAVA_FOG_COLOR), "{c:?}");
}

#[test]
fn powder_snow_fog_distances_and_colour() {
    let mut f = inputs();
    f.fog_type = FogType::PowderSnow;
    // `PowderedSnowFogEnvironment.setupFog`: 0..2.
    let d = setup_fog(12, &f);
    assert_eq!(d.environment, FogEnvironment::PowderSnow);
    assert_eq!(d.environmental_start, 0.0);
    assert_eq!(d.environmental_end, 2.0);
    assert_eq!(d.sky_end, 2.0);
    // Spectator: -8..render_distance/2.
    f.spectator = true;
    let d = setup_fog(12, &f);
    assert_eq!(d.environmental_start, -8.0);
    assert_eq!(d.environmental_end, 96.0);
    // Colour is the fixed powder-snow colour; the void darkness term is skipped.
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], -96.0, -64.0);
    assert!(close3(c, POWDER_SNOW_FOG_COLOR), "{c:?}");
}

#[test]
fn blindness_fog_distances_and_darkness() {
    let mut f = inputs();
    // `BlindnessFogEnvironment.setupFog`: an infinite effect is a flat 5 blocks.
    f.blindness = Some(-1);
    let d = setup_fog(12, &f);
    assert_eq!(d.environment, FogEnvironment::Blindness);
    assert_eq!(d.environmental_start, 1.25);
    assert_eq!(d.environmental_end, 5.0);
    assert_eq!(d.sky_end, 4.0);
    // A 10-tick effect: lerp(min(1, 10/20), render_distance, 5) = lerp(0.5, 192, 5) = 98.5.
    f.blindness = Some(10);
    let d = setup_fog(12, &f);
    assert!(close(d.environmental_end, 98.5), "{}", d.environmental_end);
    // Darkness: full while the effect runs, the remaining fraction in the last second.
    assert_eq!(blindness_darkness(-1, 0.0), 1.0);
    assert_eq!(blindness_darkness(100, 0.0), 1.0);
    assert!(close(blindness_darkness(10, 0.0), 0.5));
    // The colour stays the atmosphere's (blindness does not provide a colour) but is
    // darkened to black by the full effect.
    f.blindness = Some(-1);
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    assert!(c.iter().all(|&v| close(v, 0.0)), "{c:?}");
}

#[test]
fn darkness_fog_distances_and_darkness() {
    let mut f = inputs();
    f.darkness = Some(400);
    f.darkness_blend = 1.0;
    // `DarknessFogEnvironment.setupFog`: lerp(blend, render_distance, 15); start 0.75d.
    let d = setup_fog(12, &f);
    assert_eq!(d.environment, FogEnvironment::Darkness);
    assert_eq!(d.environmental_start, 11.25);
    assert_eq!(d.environmental_end, 15.0);
    assert_eq!(d.sky_end, 15.0);
    // Halfway blend: lerp(0.5, 192, 15) = 103.5.
    f.darkness_blend = 0.5;
    let d = setup_fog(12, &f);
    assert!(close(d.environmental_end, 103.5), "{}", d.environmental_end);
    // The colour is the atmosphere's darkened by the blend factor.
    f.darkness_blend = 0.5;
    let base = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    let brightness = (1.0 - 0.5f32) * (1.0 - 0.5);
    assert!(close3(
        c,
        [
            base[0] * brightness,
            base[1] * brightness,
            base[2] * brightness
        ]
    ));
}

#[test]
fn blindness_wins_over_darkness() {
    // Vanilla's list order puts blindness before darkness: with both, blindness sets the
    // distances and its full darkness.
    let mut f = inputs();
    f.blindness = Some(-1);
    f.darkness = Some(400);
    f.darkness_blend = 0.0;
    assert_eq!(first_applicable(&f), FogEnvironment::Blindness);
    assert_eq!(setup_fog(12, &f).environmental_end, 5.0);
}

#[test]
fn effect_environment_overrides_water_distances_but_not_colour() {
    // Underwater with blindness: blindness sets the distances, water provides the colour.
    let mut f = inputs();
    f.fog_type = FogType::Water;
    f.water_fog_color = [0.1, 0.2, 0.3];
    f.blindness = Some(-1);
    let d = setup_fog(12, &f);
    assert_eq!(d.environment, FogEnvironment::Blindness);
    assert_eq!(d.environmental_end, 5.0);
    // Full blindness darkens the water colour to black.
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    assert!(c.iter().all(|&v| close(v, 0.0)), "{c:?}");
}

#[test]
fn nether_dimension_fog() {
    let mut f = inputs();
    f.dimension = DimensionFog::nether();
    // `DimensionTypes` nether: `FOG_START_DISTANCE` 10, `FOG_END_DISTANCE` 96.
    let d = setup_fog(12, &f);
    assert_eq!(d.environmental_start, 10.0);
    assert_eq!(d.environmental_end, 96.0);
    // No day timeline: the fog colour is the biome colour mixed toward the (black) sky.
    f.dimension = f.dimension.with_biome(&BiomeFog {
        fog_color: Some(0x330808),
        ..Default::default()
    });
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 70.0, 0.0);
    let sky_fog_end = (512.0 / 16.0f32).min(12.0);
    let mix = 1.0 - clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0).powf(0.25);
    let base = rgb24(0x330808);
    let expect = [
        base[0] + (0.0 - base[0]) * mix,
        base[1] + (0.0 - base[1]) * mix,
        base[2] + (0.0 - base[2]) * mix,
    ];
    assert!(close3(c, expect), "{c:?} {expect:?}");
}

#[test]
fn end_dimension_fog_colour() {
    let mut f = inputs();
    f.dimension = DimensionFog::end();
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, 0.0);
    // End fog colour mixed toward the black End sky.
    let base = rgb24(-15199464);
    let sky_fog_end = (512.0 / 16.0f32).min(12.0);
    let mix = 1.0 - clamped_lerp(sky_fog_end / 32.0, 0.25, 1.0).powf(0.25);
    let expect = [
        base[0] * (1.0 - mix),
        base[1] * (1.0 - mix),
        base[2] * (1.0 - mix),
    ];
    assert!(close3(c, expect), "{c:?} {expect:?}");
}

#[test]
fn boss_world_fog_clamps_the_atmosphere() {
    let mut f = inputs();
    f.boss_world_fog = true;
    let d = setup_fog(12, &f);
    // `AtmosphericFogEnvironment.setupFog` with `shouldCreateWorldFog`: start <= 10,
    // end <= 96, sky/cloud = end.
    assert_eq!(d.environmental_start, 0.0);
    assert_eq!(d.environmental_end, 96.0);
    assert_eq!(d.sky_end, 96.0);
    assert_eq!(d.cloud_end, 96.0);
}

#[test]
fn boss_darken_world_amount() {
    let mut f = inputs();
    f.darken_world_amount = 1.0;
    let base = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    assert!(close3(c, [base[0] * 0.7, base[1] * 0.6, base[2] * 0.6]));
}

#[test]
fn night_vision_brightens_but_not_with_darkness() {
    let mut f = inputs();
    f.night_vision = Some(-1);
    let base = atmospheric_fog_color(6000, 12, [0.0, 0.0, -1.0], 64.0, -64.0);
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    let scale = 1.0 / base[0].max(base[1]).max(base[2]);
    assert!(close3(
        c,
        [base[0] * scale, base[1] * scale, base[2] * scale]
    ));
    // With darkness too, the night-vision branch is skipped.
    f.darkness = Some(400);
    f.darkness_blend = 0.0;
    let c = compute_fog_color(&f, 6000, 12, 0.0, [0.0, 0.0, -1.0], 64.0, -64.0);
    assert!(close3(c, base));
}
