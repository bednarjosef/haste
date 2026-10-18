use super::*;

/// The frustum cull keeps entities inside the view and drops those behind the camera:
/// vanilla `EntityRenderer.shouldRender` (`Frustum.isVisible` on the inflated AABB).
/// A sphere around the model stands in for the AABB, so an entity half-in view stays.
/// The planes come from the renderer's own `frustum_planes` on a straight, level view.
#[test]
fn frustum_cull_keeps_visible_and_drops_behind() {
    let camera = crate::Camera {
        pos: glam::DVec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: crate::ViewBob::default(),
    };
    let view_proj = crate::Renderer::view_projection(&camera, 1.0, None);
    let planes = crate::Renderer::frustum_planes(&view_proj);
    let r = 1.0;
    // Straight ahead at 10 blocks (yaw 0 looks along +z): kept.
    assert!(aabb_in_frustum(
        &planes,
        glam::DVec3::new(0.0, 0.0, 10.0),
        r
    ));
    // Behind the camera: dropped.
    assert!(!aabb_in_frustum(
        &planes,
        glam::DVec3::new(0.0, 0.0, -10.0),
        r
    ));
    // Ahead but far outside the 70-degree fov: dropped.
    assert!(!aabb_in_frustum(
        &planes,
        glam::DVec3::new(100.0, 0.0, 10.0),
        r
    ));
}

/// A model's render size (vanilla `Entity.shouldRenderAtSqrDistance` compares against
/// `64 * boundingBox.getSize()`): an 8x8x8-unit cube spans half a block each way, so
/// its diagonal is ~0.87 blocks (clamped to 1) and the sphere radius is the
/// half-diagonal, ~0.43 blocks.
#[test]
fn model_extents_from_a_cube() {
    let part = ModelPart {
        name: "root".into(),
        id: parts::ROOT,
        pivot: Vec3::ZERO,
        rotation: Vec3::ZERO,
        visible: true,
        cubes: vec![haste_core::model::Cube {
            origin: Vec3::new(-4.0, 0.0, -4.0),
            size: Vec3::new(8.0, 8.0, 8.0),
            inflate: 0.0,
            uv: [0, 0],
            mirror: false,
        }],
        children: Vec::new(),
    };
    let (size, half) = model_extents(&part);
    let diagonal = (3.0_f32 * 8.0 * 8.0).sqrt();
    assert!(
        (size - (diagonal / 16.0).max(1.0)).abs() < 1e-5,
        "size {size}"
    );
    assert!((half - diagonal / 16.0 / 2.0).abs() < 1e-5, "half {half}");
}

/// `EntityRenderer.extractShadowPiece`: the shadow alpha fades with height above the
/// block (`powerAtDepth = pow - (entity.y - pos.y) * 0.5`, then `* 0.5 * brightness`).
#[test]
fn shadow_alpha_fades_with_height_above_the_block() {
    // pow = 1: no camera distance falloff and shadow strength 1; the entity's feet at y=1.5.
    let pow = 1.0f32;
    let feet = 1.5f32;
    let alpha_at = |block_y: i32| {
        let power_at_depth = pow - (feet - block_y as f32) * 0.5;
        shadow_piece_alpha(power_at_depth, lightmap_brightness(0.0, 15))
    };
    // The block under the feet is strongest, one lower is a quarter as strong, two lower
    // is negative and clamps to zero.
    assert!((alpha_at(1) - 0.375).abs() < 1e-6, "{}", alpha_at(1));
    assert!((alpha_at(0) - 0.125).abs() < 1e-6, "{}", alpha_at(0));
    assert_eq!(alpha_at(-1), 0.0);
    // The alpha is clamped to 1.
    assert_eq!(shadow_piece_alpha(4.0, 1.0), 1.0);
}

/// `Lightmap.getBrightness`: the curved `v / (4 - 3v)` curve with the dimension's ambient
/// light floor.
#[test]
fn lightmap_brightness_is_the_vanilla_curve() {
    assert_eq!(lightmap_brightness(0.0, 15), 1.0);
    assert_eq!(lightmap_brightness(0.0, 0), 0.0);
    // 7 / (15 * (4 - 3 * 7/15)) = 7/39.
    assert!((lightmap_brightness(0.0, 7) - 7.0 / 39.0).abs() < 1e-6);
    // Nether ambient 0.1 lifts every level towards 1.
    assert!((lightmap_brightness(0.1, 0) - 0.1).abs() < 1e-6);
}

/// `EntityRenderer.extractNameTags`/`LivingEntityRenderer.shouldShowName`: 64 blocks, 32
/// while crouching (distanceSq >= 1024 hides it).
#[test]
fn name_tag_culls_at_64_and_32_when_crouching() {
    assert!(name_tag_visible(63.9 * 63.9, false));
    assert!(!name_tag_visible(64.1 * 64.1, false));
    assert!(name_tag_visible(31.9 * 31.9, true));
    assert!(!name_tag_visible(32.0 * 32.0, true));
    assert!(!name_tag_visible(40.0 * 40.0, true));
    // Standing is not affected by the 32 limit.
    assert!(name_tag_visible(40.0 * 40.0, false));
}

/// `body_matrix` composes the ops left to right like `LivingEntityRenderer.setupRotations`
/// (`poseStack` calls): `rotateZ(90); translate(0, 2, 0)` maps the origin to (-2, 0, 0).
#[test]
fn body_matrix_applies_ops_in_vanilla_order() {
    let mut b = BodyTransform::default();
    b.push(BodyOp::RotateZ(std::f32::consts::FRAC_PI_2));
    b.push(BodyOp::Translate(Vec3::new(0.0, 2.0, 0.0)));
    let m = body_matrix(&b);
    let p = m.transform_point3(Vec3::ZERO);
    assert!((p.x - -2.0).abs() < 1e-5, "x {}", p.x);
    assert!(p.y.abs() < 1e-5, "y {}", p.y);
    // A single X rotation: (0, 1, 0) -> (0, 0, 1).
    let mut b = BodyTransform::default();
    b.push(BodyOp::RotateX(std::f32::consts::FRAC_PI_2));
    let p = body_matrix(&b).transform_point3(Vec3::Y);
    assert!((p.z - 1.0).abs() < 1e-5, "z {}", p.z);
}

/// Every texture haste-entity's model table names must exist in the client jar (the
/// renderer loads them on demand by identifier; a typo would draw the placeholder at
/// runtime with only a log line). Cross-crate on purpose: haste-render owns the jar.
#[test]
#[ignore = "needs the client jar in the cache"]
fn entity_table_textures_exist_in_the_jar() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    for texture in haste_entity::registry::table_textures() {
        let id = Identifier::minecraft(texture);
        assets
            .texture_any(&id)
            .unwrap_or_else(|e| panic!("table texture {texture}: {e:#}"));
    }
}

/// The item frame block models and the painting sprites this renderer loads must exist in
/// the client jar (a typo would only show as placeholder quads at runtime).
#[test]
#[ignore = "needs the client jar in the cache"]
fn frame_and_painting_assets_exist_in_the_jar() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    // `BlockModelResolver.updateForItemFrame` -> `item_frame` / `glow_item_frame`.
    for (block, map) in [
        ("item_frame", false),
        ("item_frame", true),
        ("glow_item_frame", false),
        ("glow_item_frame", true),
    ] {
        let def = assets
            .blockstate(&Identifier::minecraft(block))
            .unwrap_or_else(|e| panic!("blockstate {block}: {e:#}"));
        let refs = def.model_refs_for(&[("map".to_string(), map.to_string())]);
        assert!(!refs.is_empty(), "{block} map={map} has no model");
        for r in refs {
            let id = Identifier::parse(&r.model).unwrap();
            let quads = assets
                .model_quads(&id)
                .unwrap_or_else(|e| panic!("model {}: {e:#}", r.model));
            assert!(!quads.is_empty(), "{} has no quads", r.model);
            for q in &quads {
                assets
                    .texture_any(&parse_tex(&q.texture))
                    .unwrap_or_else(|e| panic!("frame texture {}: {e:#}", q.texture));
            }
        }
    }
    // The painting front sprite and the back sprite (the `paintings` atlas directory).
    for sprite in ["painting/back", "painting/match", "painting/kebab"] {
        assets
            .texture_any(&Identifier::minecraft(sprite))
            .unwrap_or_else(|e| panic!("painting texture {sprite}: {e:#}"));
    }
}

#[test]
fn a_cube_has_six_outward_ccw_faces() {
    let model = CubeModel {
        texture: Identifier::minecraft("x"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: vec![haste_core::model::Cube {
                origin: Vec3::new(-4.0, -8.0, -4.0),
                size: Vec3::splat(8.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            children: vec![],
        },
    };
    let mut out = Vec::new();
    let root = Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0);
    emit_part_tinted(
        &mut out,
        &model,
        &model.root,
        &PoseOverrides::default(),
        root,
        [0, 0, 64, 64],
        (15, 15),
        0.0,
        [1.0; 3],
    );
    assert_eq!(out.len(), 36);
    // Every triangle faces away from the cube centre (0, 0.25, 0).
    let centre = Vec3::new(0.0, 0.25, 0.0);
    for t in out.chunks(3) {
        let p: Vec<Vec3> = t.iter().map(|v| Vec3::from(v.pos)).collect();
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        let mid = (p[0] + p[1] + p[2]) / 3.0;
        assert!(n.dot(mid - centre) > 0.0, "inward triangle {p:?}");
    }
    // The front face (texture 8..16 x 8..16) stays at model -z (the root flips x and y).
    let front: Vec<&[EntityVertex]> = out
        .chunks(3)
        .filter(|t| {
            t.iter()
                .all(|v| (8.0..=16.0).contains(&v.uv[0]) && (8.0..=16.0).contains(&v.uv[1]))
        })
        .collect();
    assert_eq!(front.len(), 2);
    assert!(front
        .iter()
        .flat_map(|t| t.iter())
        .all(|v| (v.pos[2] + 0.25).abs() < 1e-5));
}

#[test]
fn hurt_mixes_red_into_the_vertex_colour() {
    // Untinted: the colour is the shade alone.
    assert_eq!(hurt_color(0.8, 0.0), [0.8, 0.8, 0.8]);
    // Fully hurt at vanilla's ~30 % overlay: red boosted (clamped at 1.0 here),
    // green/blue pulled down.
    let [r, g, b] = hurt_color(0.8, 1.0);
    assert_eq!(r, 1.0);
    assert!((g - 0.8 * (1.0 - HURT_RED_STRENGTH)).abs() < 1e-6);
    assert_eq!(g, b);
    // Monotone in hurt: the red channel grows, green shrinks.
    assert!(hurt_color(0.8, 0.25)[0] < hurt_color(0.8, 0.75)[0]);
    assert!(hurt_color(0.8, 0.25)[1] > hurt_color(0.8, 0.75)[1]);
}

#[test]
fn dropped_item_spin_is_age_over_20_plus_the_phase() {
    // `ItemEntity.getSpin`: radians, the age over 20 plus the entity's phase, so
    // one turn takes 40*pi ticks (about 125.7) — a slow spin.
    assert_eq!(item_entity_spin(0.0, 0.0), 0.0);
    assert_eq!(item_entity_spin(20.0, 0.0), 1.0);
    assert!((item_entity_spin(10.0, 0.0) - 0.5).abs() < 1e-6);
    assert!((item_entity_spin(0.0, 1.0) - 1.0).abs() < 1e-6);
    assert!((item_entity_spin(10.0, 1.0) - 1.5).abs() < 1e-6);
}

#[test]
fn dropped_item_bobs_a_tenth_of_a_block_every_20_pi_ticks() {
    // `ItemEntityRenderer.submit`: sin(age / 10 + phase) * 0.1 + 0.1, so the bob
    // spans 0..0.2 blocks with a 20*pi-tick period (about 62.8).
    assert!((item_entity_bob(0.0, 0.0) - 0.1).abs() < 1e-6);
    // sin tops out a quarter period in: age = 5*pi.
    assert!((item_entity_bob(5.0 * std::f32::consts::PI, 0.0) - 0.2).abs() < 1e-5);
    assert!((item_entity_bob(15.0 * std::f32::consts::PI, 0.0) - 0.0).abs() < 1e-5);
    // The phase shifts the whole curve.
    assert!((item_entity_bob(0.0, std::f32::consts::FRAC_PI_2) - 0.2).abs() < 1e-5);
    for age in [0.0f32, 7.5, 19.3, 33.7, 60.1] {
        let b = item_entity_bob(age, entity_phase(EntityId(7)));
        assert!((0.0..=0.2).contains(&b), "bob {b} at age {age}");
    }
}

#[test]
fn dropped_item_copies_follow_the_stack_size() {
    // `ItemClusterRenderState.getRenderedAmount`: 1 for a single item, then one
    // more per full 16 of the count, up to 5.
    assert_eq!(item_entity_copies(0), 1);
    assert_eq!(item_entity_copies(1), 1);
    assert_eq!(item_entity_copies(2), 2);
    assert_eq!(item_entity_copies(16), 2);
    assert_eq!(item_entity_copies(17), 3);
    assert_eq!(item_entity_copies(32), 3);
    assert_eq!(item_entity_copies(33), 4);
    assert_eq!(item_entity_copies(48), 4);
    assert_eq!(item_entity_copies(49), 5);
    assert_eq!(item_entity_copies(64), 5);
    assert_eq!(item_entity_copies(99), 5);
}

#[test]
fn dropped_item_lift_puts_the_bbox_bottom_a_sixteenth_up() {
    // `ItemEntityRenderer.submit` lifts by -bbox.minY + 1/16 (the bounding box is the
    // model through its ground display, so a block item reaching minY -0.125 needs
    // 0.1875 and a centred model spanning -0.5..0.5 needs 0.5625).
    assert!((item_entity_lift(-0.5) - 0.5625).abs() < 1e-6);
    assert!((item_entity_lift(-0.125) - 0.1875).abs() < 1e-6);
    assert!((item_entity_lift(0.0) - 0.0625).abs() < 1e-6);
    // A model that dips below 0 needs more lift; one that starts above 0, less.
    assert!(item_entity_lift(-0.25) > item_entity_lift(0.0));
    assert!(item_entity_lift(0.25) < item_entity_lift(0.0));
}

#[test]
fn entity_phase_and_seed_streams_cover_their_ranges() {
    // The phase covers [0, 2pi) across ids and is stable for one id.
    let phases: Vec<f32> = (0..200).map(|i| entity_phase(EntityId(i))).collect();
    assert!(phases
        .iter()
        .all(|p| (0.0..std::f32::consts::TAU).contains(p)));
    assert!(phases.iter().any(|p| *p > std::f32::consts::PI));
    assert!(phases.iter().any(|p| *p < std::f32::consts::PI));
    assert_eq!(entity_phase(EntityId(42)), entity_phase(EntityId(42)));
    // The seeded stream is in [0,1) and stable per (seed, index).
    for (seed, i) in [(0u32, 0u32), (5, 1), (1234, 7), (187, 3)] {
        let r = seeded_float(seed, i);
        assert!((0.0..1.0).contains(&r));
        assert_eq!(r, seeded_float(seed, i));
    }
    let values: Vec<f32> = (0..50).map(|i| seeded_float(9, i)).collect();
    assert!(values.iter().any(|r| *r < 0.33));
    assert!(values.iter().any(|r| *r > 0.66));
}

#[test]
fn head_top_of_the_humanoid_layout() {
    let part = ModelPart {
        name: "head".into(),
        id: parts::HEAD,
        pivot: Vec3::new(0.0, -24.0, 0.0),
        rotation: Vec3::ZERO,
        visible: true,
        cubes: vec![haste_core::model::Cube {
            origin: Vec3::new(-4.0, -8.0, -4.0),
            size: Vec3::splat(8.0),
            inflate: 0.0,
            uv: [0, 0],
            mirror: false,
        }],
        children: vec![],
    };
    assert_eq!(model_top(&part, 0.0), 32.0);
}

/// A hand with no animation: no swing, the equip progress up, no bob, no lag.
fn still_hand() -> FirstPerson {
    FirstPerson {
        skin: None,
        slim: false,
        sleeve: true,
        item: None,
        light: (15, 0),
        attack: TickPair::fixed(0.0),
        equip: TickPair::fixed(1.0),
        x_bob: TickPair::fixed(0.0),
        y_bob: TickPair::fixed(0.0),
    }
}

#[test]
fn tick_pair_lerps_the_two_ends_of_a_tick() {
    // `Mth.lerp(partial, start, end)`.
    let p = TickPair {
        start: 0.25,
        end: 0.75,
    };
    assert_eq!(p.at(0.0), 0.25);
    assert_eq!(p.at(0.5), 0.5);
    assert_eq!(p.at(1.0), 0.75);
    assert_eq!(TickPair::fixed(0.5).at(0.37), 0.5);
}

#[test]
fn no_bob_and_no_lag_is_the_identity() {
    // With nothing animated, `submitHandsWithItems`'s outer transforms change nothing.
    assert_eq!(
        hand_root_matrix(&still_hand(), (0.0, 0.0), 0.0),
        Mat4::IDENTITY
    );
}

#[test]
fn the_arm_sits_at_vanilla_rest_position() {
    // The arm's vertices through `renderPlayerArm`'s transform, the right arm's own
    // `PartPose` (-5, 2, 0) and its 0.1 rad z tilt: the hand ends up right of the view
    // centre, below it and in front of the camera, reaching down and forward.
    let m = arm_matrix(&still_hand(), 0.0) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
    let model = arm_model(false, true);
    let mut out = Vec::new();
    emit_part_tinted(
        &mut out,
        &model,
        &model.root,
        &PoseOverrides::default(),
        m,
        [0, 0, 64, 64],
        (15, 15),
        0.0,
        [1.0; 3],
    );
    // The arm and its sleeve, six faces of six vertices each.
    assert_eq!(out.len(), 72);
    let lo = out
        .iter()
        .map(|v| Vec3::from(v.pos))
        .reduce(Vec3::min)
        .unwrap();
    let hi = out
        .iter()
        .map(|v| Vec3::from(v.pos))
        .reduce(Vec3::max)
        .unwrap();
    println!("the bare arm's box: {lo:?} .. {hi:?}");
    assert!(lo.x > 0.0 && hi.x < 1.2, "x {lo:?} .. {hi:?}");
    assert!(hi.y < 0.0 && lo.y > -1.6, "y {lo:?} .. {hi:?}");
    assert!(hi.z < -0.3 && lo.z > -1.6, "z {lo:?} .. {hi:?}");
    // A swing moves the whole hand (every vertex moves, and the arm's box widens upwards).
    let mut swung_hand = still_hand();
    swung_hand.attack = TickPair::fixed(0.25);
    let swung = arm_matrix(&swung_hand, 0.0) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
    let mut out2 = Vec::new();
    emit_part_tinted(
        &mut out2,
        &arm_model(false, true),
        &arm_model(false, true).root,
        &PoseOverrides::default(),
        swung,
        [0, 0, 64, 64],
        (15, 15),
        0.0,
        [1.0; 3],
    );
    let moved = out
        .iter()
        .zip(&out2)
        .filter(|(a, b)| (Vec3::from(a.pos) - Vec3::from(b.pos)).length() > 1e-4)
        .count();
    assert_eq!(moved, out.len(), "the swing must move every vertex");
}

/// Compares two matrices element by within `tol`.
fn close(a: Mat4, b: Mat4, tol: f32) -> bool {
    a.to_cols_array_2d()
        .iter()
        .zip(b.to_cols_array_2d())
        .all(|(x, y)| x.iter().zip(y).all(|(x, y)| (x - y).abs() < tol))
}

/// `FirstPersonHandsAndItemsRenderer.renderPlayerArm`, transcribed pose-stack call by
/// pose-stack call (translate in blocks, rotations in degrees): the renderer's chain must
/// produce the same matrix.
fn vanilla_render_player_arm(attack: f32, inverse_arm_height: f32) -> Mat4 {
    let pi = std::f32::consts::PI;
    let sqrt_attack = mth_sqrt(attack);
    let mut m = Mat4::IDENTITY;
    let x = -0.3 * (sqrt_attack * pi).sin();
    let y = 0.4 * (sqrt_attack * (pi * 2.0)).sin();
    let z = -0.4 * (attack * pi).sin();
    m *= Mat4::from_translation(Vec3::new(
        x + 0.64000005,
        y + -0.6 + inverse_arm_height * -0.6,
        z + -0.71999997,
    ));
    m *= rot_deg(1, 45.0);
    let z_swing = (attack * attack * pi).sin();
    let y_swing = (sqrt_attack * pi).sin();
    m *= rot_deg(1, y_swing * 70.0);
    m *= rot_deg(2, z_swing * -20.0);
    m *= Mat4::from_translation(Vec3::new(-1.0, 3.6, 3.5));
    m *= rot_deg(2, 120.0);
    m *= rot_deg(0, 200.0);
    m *= rot_deg(1, -135.0);
    m *= Mat4::from_translation(Vec3::new(5.6, 0.0, 0.0));
    m
}

/// `Mth.sqrt`: NaN for negative values, the root otherwise.
fn mth_sqrt(v: f32) -> f32 {
    if v < 0.0 {
        f32::NAN
    } else {
        v.sqrt()
    }
}

/// `rotateDegrees(Axis.XP|YP|ZP, degrees)`: 0 = X, 1 = Y, 2 = Z.
fn rot_deg(axis: u8, deg: f32) -> Mat4 {
    let r = deg.to_radians();
    match axis {
        0 => Mat4::from_rotation_x(r),
        1 => Mat4::from_rotation_y(r),
        _ => Mat4::from_rotation_z(r),
    }
}

#[test]
fn the_empty_hands_transform_is_vanillas_render_player_arm() {
    for (attack, equip) in [(0.0, 1.0), (0.25, 1.0), (0.5, 0.5), (0.75, 1.0), (1.0, 0.0)] {
        let mut h = still_hand();
        h.attack = TickPair::fixed(attack);
        h.equip = TickPair::fixed(equip);
        let want = vanilla_render_player_arm(attack, 1.0 - equip);
        assert!(
            close(arm_matrix(&h, 0.0), want, 1e-6),
            "attack {attack} equip {equip}: {:?} vs {want:?}",
            arm_matrix(&h, 0.0).to_cols_array_2d(),
        );
    }
}

#[test]
fn the_held_items_transform_is_vanillas_apply_item_arm_transform() {
    // `applyItemArmTransform` + `swingArm`'s translate + `applyItemArmAttackTransform`, at a
    // quarter of the swing: -0.4 * sin(PI / 2) = -0.4 left and -0.2 * sin(PI / 4) =
    // -0.14142136 forward, plus the swing's rotations.
    for (attack, equip) in [(0.0, 1.0), (0.25, 1.0), (0.5, 0.4), (0.75, 1.0), (1.0, 0.0)] {
        let mut h = still_hand();
        h.attack = TickPair::fixed(attack);
        h.equip = TickPair::fixed(equip);
        let pi = std::f32::consts::PI;
        let sqrt_attack = mth_sqrt(attack);
        let inverse_arm_height = 1.0 - equip;
        let mut want =
            Mat4::from_translation(Vec3::new(0.56, -0.52 + inverse_arm_height * -0.6, -0.72));
        want *= Mat4::from_translation(Vec3::new(
            -0.4 * (sqrt_attack * pi).sin(),
            0.2 * (sqrt_attack * (pi * 2.0)).sin(),
            -0.2 * (attack * pi).sin(),
        ));
        let y_swing = (attack * attack * pi).sin();
        want *= rot_deg(1, 45.0 + y_swing * -20.0);
        let xz_swing = (sqrt_attack * pi).sin();
        want *= rot_deg(2, xz_swing * -20.0);
        want *= rot_deg(0, xz_swing * -80.0);
        want *= rot_deg(1, -45.0);
        assert!(
            close(item_arm_matrix(&h, 0.0), want, 1e-6),
            "attack {attack} equip {equip}: {:?} vs {want:?}",
            item_arm_matrix(&h, 0.0).to_cols_array_2d(),
        );
    }
}

#[test]
fn the_swing_is_the_interpolated_attack_value() {
    // A swing's tick pair lerps into the arm and item transforms at the partial tick
    // (vanilla interpolates `swingAnimation` the same way before both transforms read it):
    // mid-tick of a 0 -> 0.5 swing is the 0.25 matrix, not an average of matrices.
    let mut h = still_hand();
    h.attack = TickPair {
        start: 0.0,
        end: 0.5,
    };
    for partial in [0.25f32, 0.5, 0.75] {
        let mut lerped = still_hand();
        lerped.attack = TickPair::fixed(h.attack.at(partial));
        assert!(
            close(arm_matrix(&h, partial), arm_matrix(&lerped, 0.0), 1e-6),
            "arm at partial {partial}"
        );
        assert!(
            close(
                item_arm_matrix(&h, partial),
                item_arm_matrix(&lerped, 0.0),
                1e-6
            ),
            "item at partial {partial}"
        );
    }
    // The mid-tick matrix is none of the ends' matrices.
    assert!(!close(
        arm_matrix(&h, 0.5),
        arm_matrix(&still_hand(), 0.0),
        1e-4
    ));
}

#[test]
fn a_lowered_equip_progress_drops_the_hand() {
    // `inverseArmHeight = 1 - equip` is inside the outermost translate, so dropping the
    // equip progress from 1 to 0 moves the arm (and the item) down by exactly 0.6 blocks:
    // `renderPlayerArm`'s `inverseArmHeight * -0.6` and `applyItemArmTransform`'s.
    let mut down = still_hand();
    down.equip = TickPair::fixed(0.0);
    for (rest, moved) in [
        (arm_matrix(&still_hand(), 0.0), arm_matrix(&down, 0.0)),
        (
            item_arm_matrix(&still_hand(), 0.0),
            item_arm_matrix(&down, 0.0),
        ),
    ] {
        let d = moved.transform_point3(Vec3::ZERO) - rest.transform_point3(Vec3::ZERO);
        assert!(d.x.abs() < 1e-6 && d.z.abs() < 1e-6, "{d:?}");
        assert!((d.y - -0.6).abs() < 1e-6, "{d:?}");
    }
}

#[test]
fn the_walk_bob_is_carried_by_the_prefix_not_the_hand_root() {
    // Vanilla applies `bobView` to the hand exactly once, on `renderItemInHand`'s pose
    // stack (`GameRenderer.bobView`, the same stack the world's projection carries); the
    // hand's own root (`submitHandsWithItems`) has only the view-lag rotations. The
    // prefix is the renderer's `bob_transform`, so with a bobbing prefix the hand root
    // must not add, remove or reorder anything: the prefix is all there is.
    let mut h = still_hand();
    h.x_bob = TickPair::fixed(20.0);
    let lag = hand_root_matrix(&h, (25.0, 35.0), 0.5);
    // A pure view-bob prefix (translation only): the composed matrix is the prefix
    // times the lag rotations, never the translation twice.
    let prefix = Mat4::from_translation(Vec3::new(0.25, -0.5, 0.0));
    let m = prefix * lag;
    assert!(close(
        m,
        prefix * hand_root_matrix(&h, (25.0, 35.0), 0.5),
        1e-7
    ));
    assert!((m.transform_point3(Vec3::ZERO) - Vec3::new(0.25, -0.5, 0.0)).length() < 1e-6);
    // `hand_root_matrix` with nothing animated is the identity: the bob lives outside.
    assert_eq!(
        hand_root_matrix(&still_hand(), (0.0, 0.0), 1.0),
        Mat4::IDENTITY
    );
}

#[test]
fn the_hand_lags_behind_the_camera_turn() {
    // `submitHandsWithItems` rotates the hand by (viewXRot - xBob) * 0.1 on X and
    // (viewYRot - yBob) * 0.1 on Y, in degrees.
    let mut h = still_hand();
    h.x_bob = TickPair::fixed(20.0);
    h.y_bob = TickPair::fixed(30.0);
    let m = hand_root_matrix(&h, (25.0, 35.0), 0.0);
    // The yaw lags 30 degrees behind a look at 25: (25 - 30) * 0.1 = -0.5 degrees turns the
    // hand half a degree to the left; the pitch lags 20 behind 35: 1.5 degrees up.
    let turned = m.transform_vector3(Vec3::Z);
    assert!(
        (turned.x - -(0.5f32).to_radians().sin()).abs() < 1e-5,
        "{turned:?}"
    );
    assert!(
        (turned.y - -(1.5f32).to_radians().sin()).abs() < 1e-5,
        "{turned:?}"
    );
    // The hand's up axis pitches the same way.
    let pitched = m.transform_vector3(Vec3::Y);
    assert!(
        (pitched.z - (1.5f32).to_radians().sin()).abs() < 1e-5,
        "{pitched:?}"
    );
    // With no bob at all the hand still carries the whole view lag, the way a fresh client
    // does before its bob has caught up: (35 - 0) * 0.1 = 3.5 degrees of pitch and
    // (25 - 0) * 0.1 = 2.5 degrees of yaw.
    let no_lag = hand_root_matrix(&still_hand(), (25.0, 35.0), 0.0);
    let mut want = Mat4::IDENTITY;
    want *= Mat4::from_rotation_x(3.5f32.to_radians());
    want *= Mat4::from_rotation_y(2.5f32.to_radians());
    assert!(close(no_lag, want, 1e-6), "{no_lag:?}");
}

#[test]
fn the_arm_model_is_the_player_models_right_arm() {
    // `HumanoidModel.createMesh` + `PlayerModel.createMesh` + `AvatarRenderer.renderHand`.
    let wide = arm_model(false, true);
    assert_eq!(wide.root.pivot, Vec3::new(-5.0, 2.0, 0.0));
    assert_eq!(wide.root.rotation, Vec3::new(0.0, 0.0, 0.1));
    let cube = &wide.root.cubes[0];
    assert_eq!(cube.origin, Vec3::new(-3.0, -2.0, -2.0));
    assert_eq!(cube.size, Vec3::new(4.0, 12.0, 4.0));
    assert_eq!(cube.uv, [40, 16]);
    let sleeve = &wide.root.children[0];
    assert_eq!(sleeve.name, "right_sleeve");
    assert_eq!(sleeve.cubes[0].uv, [40, 32]);
    assert_eq!(sleeve.cubes[0].inflate, 0.25);
    // Slim: a 3 px arm one pixel to the right.
    let slim = arm_model(true, true);
    assert_eq!(slim.root.cubes[0].origin, Vec3::new(-2.0, -2.0, -2.0));
    assert_eq!(slim.root.cubes[0].size, Vec3::new(3.0, 12.0, 4.0));
}

/// `ItemInHandLayer.submitArmWithItem`: rotate X -90, rotate Y 180, translate
/// (±1, 2, -10)/16 (adult). The left hand is the right hand offset to -1 in x.
#[test]
fn the_held_item_layer_transform_is_vanillas() {
    let deg = |d: f32| d.to_radians();
    let right = held_item_layer_matrix(false);
    let mut want = Mat4::from_rotation_x(deg(-90.0));
    want *= Mat4::from_rotation_y(deg(180.0));
    want *= Mat4::from_translation(Vec3::new(1.0 / 16.0, 2.0 / 16.0, -10.0 / 16.0));
    assert!(close(right, want, 1e-6));
    let left = held_item_layer_matrix(true);
    let mut want_left = Mat4::from_rotation_x(deg(-90.0));
    want_left *= Mat4::from_rotation_y(deg(180.0));
    want_left *= Mat4::from_translation(Vec3::new(-1.0 / 16.0, 2.0 / 16.0, -10.0 / 16.0));
    assert!(close(left, want_left, 1e-6));
}

/// `ItemTransform.apply(true, ...)`: the left-hand item negates translation.x and
/// rotation.y/z of the third-person transform.
#[test]
fn the_left_hand_fixes_the_third_person_transform() {
    let part = haste_assets::ItemModelPart {
        geometry: haste_assets::ItemGeometry::None,
        tints: vec![],
        gui: haste_assets::DisplayTransform::IDENTITY,
        firstperson: haste_assets::DisplayTransform::IDENTITY,
        thirdperson: haste_assets::DisplayTransform::IDENTITY,
        head: haste_assets::DisplayTransform::IDENTITY,
        ground: haste_assets::DisplayTransform::IDENTITY,
        fixed: haste_assets::DisplayTransform::IDENTITY,
        on_shelf: haste_assets::DisplayTransform::IDENTITY,
        gui_light_front: false,
        transformation: None,
    };
    let d = haste_assets::DisplayTransform {
        rotation: [10.0, 20.0, 30.0],
        translation: [0.1, 0.2, 0.3],
        scale: [0.5, 0.5, 0.5],
    };
    let want = item_display_matrix(
        &part,
        &haste_assets::DisplayTransform {
            rotation: [10.0, -20.0, -30.0],
            translation: [-0.1, 0.2, 0.3],
            scale: [0.5, 0.5, 0.5],
        },
        false,
    );
    assert!(close(item_display_matrix(&part, &d, true), want, 1e-6));
}

/// `ArmedModel.translateToHand` -> `part_matrix` gives the arm's animated matrix: the arm's
/// pivot and rotation plus the pose override, under the entity root.
#[test]
fn part_matrix_places_the_hand_at_the_animated_arm() {
    let model = haste_entity_test_humanoid();
    let mut pose = PoseOverrides::default();
    let rest = part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::RIGHT_ARM).unwrap();
    // The right arm's own pivot (no override): (-5, -22, 0).
    assert_eq!(
        rest.transform_point3(Vec3::ZERO),
        Vec3::new(-5.0, -22.0, 0.0)
    );
    // A pose override rotates the arm about its pivot.
    pose.set_rotation(
        parts::RIGHT_ARM,
        Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2),
    );
    let swung = part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::RIGHT_ARM).unwrap();
    let p = swung.transform_point3(Vec3::new(0.0, 10.0, 0.0));
    assert!(
        (p.x - -15.0).abs() < 1e-4 && (p.y - -22.0).abs() < 1e-4,
        "{p:?}"
    );
    // A part no model carries resolves to nothing.
    assert!(part_matrix(&model.root, &pose, Mat4::IDENTITY, parts::intern("nope")).is_none());
}

/// `CustomHeadLayer.translateToHead`: translate (0, -0.25, 0), rotate Y 180, scale
/// (0.625, -0.625, -0.625).
#[test]
fn the_custom_head_transform_is_vanillas() {
    let deg = |d: f32| d.to_radians();
    let want = Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
        * Mat4::from_rotation_y(deg(180.0))
        * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625));
    assert!(close(custom_head_layer_matrix(), want, 1e-6));
}

/// `ElytraModel.createLayer`: a left and a right wing at the shoulders, 64x32 texture.
#[test]
fn the_elytra_model_has_two_wings() {
    let m = elytra_model();
    assert_eq!(m.tex_width, 64);
    assert_eq!(m.tex_height, 32);
    assert_eq!(m.root.children.len(), 2);
    let left = &m.root.children[0];
    assert_eq!(left.name, "left_wing");
    assert_eq!(left.pivot, Vec3::new(5.0, -24.0, 0.0));
    assert_eq!(left.cubes[0].size, Vec3::new(10.0, 20.0, 2.0));
    assert_eq!(left.cubes[0].inflate, 1.0);
    assert!(!left.cubes[0].mirror);
    assert!(m.root.children[1].cubes[0].mirror);
}

/// The armour model has the parts vanilla retains per slot, with the right deformations.
#[test]
fn armour_models_keep_vanillas_parts() {
    use haste_core::model::EquipmentSlot as S;
    let head = humanoid_armor(S::Head);
    assert!(head.root.children.iter().any(|c| c.name == "head"));
    assert_eq!(head.tex_width, 64);
    assert_eq!(head.tex_height, 32);
    let chest = humanoid_armor(S::Chest);
    assert_eq!(chest.root.children.len(), 3);
    assert!(chest.root.children.iter().any(|c| c.name == "left_arm"));
    let legs = humanoid_armor(S::Legs);
    let body = legs
        .root
        .children
        .iter()
        .find(|c| c.name == "body")
        .unwrap();
    assert_eq!(body.cubes[0].inflate, 0.5);
    let feet = humanoid_armor(S::Feet);
    assert_eq!(feet.root.children.len(), 2);
    assert_eq!(feet.root.children[0].cubes[0].inflate, 0.9);
    // The left arm/leg mirror their texture.
    assert!(
        chest
            .root
            .children
            .iter()
            .find(|c| c.name == "left_arm")
            .unwrap()
            .cubes[0]
            .mirror
    );
}

/// A humanoid model for the tests above (the renderer cannot depend on haste-entity).
fn haste_entity_test_humanoid() -> CubeModel {
    CubeModel {
        texture: Identifier::minecraft("x"),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: vec![],
            children: vec![ModelPart {
                name: "right_arm".into(),
                id: parts::RIGHT_ARM,
                pivot: Vec3::new(-5.0, -22.0, 0.0),
                rotation: Vec3::ZERO,
                visible: true,
                cubes: vec![],
                children: vec![],
            }],
        },
    }
}

#[test]
fn item_frame_facing_transform_matches_vanilla() {
    use haste_core::Direction as D;
    let deg = |d: f32| d.to_radians();
    // `ItemFrameRenderer.submit`: translate to `direction * 0.46875`, then the rotations.
    assert!(close(
        item_frame_transform(D::North),
        Mat4::from_translation(Vec3::new(0.0, 0.0, -0.46875)),
        1e-6
    ));
    assert!(close(
        item_frame_transform(D::South),
        Mat4::from_translation(Vec3::new(0.0, 0.0, 0.46875)) * Mat4::from_rotation_y(deg(180.0)),
        1e-6
    ));
    assert!(close(
        item_frame_transform(D::West),
        Mat4::from_translation(Vec3::new(-0.46875, 0.0, 0.0)) * Mat4::from_rotation_y(deg(90.0)),
        1e-6
    ));
    assert!(close(
        item_frame_transform(D::East),
        Mat4::from_translation(Vec3::new(0.46875, 0.0, 0.0)) * Mat4::from_rotation_y(deg(-90.0)),
        1e-6
    ));
    assert!(close(
        item_frame_transform(D::Up),
        Mat4::from_translation(Vec3::new(0.0, 0.46875, 0.0))
            * Mat4::from_rotation_x(deg(-90.0))
            * Mat4::from_rotation_y(deg(180.0)),
        1e-6
    ));
    assert!(close(
        item_frame_transform(D::Down),
        Mat4::from_translation(Vec3::new(0.0, -0.46875, 0.0))
            * Mat4::from_rotation_x(deg(90.0))
            * Mat4::from_rotation_y(deg(180.0)),
        1e-6
    ));
}

#[test]
fn item_frame_item_rotates_45_degrees_per_step_at_half_scale() {
    // `ItemFrameRenderer.submit`: `rotateDegrees(ZP, rotation * 45)`, then scale 0.5.
    let rest = item_frame_item_transform(0, false);
    assert!(close(
        rest,
        Mat4::from_translation(Vec3::new(0.0, 0.0, 0.4375)) * Mat4::from_scale(Vec3::splat(0.5)),
        1e-6
    ));
    // An invisible frame draws no border, so its item sits flush against the wall.
    assert!(close(
        item_frame_item_transform(0, true),
        Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5)) * Mat4::from_scale(Vec3::splat(0.5)),
        1e-6
    ));
    let one = item_frame_item_transform(1, false);
    assert!(close(
        one,
        Mat4::from_translation(Vec3::new(0.0, 0.0, 0.4375))
            * Mat4::from_rotation_z((45.0f32).to_radians())
            * Mat4::from_scale(Vec3::splat(0.5)),
        1e-6
    ));
    // The model's x axis turns 45 degrees and shrinks to half.
    let x = one.transform_vector3(Vec3::X);
    assert!((x.length() - 0.5).abs() < 1e-6, "{x:?}");
    assert!(
        (x.x - 0.5 * (45.0f32).to_radians().cos()).abs() < 1e-6
            && (x.y - 0.5 * (45.0f32).to_radians().sin()).abs() < 1e-6,
        "{x:?}"
    );
}

#[test]
fn painting_facing_transform_matches_vanilla() {
    use haste_core::Direction as D;
    let deg = |d: f32| d.to_radians();
    assert!(close(painting_transform(D::North), Mat4::IDENTITY, 1e-6));
    assert!(close(
        painting_transform(D::South),
        Mat4::from_rotation_y(deg(180.0)),
        1e-6
    ));
    assert!(close(
        painting_transform(D::West),
        Mat4::from_rotation_y(deg(90.0)),
        1e-6
    ));
    assert!(close(
        painting_transform(D::East),
        Mat4::from_rotation_y(deg(-90.0)),
        1e-6
    ));
}

/// Hand shading is world-oriented (`GameRenderer.renderItemInHand`): the face toward the
/// camera faces world up when looking straight down (brightest) and world down when looking
/// straight up (ambient only).
#[test]
fn hand_shade_is_world_oriented() {
    let toward_camera = Vec3::new(0.0, 0.0, 1.0);
    let down = hand_normal_rotation(0.0, 90.0) * toward_camera;
    let up = hand_normal_rotation(0.0, -90.0) * toward_camera;
    assert!(down.abs_diff_eq(Vec3::Y, 1e-5), "{down}");
    assert!(up.abs_diff_eq(-Vec3::Y, 1e-5), "{up}");
    assert!(entity_shade(down) > entity_shade(up));
    assert!((entity_shade(up) - 0.4).abs() < 1e-6);
}

/// The tail upload after `append_block_entities`: only the block entities' vertices past
/// the entities' `start` go up when the existing buffer already holds them, everything
/// when the buffer grew (a fresh wgpu buffer holds no head), and nothing when no block
/// entity appended vertices.
#[test]
fn append_upload_range_covers_only_the_appended_tail() {
    // Entities took 100 vertices; 40 more appended, buffer big enough: tail only.
    assert_eq!(append_upload_range(140, 256, 100), 100..140);
    // Nothing appended: no upload at all.
    assert_eq!(append_upload_range(100, 256, 100), 100..100);
    // The append outgrew the buffer: the whole stream is rewritten (fresh buffer).
    assert_eq!(append_upload_range(300, 256, 100), 0..300);
    // No entities either: the block entities are the whole stream, from 0.
    assert_eq!(append_upload_range(40, 256, 0), 0..40);
    // Outgrew it with no room to spare: still the whole stream.
    assert_eq!(append_upload_range(512, 256, 100), 0..512);
}
