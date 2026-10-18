use super::*;
use haste_core::Direction;

#[test]
fn destroy_spawns_the_vanilla_grid() {
    let mut particles = Particles::new(types_with_blocks());
    particles.spawn_destroy(
        BlockPos::new(10, 64, -5),
        BlockStateId(0),
        &full_cube_shapes(),
        &OneBiome,
    );
    // A full cube: ceil(1.0/0.25) = 4 per axis, at least 2 -> 4x4x4 = 64 particles
    // (vanilla `addDestroyBlockEffect`'s density 0.25 grid).
    assert_eq!(particles.alive(), 64);
    for p in &particles.pool {
        // TerrainParticle: gravity 1, quad halved from the 0.1..=0.2 default. The
        // colour is the grey 0.6 as a linear vertex multiplier (the entity pipeline
        // multiplies linearized texels; vanilla multiplies encoded ones).
        assert_eq!(p.behaviour.gravity, 1.0);
        assert!((0.05..=0.1).contains(&p.quad), "quad {}", p.quad);
        assert_eq!(p.color, crate::sky::linear([0.6, 0.6, 0.6]));
        // The speed constructor's output: the vector rescaled to a length of
        // (r+r+1)*0.15*0.4 = 0.06..<0.18 (the ±0.375 grid offsets only pick its
        // direction), then y lifted 0.1 — so y sits in 0.1 ± 0.18.
        let horizontal = (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt();
        assert!(horizontal < 0.18, "horizontal {horizontal}");
        assert!(
            p.vel.y > 0.1 - 0.18 && p.vel.y < 0.1 + 0.18,
            "y {}",
            p.vel.y
        );
        // The lifetime is Particle's default 4..=40.
        assert!((4.0..=40.0).contains(&p.lifetime), "life {}", p.lifetime);
        // The uv sub-piece: one 4x4 texel quarter of the 16x16 sprite.
        let sprite = p.block_sprite.expect("terrain particle");
        let [u0, v0, u1, v1] = sprite.uv;
        assert!((u1 - u0 - 4.0).abs() < 1e-3, "u span {}", u1 - u0);
        assert!((v1 - v0 - 4.0).abs() < 1e-3, "v span {}", v1 - v0);
        assert!((0.0..=12.0).contains(&u0), "u0 {u0}");
        assert!((0.0..=12.0).contains(&v0), "v0 {v0}");
    }
    // Every grid cell position appears once: the 64 rel centres of a 4x4x4 grid
    // over the unit cube are (i+0.5)/4, i in 0..4, per axis.
    let mut cells = std::collections::HashSet::new();
    for p in &particles.pool {
        let rel = [p.pos.x - 10.0, p.pos.y - 64.0, p.pos.z + 5.0];
        for v in rel {
            assert!([0.125, 0.375, 0.625, 0.875].contains(&v), "cell {rel:?}");
        }
        let cell: [u64; 3] = rel.map(|v| (v * 8.0) as u64);
        cells.insert(cell);
    }
    assert_eq!(cells.len(), 64);
}

/// The terrain-particle tint is a linear-RGB vertex colour, not the sRGB-scale
/// product vanilla feeds its shader: vanilla multiplies the sprite's encoded texels
/// by the encoded 0.6 x tint (`TerrainParticle`'s constructor), while the particle pass
/// runs the entity pipeline, which multiplies linearized texels — handing it the
/// encoded-scale product made tinted particles brighter and hue-shifted against the
/// blocks they came from (the leaves-particle bug). Birch's tint is a fixed constant,
/// oak's the biome foliage colour, so no colormaps are needed.
#[test]
fn terrain_tint_is_a_linear_vertex_colour() {
    // Birch leaves: BlockTintSources.constant(-8345771) = 0x80A755.
    let birch = crate::block_particles::BlockParticleSprite {
        rect: [0, 0, 16, 16],
        tint: crate::tint::TintKind::Fixed(0x80A755),
        spawn: true,
        upper_half_tint_below: false,
    };
    // Oak leaves: the foliage colour of the one biome around the position.
    let oak = crate::block_particles::BlockParticleSprite {
        tint: crate::tint::TintKind::Foliage,
        ..birch
    };
    let mut climate = crate::tint::BiomeClimate::plains();
    climate.foliage_color = Some(0x48B518);
    let colors =
        crate::tint::BiomeColors::new(&[climate], &crate::tint::Colormaps::uniform(0xFFFFFF));
    let linear = |c: u32| {
        crate::sky::linear([
            0.6 * ((c >> 16 & 0xFF) as f32 / 255.0),
            0.6 * ((c >> 8 & 0xFF) as f32 / 255.0),
            0.6 * ((c & 0xFF) as f32 / 255.0),
        ])
    };
    assert_eq!(
        crate::block_particles::terrain_tint(&birch, BlockPos::new(0, 64, 0), &OneBiome, &colors),
        linear(0x80A755)
    );
    assert_eq!(
        crate::block_particles::terrain_tint(&oak, BlockPos::new(0, 64, 0), &OneBiome, &colors),
        linear(0x48B518)
    );
    // The conversion is load-bearing: linearizing 0.6 darkens it to ~0.319.
    assert!((0.31..0.33).contains(&crate::sky::linear([0.6, 0.6, 0.6])[0]));
}

#[test]
fn destroy_skips_states_without_terrain_particles() {
    // No block table: nothing spawns.
    let mut particles = Particles::new(types_for_tests(&[("flame", &[[0, 0, 8, 8]])]));
    particles.spawn_destroy(
        BlockPos::new(10, 64, -5),
        BlockStateId(0),
        &full_cube_shapes(),
        &OneBiome,
    );
    assert_eq!(particles.alive(), 0);
    // The pool cap applies like every spawn.
    let mut particles = Particles::new(types_with_blocks());
    for i in 0..(MAX_PARTICLES / 64 + 8) as i32 {
        particles.spawn_destroy(
            BlockPos::new(10 + i, 64, -5),
            BlockStateId(0),
            &full_cube_shapes(),
            &OneBiome,
        );
    }
    assert_eq!(particles.alive(), MAX_PARTICLES);
}

#[test]
fn crack_sits_on_the_hit_face_with_power_scaled_velocity() {
    let mut particles = Particles::new(types_with_blocks());
    particles.spawn_crack(
        BlockPos::new(10, 64, -5),
        Direction::North,
        BlockStateId(0),
        &full_cube_shapes(),
        &OneBiome,
    );
    assert_eq!(particles.alive(), 1);
    let p = &particles.pool[0];
    // North face: z = min - 0.1, one block outside the front of the cube; x
    // and y stay within the (0.1 .. 0.9) inner span of the bounds.
    assert!((p.pos.z - (-5.0 + 0.0 - 0.1)).abs() < 1e-9, "z {:?}", p.pos);
    assert!(p.pos.x > 10.0 && p.pos.x < 11.0, "x {:?}", p.pos);
    assert!(p.pos.y > 64.0 && p.pos.y < 65.0, "y {:?}", p.pos);
    // setPower(0.2): the horizontal velocity at most a fifth of the constructor's
    // 0.06..=0.12 range, y within 0.1 ± 0.024.
    assert!(p.vel.x.abs() <= 0.12 * 0.2 + 1e-6, "vx {}", p.vel.x);
    assert!(p.vel.z.abs() <= 0.12 * 0.2 + 1e-6, "vz {}", p.vel.z);
    assert!((p.vel.y - 0.1).abs() <= 0.12 * 0.2 + 1e-6, "vy {}", p.vel.y);
    // scale(0.6) on the halved quad: 0.6 * (0.1..=0.2) / 2.
    assert!((0.03..=0.06).contains(&p.quad), "quad {}", p.quad);

    // The other faces put the particle outside their own side of the cube.
    type FaceAxis = fn(&ParticleState) -> f32;
    let cases: [(Direction, FaceAxis); 5] = [
        (Direction::Down, |p| p.pos.y),
        (Direction::Up, |p| p.pos.y),
        (Direction::South, |p| p.pos.z),
        (Direction::West, |p| p.pos.x),
        (Direction::East, |p| p.pos.x),
    ];
    for (face, axis) in cases {
        let mut particles = Particles::new(types_with_blocks());
        particles.spawn_crack(
            BlockPos::new(10, 64, -5),
            face,
            BlockStateId(0),
            &full_cube_shapes(),
            &OneBiome,
        );
        let p = &particles.pool[0];
        let v = axis(p);
        let (lo, hi) = match face {
            Direction::Down => (63.9, 64.0),
            Direction::Up => (65.0, 65.1),
            Direction::South => (-4.0, -3.9),
            Direction::West => (9.9, 10.0),
            Direction::East => (11.0, 11.1),
            _ => unreachable!(),
        };
        assert!(
            v > lo - 1e-4 && v < hi + 1e-4,
            "{face:?}: {v} not in {lo}..{hi}"
        );
    }
}

/// The sprint particle: on the block below the feet (0.2 down), y + 0.1, the
/// horizontal jitter clamped into the support column, velocity the constructor
/// over `movement * -4` and +1.5 y.
#[test]
fn sprint_particle_matches_entity_spawn_sprint_particle() {
    let mut particles = Particles::new(types_with_blocks());
    let feet = DVec3::new(10.5, 64.0, -5.5);
    let velocity = DVec3::new(0.2, -0.078, 0.0);
    particles.spawn_sprint(
        feet,
        velocity,
        BlockStateId(0),
        &full_cube_shapes(),
        &OneBiome,
    );
    assert_eq!(particles.alive(), 1);
    let p = &particles.pool[0];
    // `getOnPosLegacy` at y=64 - 0.2 -> the block at 63; the particle sits at
    // the feet height + 0.1, within the support column (the feet are in it).
    assert!((p.pos.y - 64.1).abs() < 1e-6, "y {}", p.pos.y);
    assert!(p.pos.x > 10.0 && p.pos.x < 11.0, "x {}", p.pos.x);
    assert!(p.pos.z > -6.0 && p.pos.z < -5.0, "z {}", p.pos.z);
    // The speed constructor's bounds (see the destroy grid test), y lifted.
    assert!(
        p.vel.y > 0.1 - 0.18 && p.vel.y < 0.1 + 0.18,
        "vy {}",
        p.vel.y
    );
    let horizontal = (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt();
    assert!(horizontal < 0.18, "horizontal {horizontal}");
    // A full-cube quad (no scale): the halved default.
    assert!((0.05..=0.1).contains(&p.quad), "quad {}", p.quad);
}

/// The block sprite table over the real assets: every state of a few familiar
/// blocks resolves a 16x16 (or larger) sprite, barrier/structure_void refuse to
/// spawn, and grass_block's particles are untinted (vanilla's
/// `colorAsTerrainParticle` override) while leaves take the foliage tint.
#[test]
#[ignore = "needs the client jar and datagen reports in the cache"]
fn block_particle_table_resolves_real_sprites() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let types = ParticleTypes::build(&mut assets, &[], Some(&states), &[]).unwrap();
    let blocks = types.blocks.as_ref().unwrap();
    // The state id of `name` with `props` (BlocksJsonStates has no lookup, so scan).
    let find = |name: &str, props: &[(&str, &str)]| {
        use haste_core::view::BlockStateTable;
        let want: std::collections::HashMap<_, _> = props
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        (0..states.len() as u32)
            .map(BlockStateId)
            .find(|s| {
                states.block_of(*s).path() == name
                    && states
                        .properties(*s)
                        .into_iter()
                        .filter(|(k, _)| want.contains_key(k))
                        .eq(want.iter().map(|(k, v)| (k.clone(), v.clone())))
            })
            .unwrap_or_else(|| panic!("no state for {name} {props:?}"))
    };
    // Stone: a full 16x16 sprite, spawnable, untinted.
    let stone = find("stone", &[]);
    let s = blocks.sprite(stone).expect("stone has a particle sprite");
    assert!(s.spawn);
    assert_eq!(s.tint, crate::tint::TintKind::None);
    assert_eq!(s.rect[2], 16);
    assert_eq!(s.rect[3], 16);
    // Barrier breaks without terrain particles (vanilla `noTerrainParticles`).
    let barrier = find("barrier", &[]);
    assert!(!blocks.sprite(barrier).expect("sprite").spawn);
    // Grass block particles are WHITE (BlockTintSources.grassBlock overrides
    // colorAsTerrainParticle), oak leaves take the biome foliage colour.
    let grass = find("grass_block", &[]);
    assert_eq!(
        blocks.sprite(grass).unwrap().tint,
        crate::tint::TintKind::None
    );
    let leaves = find("oak_leaves", &[]);
    assert_eq!(
        blocks.sprite(leaves).unwrap().tint,
        crate::tint::TintKind::Foliage
    );
    // An upper tall grass reads the biome below (doubleTallGrass).
    let upper = find("tall_grass", &[("half", "upper")]);
    assert!(blocks.sprite(upper).unwrap().upper_half_tint_below);
    let lower = find("tall_grass", &[("half", "lower")]);
    assert!(!blocks.sprite(lower).unwrap().upper_half_tint_below);
    // Out-of-range ids resolve to the placeholder (stone), like the baked models.
    assert!(blocks.sprite(BlockStateId(u32::MAX - 5)).is_some());
}
