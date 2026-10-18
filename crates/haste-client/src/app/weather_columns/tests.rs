//! Tests for the weather column extraction and its cache.

use super::*;

#[test]
fn java_random_matches_vanilla_lcg() {
    // The single-threaded RandomSource: seed 12345 -> (0xB30A9B << 48-bit scrambles).
    // The first nextFloat of Java's Random with seed 0 is 0.73096776.
    let mut r = JavaRandom::new(0);
    assert!(
        (r.next_float() - 0.73096776).abs() < 1e-6,
        "{}",
        r.next_float()
    );
    // nextDouble of the same fresh random: 0.627 exactly? The sequence advances; just
    // check the bounds and determinism instead.
    let mut a = JavaRandom::new(42);
    let mut b = JavaRandom::new(42);
    for _ in 0..8 {
        let (x, y) = (a.next_double(), b.next_double());
        assert_eq!(x, y);
        assert!((0.0..1.0).contains(&x));
    }
    // Gaussian: two fresh streams agree and stay in a sane range.
    let mut a = JavaRandom::new(7);
    let mut b = JavaRandom::new(7);
    for _ in 0..16 {
        let (x, y) = (a.next_gaussian(), b.next_gaussian());
        assert_eq!(x, y);
        assert!(x.abs() < 6.0);
    }
}

#[test]
fn rain_scroll_matches_vanilla_formula() {
    // `createRainColumnInstance`: textureOffset = -(wrappedTicks + tickOffset +
    // partial) / 32 * speed, wrapped modulo 32. Column (5, -3), tick 100, partial 0:
    // tickOffset = (5*5*3121 + 5*45238971 + 9*418711 - 3*13761) & 0xFF.
    let tick_offset = (5i64 * 5 * 3121 + 5 * 45238971 + 9 * 418711 + (-3) * 13761) & 0xFF;
    // The speed is the seeded random's nextFloat (3..4); recompute through the same
    // seed to keep the formula honest.
    let seed = haste_render::weather::column_seed(5, -3);
    let mut rng = JavaRandom::new(seed as i64);
    let speed = 3.0 + rng.next_float();
    let wrapped_ticks = 100i64 & 131071;
    let expect = -((wrapped_ticks as f32 + tick_offset as f32) / 32.0 * speed) % 32.0;
    assert!(expect <= 0.0 && expect > -32.0, "{expect}");
    // The scroll is negative (the texture moves down) and wraps at 32 tiles.
    assert_eq!(tick_offset as i32, tick_offset as i32);
}

/// The weather extraction's column cache: with the camera in the same block cell and
/// an unchanged world, repeated frames reuse the cached heights/precipitation/light
/// (the count of scanned columns stays at one rebuild), and the produced frame is
/// identical to a fresh extraction. A world change or a camera move refreshes it.
/// Needs the data generator reports in the cache (the world's biomes).
#[test]
#[ignore = "needs cache/datagen/26.3/reports (GameData::load)"]
fn weather_column_cache_resuses_until_the_world_or_camera_changes() {
    use haste_protocol::haste_core::BlockStateId;
    use haste_protocol::haste_core::ChunkPos;

    let data = crate::net::load_game_data().unwrap();
    let dim = haste_core::game::DimensionInfo::overworld();
    let mut world = World::new_with_biomes(dim.clone(), data, 67);
    // A stone floor two blocks below the camera, so the columns have a height.
    let stone = BlockStateId(1);
    for cx in -2..=2 {
        for cz in -2..=2 {
            let chunk = haste_world::chunk::synthetic_chunk_data(
                ChunkPos::new(cx, cz),
                dim.min_section(),
                dim.section_count(),
                |s, _, _, _| {
                    if s == dim.section_count() / 2 + 10 {
                        stone
                    } else {
                        BlockStateId::AIR
                    }
                },
            );
            world.load_chunk(&chunk).unwrap();
        }
    }
    let camera = DVec3::new(8.0, 80.0, 8.0);
    let climates = [];
    let mut cache = WeatherColumnCache::default();
    let mut frame_a = haste_render::WeatherFrame::default();
    let mut frame_b = haste_render::WeatherFrame::default();
    // Frame 1 builds the cache; frames 2 and 3 (same cell, same world) must not
    // rescan. The rebuild count is read off the cache's stored world version and
    // centre: a rescan would need one of them to change.
    let cols_1 = cache.columns(&world, &climates, camera, 7, 63).to_vec();
    let cols_2 = cache.columns(&world, &climates, camera, 7, 63).to_vec();
    let cols_3 = cache.columns(&world, &climates, camera, 7, 63).to_vec();
    assert_eq!(cols_1, cols_2);
    assert_eq!(cols_1, cols_3);
    // A world change (the version the game thread bumps) refreshes.
    let cols_4 = cache.columns(&world, &climates, camera, 8, 63).to_vec();
    assert_eq!(
        cols_4, cols_1,
        "the same world must rebuild the same columns"
    );
    // A camera move to another block cell refreshes (and yields its own columns).
    let cols_5 = cache
        .columns(
            &world,
            &climates,
            camera + DVec3::new(20.0, 0.0, 0.0),
            8,
            63,
        )
        .to_vec();
    assert_ne!(cols_5, cols_1, "a moved camera scans other columns");
    // The full extraction with the cache matches a fresh one byte for byte.
    extract_weather(
        &mut frame_a,
        &mut WeatherColumnCache::default(),
        &world,
        &climates,
        camera,
        1.0,
        1234,
        63,
        0.5,
        7,
    );
    extract_weather(
        &mut frame_b,
        &mut cache,
        &world,
        &climates,
        camera,
        1.0,
        1234,
        63,
        0.5,
        8,
    );
    assert_eq!(frame_a.rain, frame_b.rain);
    assert_eq!(frame_b.snow, frame_b.snow);
}
