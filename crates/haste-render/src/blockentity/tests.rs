use super::*;
use crate::entity::emit_part;

fn emit(inst: &BlockEntityInstance) -> Vec<EntityVertex> {
    let mut out = Vec::new();
    let f = |out: &mut Vec<EntityVertex>,
             model: &CubeModel,
             part: &ModelPart,
             pose: &PoseOverrides,
             m: Mat4,
             rect: [u32; 4],
             light: (u8, u8)| {
        emit_part(out, model, part, pose, m, rect, light, 0.0);
    };
    emit_block_entity(&mut out, inst, glam::DVec3::ZERO, [0, 0, 64, 64], &f);
    out
}

fn chest(facing: Direction, half: ChestHalf, open: f32) -> BlockEntityInstance {
    BlockEntityInstance {
        pos: BlockPos::new(0, 0, 0),
        facing,
        half,
        kind: BlockEntityKind::Chest(ChestMaterial::Normal),
        open: TickPair::fixed(open),
        light: (15, 0),
        sign_angle: 0.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        beam_time: 0.0,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    }
}

fn shulker(facing: Direction, progress: f32) -> BlockEntityInstance {
    BlockEntityInstance {
        pos: BlockPos::new(0, 0, 0),
        facing,
        half: ChestHalf::Single,
        kind: BlockEntityKind::Shulker(None),
        open: TickPair::fixed(progress),
        light: (15, 0),
        sign_angle: 0.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        beam_time: 0.0,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    }
}

/// Closed single chest: three cubes, six faces each, inside the block (the bottom spans
/// x/z 1..15, y 0..10; the lid adds up to y 14; the lock reaches the block's front edge).
#[test]
fn closed_chest_fits_its_block() {
    let out = emit(&chest(Direction::South, ChestHalf::Single, 0.0));
    assert_eq!(out.len(), 3 * 6 * 6);
    for v in &out {
        let [x, y, z] = v.pos;
        assert!((0.99 / 16.0..=15.01 / 16.0).contains(&x), "x: {x}");
        assert!((-0.01 / 16.0..=14.01 / 16.0).contains(&y), "y: {y}");
        assert!((0.99 / 16.0..=16.01 / 16.0).contains(&z), "z: {z}");
    }
}

/// The lid swings up and over the hinge at the back: fully open, the lid's front edge
/// sits above the block and behind the hinge line (vanilla's `-PI/2` x rotation).
#[test]
fn open_chest_lid_swings_back() {
    let out = emit(&chest(Direction::South, ChestHalf::Single, 1.0));
    let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
    assert!(max_y > 1.0, "lid does not rise above the block: {max_y}");
    // The lock (front, z 14..15 closed) ends up behind the block's centre.
    // Every vertex of the moved lid stays behind its hinge z = 1/16 + 14/16.
    let hinge = 15.0 / 16.0;
    let front_z = out.iter().map(|v| v.pos[2]).fold(0.0f32, f32::max);
    assert!(front_z <= hinge + 1e-4, "lid front z {front_z} > {hinge}");
}

/// A north-facing chest turns its front (the lock, at the block's front edge) from the
/// south side to the north side: the rotation is vanilla's `-toYRot(north) = -180
/// degrees` about the block centre, so the lock's face that touched z = 1 touches z = 0.
#[test]
fn north_facing_chest_turns_its_front() {
    let south = emit(&chest(Direction::South, ChestHalf::Single, 0.0));
    let north = emit(&chest(Direction::North, ChestHalf::Single, 0.0));
    // The lock protrudes past the box to the block edge: the chest's extreme z is its
    // front. Facing south it is z = 16/16; facing north the chest is rotated within the
    // block, so the extreme front is z = 0 and the box's back fills up to 15/16.
    let max_z = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[2]).fold(0.0f32, f32::max);
    let min_z = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[2]).fold(f32::MAX, f32::min);
    assert!(max_z(&south) > 15.5 / 16.0);
    assert!(
        min_z(&north) < 0.5 / 16.0,
        "front stays south: {}",
        min_z(&north)
    );
    assert!(max_z(&north) > 14.5 / 16.0, "chest lost its body");
}

/// The lid curve: vanilla `1 - (1 - open)^3`.
#[test]
fn chest_open_curve_matches_vanilla() {
    assert_eq!(chest_open_curve(0.0), 0.0);
    assert_eq!(chest_open_curve(1.0), 1.0);
    assert!((chest_open_curve(0.5) - 0.875).abs() < 1e-6);
    assert!((chest_open_curve(0.1) - 0.271).abs() < 1e-6);
}

/// Vanilla `ChestModel.createSingleBodyLayer`: bottom, lid and lock with the `addBox` origins,
/// sizes and texture offsets (object convention, y up), lid and lock on the (0, 9, 1) pivot.
#[test]
fn single_chest_model_matches_vanilla_layer() {
    let m = chest_model(ChestHalf::Single);
    assert_eq!((m.tex_width, m.tex_height), (64, 64));
    let names: Vec<&str> = m.root.children.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["bottom", "lid", "lock"]);
    let bottom = &m.root.children[0];
    assert_eq!(bottom.pivot, Vec3::ZERO);
    assert_eq!(bottom.cubes.len(), 1);
    assert_eq!(bottom.cubes[0].origin, Vec3::new(1.0, 0.0, 1.0));
    assert_eq!(bottom.cubes[0].size, Vec3::new(14.0, 10.0, 14.0));
    assert_eq!(bottom.cubes[0].uv, [0, 19]);
    let lid = &m.root.children[1];
    assert_eq!(lid.pivot, Vec3::new(0.0, 9.0, 1.0));
    assert_eq!(lid.cubes[0].origin, Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(lid.cubes[0].size, Vec3::new(14.0, 5.0, 14.0));
    assert_eq!(lid.cubes[0].uv, [0, 0]);
    // The latch: `addBox(7, -2, 14, 2, 4, 1)`, texOffs(0, 0), on the lid's pivot.
    let lock = &m.root.children[2];
    assert_eq!(lock.pivot, Vec3::new(0.0, 9.0, 1.0));
    assert_eq!(lock.cubes[0].origin, Vec3::new(7.0, -2.0, 14.0));
    assert_eq!(lock.cubes[0].size, Vec3::new(2.0, 4.0, 1.0));
    assert_eq!(lock.cubes[0].uv, [0, 0]);
}

/// Vanilla `ChestModel.createDoubleBodyLeftLayer`/`Right`: the bottom and lid widen to 15 and
/// the lock moves to the outer edge (left x 0, right x 15), still 1x4x1.
#[test]
fn double_chest_models_match_vanilla_layers() {
    let left = chest_model(ChestHalf::Left);
    assert_eq!(
        left.root.children[0].cubes[0].origin,
        Vec3::new(0.0, 0.0, 1.0)
    );
    assert_eq!(
        left.root.children[0].cubes[0].size,
        Vec3::new(15.0, 10.0, 14.0)
    );
    assert_eq!(
        left.root.children[1].cubes[0].origin,
        Vec3::new(0.0, 0.0, 0.0)
    );
    assert_eq!(
        left.root.children[1].cubes[0].size,
        Vec3::new(15.0, 5.0, 14.0)
    );
    assert_eq!(
        left.root.children[2].cubes[0].origin,
        Vec3::new(0.0, -2.0, 14.0)
    );
    assert_eq!(
        left.root.children[2].cubes[0].size,
        Vec3::new(1.0, 4.0, 1.0)
    );

    let right = chest_model(ChestHalf::Right);
    assert_eq!(
        right.root.children[0].cubes[0].origin,
        Vec3::new(1.0, 0.0, 1.0)
    );
    assert_eq!(
        right.root.children[0].cubes[0].size,
        Vec3::new(15.0, 10.0, 14.0)
    );
    assert_eq!(
        right.root.children[2].cubes[0].origin,
        Vec3::new(15.0, -2.0, 14.0)
    );
    assert_eq!(
        right.root.children[2].cubes[0].size,
        Vec3::new(1.0, 4.0, 1.0)
    );
}

/// Closed shulker box: base in the lower half, lid up to the block top; the 0.9995
/// shrink keeps everything just inside the block (0..1 on every axis, centred).
#[test]
fn closed_shulker_fits_its_block() {
    let out = emit(&shulker(Direction::Up, 0.0));
    assert_eq!(out.len(), 2 * 6 * 6);
    for v in &out {
        let [x, y, z] = v.pos;
        assert!((-1e-4..=1.0).contains(&y), "shulker y out: {y}");
        assert!((-1e-4..=1.0 + 1e-4).contains(&x), "shulker x out: {x}");
        assert!((-1e-4..=1.0 + 1e-4).contains(&z), "shulker z out: {z}");
    }
    let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
    assert!(max_y > 0.99, "lid does not reach the block top: {max_y}");
}

/// Open shulker box: the lid rises half a block (vanilla `MAX_LID_HEIGHT`) and spins.
#[test]
fn open_shulker_lid_rises_half_a_block() {
    let closed = emit(&shulker(Direction::Up, 0.0));
    let open = emit(&shulker(Direction::Up, 1.0));
    let top = |v: &Vec<EntityVertex>| v.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
    assert!((top(&open) - top(&closed) - 0.5).abs() < 1e-3);
}

/// A sideways shulker box lies in the block, not above it: every facing's rotation is
/// vanilla's `Direction.getRotation`, and the box keeps the block's bounds.
#[test]
fn sideways_shulker_stays_in_the_block() {
    for facing in Direction::ALL {
        let out = emit(&shulker(facing, 0.0));
        for v in &out {
            let [x, y, z] = v.pos;
            assert!((-1e-4..=1.0 + 1e-4).contains(&y), "{facing:?} y out: {y}");
            assert!((-1e-4..=1.0 + 1e-4).contains(&x), "{facing:?} x out: {x}");
            assert!((-1e-4..=1.0 + 1e-4).contains(&z), "{facing:?} z out: {z}");
        }
    }
}

/// Type resolution: the four families map, unknown types and copper chests do not.
#[test]
fn kinds_resolve() {
    let chest = Identifier::minecraft("chest");
    let block = Identifier::minecraft("chest");
    assert_eq!(
        kind_of(&chest, &block, false),
        Some(BlockEntityKind::Chest(ChestMaterial::Normal))
    );
    assert_eq!(
        kind_of(&Identifier::minecraft("trapped_chest"), &block, false),
        Some(BlockEntityKind::Chest(ChestMaterial::Trapped))
    );
    assert_eq!(
        kind_of(&Identifier::minecraft("ender_chest"), &block, false),
        Some(BlockEntityKind::Chest(ChestMaterial::Ender))
    );
    // Christmas overrides normal and trapped (vanilla `getChestMaterial`).
    assert_eq!(
        kind_of(&chest, &block, true),
        Some(BlockEntityKind::Chest(ChestMaterial::Christmas))
    );
    // Shulker colour from the block id.
    assert_eq!(
        kind_of(
            &Identifier::minecraft("shulker_box"),
            &Identifier::minecraft("lime_shulker_box"),
            false
        ),
        Some(BlockEntityKind::Shulker(Some(5)))
    );
    assert_eq!(
        kind_of(
            &Identifier::minecraft("shulker_box"),
            &Identifier::minecraft("shulker_box"),
            false
        ),
        Some(BlockEntityKind::Shulker(None))
    );
    // Signs and hanging signs.
    assert_eq!(
        kind_of(&Identifier::minecraft("sign"), &block, false),
        Some(BlockEntityKind::Sign { hanging: false })
    );
    assert_eq!(
        kind_of(&Identifier::minecraft("hanging_sign"), &block, false),
        Some(BlockEntityKind::Sign { hanging: true })
    );
    // Banners: standing and wall (the wall from the block id).
    assert_eq!(
        kind_of(
            &Identifier::minecraft("banner"),
            &Identifier::minecraft("red_banner"),
            false
        ),
        Some(BlockEntityKind::Banner { wall: false })
    );
    assert_eq!(
        kind_of(
            &Identifier::minecraft("banner"),
            &Identifier::minecraft("blue_wall_banner"),
            false
        ),
        Some(BlockEntityKind::Banner { wall: true })
    );
    // Skulls: every kind from its block id, standing and wall.
    for (block, kind) in [
        ("skeleton_skull", SkullKind::Skeleton),
        ("wither_skeleton_skull", SkullKind::WitherSkeleton),
        ("zombie_head", SkullKind::Zombie),
        ("creeper_head", SkullKind::Creeper),
        ("dragon_head", SkullKind::Dragon),
        ("piglin_head", SkullKind::Piglin),
        ("player_head", SkullKind::Player),
    ] {
        assert_eq!(
            kind_of(
                &Identifier::minecraft("skull"),
                &Identifier::minecraft(block),
                false
            ),
            Some(BlockEntityKind::Skull { kind, wall: false }),
            "{block}"
        );
        let wall_block = block.replace("_skull", "_wall_skull");
        let wall_block = if block.ends_with("_skull") {
            wall_block
        } else {
            block.replace("_head", "_wall_head")
        };
        assert_eq!(
            kind_of(
                &Identifier::minecraft("skull"),
                &Identifier::minecraft(&wall_block),
                false
            ),
            Some(BlockEntityKind::Skull { kind, wall: true }),
            "{wall_block}"
        );
    }
    // Unknown and out of scope.
    assert_eq!(
        kind_of(&Identifier::minecraft("copper_chest"), &block, false),
        None
    );
    assert_eq!(
        kind_of(&Identifier::new("mod", "chest"), &block, false),
        None
    );
}

/// The skull models follow vanilla's layers: the mob head is one cube on 64x32, the
/// humanoid head adds the hat on 64x64, the piglin head has snout and ears on 64x64, the
/// dragon head is the 0.75-scaled part set with the jaw on 256x256.
#[test]
fn skull_models_match_vanilla_layers() {
    let mob = skull_model(SkullKind::Skeleton);
    assert_eq!((mob.tex_width, mob.tex_height), (64, 32));
    assert_eq!(mob.root.children.len(), 1);
    let head = &mob.root.children[0];
    assert_eq!(head.cubes.len(), 1);
    assert_eq!(head.cubes[0].origin, Vec3::new(-4.0, -8.0, -4.0));
    assert_eq!(head.cubes[0].size, Vec3::new(8.0, 8.0, 8.0));

    let humanoid = skull_model(SkullKind::Player);
    assert_eq!((humanoid.tex_width, humanoid.tex_height), (64, 64));
    // Head + hat.
    assert_eq!(humanoid.root.children[0].cubes.len(), 2);
    assert_eq!(humanoid.root.children[0].cubes[1].uv, [32, 0]);
    assert_eq!(humanoid.root.children[0].cubes[1].inflate, 0.25);

    let piglin = skull_model(SkullKind::Piglin);
    assert_eq!((piglin.tex_width, piglin.tex_height), (64, 64));
    let phead = &piglin.root.children[0];
    // Head, snout and two nostrils.
    assert_eq!(phead.cubes.len(), 4);
    // The ears on their pivots.
    assert_eq!(phead.children.len(), 2);
    assert_eq!(phead.children[0].pivot, Vec3::new(4.5, -6.0, 0.0));

    let dragon = skull_model(SkullKind::Dragon);
    assert_eq!((dragon.tex_width, dragon.tex_height), (256, 256));
    let dhead = &dragon.root.children[0];
    // The pivot scaled 0.75: -7.986666 * 0.75.
    assert!((dhead.pivot.y - -5.99).abs() < 0.01, "{}", dhead.pivot.y);
    assert_eq!(dhead.cubes.len(), 6);
    // The jaw child at (0, 4, -8).
    assert_eq!(dhead.children[0].pivot, Vec3::new(0.0, 4.0, -8.0));
}

/// The skull transform: a ground skull sits in the block's bottom half (the head cube's
/// y -8..0 maps to 0..0.5 blocks); a north-wall skull is offset 0.25 into the block and
/// turned to look away from the wall.
#[test]
fn skull_root_places_the_head() {
    let pos = BlockPos::new(0, 0, 0);
    // Ground, rotation 0 (south): the head's centre (0, -4, 0 in model space) lands at
    // (0.5, 0.25, 0.5) — the block centre, a quarter up.
    let m = skull_root(pos, false, Direction::South, 0, glam::DVec3::ZERO);
    let p = m.transform_point3(Vec3::new(0.0, -4.0, 0.0));
    assert!((p.x - 0.5).abs() < 1e-6, "x: {}", p.x);
    assert!((p.y - 0.25).abs() < 1e-6, "y: {}", p.y);
    assert!((p.z - 0.5).abs() < 1e-6, "z: {}", p.z);
    // The head's top (model y -8) is at 0.5 blocks.
    let top = m.transform_point3(Vec3::new(0.0, -8.0, 0.0));
    assert!((top.y - 0.5).abs() < 1e-6, "top: {}", top.y);
    // A south wall (the skull hangs on the south wall of the block behind it, facing
    // north): the origin is 0.25 off the wall.
    let w = skull_root(pos, true, Direction::South, 0, glam::DVec3::ZERO);
    let q = w.transform_point3(Vec3::new(0.0, -4.0, 0.0));
    // The wall's facing points south (+z): the offset moves the head north
    // (0.5 - 1*0.25 = 0.25), and it sits 0.25 + 0.25 up.
    assert!((q.x - 0.5).abs() < 1e-6, "wx: {}", q.x);
    assert!((q.z - 0.25).abs() < 1e-6, "wz: {}", q.z);
    assert!((q.y - 0.5).abs() < 1e-6, "wy: {}", q.y);
}

/// The dragon jaw and piglin ear animations match vanilla's formulas.
#[test]
fn skull_animations_match_vanilla() {
    // Jaw at anim 0: (sin(0) + 1) * 0.2 = 0.2.
    assert!((skull_dragon_jaw_rotation(0.0) - 0.2).abs() < 1e-6);
    // Ears at anim 0: left -(cos(0) + 2.5) * 0.2 = -0.7, right (cos(0) + 2.5) * 0.2 = 0.7.
    let (l, r) = skull_piglin_ear_rotations(0.0);
    assert!((l - -0.7).abs() < 1e-6, "left: {l}");
    assert!((r - 0.7).abs() < 1e-6, "right: {r}");
}

/// The calendar rule is December 24..26 (vanilla `SpecialDates.isExtendedChristmas`);
/// the civil-date helper is checked on known epochs since the current date is not
/// controllable from a test. 2025-01-01 is 1735689600.
#[test]
fn xmas_covers_december_24_to_26() {
    let day = |secs: u64| {
        local_month_day(
            std::time::SystemTime::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(secs))
                .unwrap(),
        )
    };
    assert_eq!(day(1_735_689_600), (1, 1)); // 2025-01-01
    assert_eq!(day(1_734_998_400), (12, 24)); // 2024-12-24
    assert_eq!(day(1_735_084_800), (12, 25)); // 2024-12-25
    assert_eq!(day(1_735_171_200), (12, 26)); // 2024-12-26
    assert_eq!(day(1_735_257_600), (12, 27)); // 2024-12-27
    assert_eq!(day(1_734_912_000), (12, 23)); // 2024-12-23
}

fn side(color: u8, glowing: bool) -> SignSideText {
    SignSideText {
        lines: std::array::from_fn(|_| String::new()),
        color,
        glowing,
    }
}

/// A standing banner's model spans the block: the pole rises from the ground to y = 42/24
/// blocks (scale 2/3: 42 pixels * 2/3 / 16 = 1.75), the flag hangs below the bar.
#[test]
fn banner_model_standing_geometry() {
    let model = banner_model(false);
    assert_eq!(model.root.children.len(), 3);
    let names: Vec<&str> = model.root.children.iter().map(|p| &*p.name).collect();
    assert_eq!(names, vec!["pole", "bar", "flag"]);
    // The pole: addBox(-1, -42, -1, 2, 42, 2).
    let pole = &model.root.children[0];
    assert_eq!(pole.cubes[0].origin, Vec3::new(-1.0, -42.0, -1.0));
    assert_eq!(pole.cubes[0].size, Vec3::new(2.0, 42.0, 2.0));
    // The bar: addBox(-10, -44, -1, 20, 2, 2).
    let bar = &model.root.children[1];
    assert_eq!(bar.cubes[0].origin, Vec3::new(-10.0, -44.0, -1.0));
    assert_eq!(bar.cubes[0].size, Vec3::new(20.0, 2.0, 2.0));
    // The flag: pivot (0, -44, 0), addBox(-10, 0, -2, 20, 40, 1).
    let flag = &model.root.children[2];
    assert_eq!(flag.pivot, Vec3::new(0.0, -44.0, 0.0));
    assert_eq!(flag.cubes[0].origin, Vec3::new(-10.0, 0.0, -2.0));
    assert_eq!(flag.cubes[0].size, Vec3::new(20.0, 40.0, 1.0));
    // The wall banner drops the pole and hangs the bar and flag on the wall's z.
    let wall = banner_model(true);
    assert_eq!(wall.root.children.len(), 2);
    assert_eq!(wall.root.children[0].cubes[0].origin.y, -20.5);
    assert_eq!(wall.root.children[0].cubes[0].origin.z, 9.5);
    assert_eq!(wall.root.children[1].pivot, Vec3::new(0.0, -20.5, 10.5));
}

/// Vanilla `BannerRenderer.modelTransformation`: translate (0.5, 0, 0.5), rotate -angle,
/// scale (2/3, -2/3, -2/3) then 1/16 (model pixels to blocks). The ground point of the
/// pole (0, 0, 0 in model space) maps to the block centre at floor level.
#[test]
fn banner_root_places_the_block_centre() {
    let pos = BlockPos::new(0, 0, 0);
    let m = banner_root(pos, 0.0, glam::DVec3::ZERO);
    // Model point (0, 0, 0): the pole's ground point.
    let p = m.transform_point3(Vec3::ZERO);
    assert!((p.x - 0.5).abs() < 1e-6 && p.y.abs() < 1e-6 && (p.z - 0.5).abs() < 1e-6);
    // The pole top (model y -42, flipped up by the scale) rises 42 * 2/3 / 16 = 1.75
    // blocks above the block's floor.
    let top = m.transform_point3(Vec3::new(0.0, -42.0, 0.0));
    assert!((top.y - 1.75).abs() < 1e-6, "pole top: {}", top.y);
    // The flag's x extents (±10 pixels) map to ±10 * 2/3 / 16 = ±0.4167.
    let left = m.transform_point3(Vec3::new(-10.0, -44.0, 0.0));
    assert!((left.x - (0.5 - 10.0 * 2.0 / 3.0 / 16.0)).abs() < 1e-6);
}

/// Vanilla `BannerFlagModel.setupAnim`'s swing and `BannerRenderer`'s phase:
/// `(-0.0125 + 0.01 * cos(2 * PI * phase)) * PI` about x, phase over position hash and
/// game time mod 100.
#[test]
fn banner_wave_matches_vanilla() {
    // phase 0: cos(0) = 1 -> (-0.0125 + 0.01) * PI = -0.0025 * PI.
    assert!(
        (banner_flag_rotation(0.0) - (-0.0025 * std::f32::consts::PI)).abs() < 1e-6,
        "{}",
        banner_flag_rotation(0.0)
    );
    // phase 0.5: cos(PI) = -1 -> (-0.0125 - 0.01) * PI = -0.0225 * PI.
    assert!(
        (banner_flag_rotation(0.5) - (-0.0225 * std::f32::consts::PI)).abs() < 1e-6,
        "{}",
        banner_flag_rotation(0.5)
    );
    // The phase: floorMod(x*7 + y*9 + z*13 + gameTime, 100), over 100, plus partial.
    let a = banner_wave_phase(BlockPos::new(0, 0, 0), 0, 0.0);
    assert_eq!(a, 0.0);
    let b = banner_wave_phase(BlockPos::new(1, 0, 0), 0, 0.0);
    assert_eq!(b, 0.07);
    let c = banner_wave_phase(BlockPos::new(0, 0, 0), 50, 0.5);
    assert_eq!(c, 0.505);
    // Negative positions wrap (floorMod).
    let d = banner_wave_phase(BlockPos::new(-1, 0, 0), 0, 0.0);
    assert_eq!(d, 0.93);
}

/// Vanilla `AbstractSignRenderer.getDarkColor`: non-glowing text is the dye colour scaled
/// by 0.4 per channel; black glowing text keeps its special outline colour instead.
#[test]
fn sign_dark_color_scales_by_four_tenths() {
    // White (0xFFFFFF) * 0.4 = 0x666666.
    assert_eq!(sign_dark_color(&side(0, false)), 0xFF66_6666);
    // Red (0xFF0000) * 0.4 = 0x660000.
    assert_eq!(sign_dark_color(&side(14, false)), 0xFF66_0000);
    // Black glowing keeps vanilla's outline colour.
    assert_eq!(sign_dark_color(&side(15, true)), BLACK_TEXT_OUTLINE_COLOR);
    // Glowing text is drawn with the dye's own colour.
    assert_eq!(sign_glow_color(&side(0, true)), 0xFFFF_FFFF);
}

/// The sign text transform at yaw 0, south-facing front: text space is x right, y *up*
/// (vanilla's -s y scale), origin near the block centre. The front's text faces +z; the
/// back's matrix is the front's turned by 180 degrees.
#[test]
fn sign_text_transform_places_the_origin() {
    let pos = BlockPos::new(0, 0, 0);
    let front = sign_text_transform(false, 0.0, true, glam::DVec3::ZERO, pos);
    let p = front.transform_point3(Vec3::ZERO);
    // Standing sign: (0.5, 0.5, 0.5) + (0, 1/3, 1/21) — the text origin sits just north
    // of the board's centre, one third up.
    assert!((p.x - 0.5).abs() < 1e-6);
    assert!((p.y - (0.5 + 1.0 / 3.0)).abs() < 1e-6, "y: {}", p.y);
    assert!((p.z - (0.5 + 0.046666667)).abs() < 1e-6, "z: {}", p.z);
    // Text x maps to world +x and text y to world -y (the -s scale).
    let px = front.transform_point3(Vec3::new(1.0, 0.0, 0.0));
    let py = front.transform_point3(Vec3::new(0.0, 1.0, 0.0));
    assert!(px.x > p.x);
    assert!(py.y < p.y, "text y is up: {}", py.y);
    // The back is the front rotated 180 degrees about y.
    let back = sign_text_transform(false, 0.0, false, glam::DVec3::ZERO, pos);
    let pb = back.transform_point3(Vec3::ZERO);
    assert!((pb.x - p.x).abs() < 1e-6 && (pb.y - p.y).abs() < 1e-6);
    let pbx = back.transform_point3(Vec3::new(1.0, 0.0, 0.0));
    assert!(pbx.x < pb.x, "back text x flips");
}

/// A hanging sign's text transform starts higher (0.9375 - 0.3125 - 0.32) and uses the
/// bigger 0.0140625 scale.
#[test]
fn hanging_sign_text_transform() {
    let pos = BlockPos::new(0, 0, 0);
    let m = sign_text_transform(true, 0.0, true, glam::DVec3::ZERO, pos);
    let p = m.transform_point3(Vec3::ZERO);
    assert!((p.x - 0.5).abs() < 1e-6);
    assert!((p.y - (0.9375 - 0.3125 - 0.32)).abs() < 1e-6, "y: {}", p.y);
    assert!((p.z - (0.5 + 0.073)).abs() < 1e-6, "z: {}", p.z);
}

/// Line metrics: standing 10/90 (`SignBlockEntity`), hanging 9/60
/// (`HangingSignBlockEntity`).
#[test]
fn sign_line_metrics_match_vanilla() {
    assert_eq!(sign_line_metrics(false), (10, 90));
    assert_eq!(sign_line_metrics(true), (9, 60));
}

/// Vanilla `CampfireRenderer.submit`: every item sits 0.44921875 up, laid flat, in its own
/// quadrant (slot 0 at the facing's front, then clockwise), scaled by 0.375.
#[test]
fn campfire_item_matrix_places_each_slot() {
    let pos = BlockPos::new(0, 0, 0);
    let cam = glam::DVec3::ZERO;
    // The item model's centre (0.5, 0.5, 0.5 in model space) is shifted into the slot's
    // quadrant: -0.3125 + 0.5 * 0.375 = -0.125 in each of the two horizontal axes, laid
    // flat by the 90-degree x rotation.
    let centre = |slot: usize| {
        campfire_item_matrix(pos, Direction::South, slot, cam)
            .transform_point3(Vec3::new(0.5, 0.5, 0.5))
    };
    let s0 = centre(0);
    assert!((s0.x - 0.375).abs() < 1e-6, "x: {}", s0.x);
    // The 90-degree x rotation folds the model's own half depth (0.5 * 0.375) into y.
    assert!(
        (s0.y - (0.44921875 - 0.5 * CAMPFIRE_ITEM_SIZE)).abs() < 1e-6,
        "y: {}",
        s0.y
    );
    assert!((s0.z - 0.375).abs() < 1e-6, "z: {}", s0.z);
    // Slot 1 turns 90 degrees (west), slot 2 180 (north), slot 3 270 (east).
    let s1 = centre(1);
    assert!(
        (s1.x - 0.625).abs() < 1e-6 && (s1.z - 0.375).abs() < 1e-6,
        "{s1:?}"
    );
    let s2 = centre(2);
    assert!(
        (s2.x - 0.625).abs() < 1e-6 && (s2.z - 0.625).abs() < 1e-6,
        "{s2:?}"
    );
    let s3 = centre(3);
    assert!(
        (s3.x - 0.375).abs() < 1e-6 && (s3.z - 0.625).abs() < 1e-6,
        "{s3:?}"
    );
    // The model's 0.375 scale: one block of model space is 0.375 blocks wide.
    let edge = campfire_item_matrix(pos, Direction::South, 0, cam)
        .transform_point3(Vec3::new(1.5, 0.5, 0.5))
        - s0;
    assert!(
        (edge.x - CAMPFIRE_ITEM_SIZE).abs() < 1e-6,
        "scale: {}",
        edge.x
    );
    // A west-facing campfire's slots are shifted by one (slot 0 goes to the west).
    let shifted = campfire_item_matrix(pos, Direction::West, 0, cam)
        .transform_point3(Vec3::new(0.5, 0.5, 0.5));
    assert!((shifted.x - s1.x).abs() < 1e-6, "{} vs {}", shifted.x, s1.x);
    assert!((shifted.z - s1.z).abs() < 1e-6);
}

/// Vanilla `SpawnerRenderer.submitEntityInSpawner` + `LivingEntityRenderer.submit`: the
/// block's centre at 0.4, the model tipped 30 degrees back and scaled down, with the
/// display entity's fixed 180-degree yaw and the model flip.
#[test]
fn spawner_root_tips_and_scales_the_mob() {
    let m = spawner_root(
        BlockPos::new(0, 0, 0),
        0.0,
        SPAWNER_SCALE,
        glam::DVec3::ZERO,
    );
    // The feet (model origin) sit at the block's centre, dropped by 0.2.
    let p = m.transform_point3(Vec3::ZERO);
    assert!((p.x - 0.5).abs() < 1e-6, "x: {}", p.x);
    assert!((p.y - (0.4 - 0.2)).abs() < 1e-6, "y: {}", p.y);
    assert!((p.z - 0.5).abs() < 1e-6, "z: {}", p.z);
    // One model block up the y axis: `scale * cos(30)` blocks above the feet (the tip
    // takes some of it into z).
    let up = m.transform_point3(Vec3::new(0.0, -16.0, 0.0)) - p;
    assert!(
        (up.y - (SPAWNER_SCALE * 30.0f32.to_radians().cos())).abs() < 1e-6,
        "up: {}",
        up.y
    );
    assert!(
        (up.z - (-SPAWNER_SCALE * 30.0f32.to_radians().sin())).abs() < 1e-6,
        "z: {}",
        up.z
    );
    // Along the model's +z the tip bends the axis down by half its length (sin 30) and
    // shortens it by cos 30 (the yaw also turns it towards -z).
    let z = m.transform_point3(Vec3::new(0.0, 0.0, 16.0)) - p;
    assert!((z.y + SPAWNER_SCALE * 0.5).abs() < 1e-6, "tip: {}", z.y);
    assert!(
        (z.z - (-SPAWNER_SCALE * 30.0f32.to_radians().cos())).abs() < 1e-6,
        "{}",
        z.z
    );
}

/// Vanilla `VaultRenderer.submit`: the block's centre at 0.4 turned by the spin; at spin 0
/// the item cluster is upright and unrotated, at 90 degrees +x points along -z.
#[test]
fn vault_root_turns_around_the_block_centre() {
    let m = vault_root(BlockPos::new(0, 0, 0), 0.0, glam::DVec3::ZERO);
    let p = m.transform_point3(Vec3::ZERO);
    assert!((p.x - 0.5).abs() < 1e-6 && (p.y - 0.4).abs() < 1e-6 && (p.z - 0.5).abs() < 1e-6);
    let x = m.transform_point3(Vec3::new(1.0, 0.0, 0.0)) - p;
    assert!((x.x - 1.0).abs() < 1e-6 && x.z.abs() < 1e-6, "{x:?}");
    let r = vault_root(BlockPos::new(0, 0, 0), 90.0, glam::DVec3::ZERO);
    let x90 = r.transform_point3(Vec3::new(1.0, 0.0, 0.0)) - p;
    assert!(x90.x.abs() < 1e-6 && (x90.z + 1.0).abs() < 1e-6, "{x90:?}");
}

/// Vanilla `ShelfRenderer.submitItem`: the slots spread along x by 0.3125, the item stands
/// 0.25 in front of the board, and it is centred on the board unless it aligns to the
/// bottom. The camera is taken relative (the block's own corner at 0).
#[test]
fn shelf_item_matrix_spreads_the_slots() {
    let pos = BlockPos::new(0, 0, 0);
    let cam = glam::DVec3::ZERO;
    let m = shelf_item_matrix(pos, Direction::North, 0, false, 0.0, cam);
    let p = m.transform_point3(Vec3::ZERO);
    // North facing: yaw 180, so the slot offset runs along -x and the board is +z.
    assert!((p.x - (0.5 + 0.3125)).abs() < 1e-6, "x: {}", p.x);
    assert!((p.y - 0.5).abs() < 1e-6, "y: {}", p.y);
    assert!((p.z - (0.5 + 0.25)).abs() < 1e-6, "z: {}", p.z);
    // Slot 2 mirrors slot 0 across the block's centre.
    let q =
        shelf_item_matrix(pos, Direction::North, 2, false, 0.0, cam).transform_point3(Vec3::ZERO);
    assert!((q.x - (0.5 - 0.3125)).abs() < 1e-6, "{}", q.x);
    // Align to bottom drops the item by 0.25 blocks (vanilla's ALIGN_ITEMS_TO_BOTTOM,
    // applied outside the item's own scale).
    let bottom =
        shelf_item_matrix(pos, Direction::North, 1, true, 0.0, cam).transform_point3(Vec3::ZERO);
    assert!((bottom.y - (0.5 - 0.25)).abs() < 1e-6, "y: {}", bottom.y);
    // The vertical offset rides in the same space: one model unit of `offset_y` is 0.25
    // blocks (the shelf's scale).
    let lifted =
        shelf_item_matrix(pos, Direction::North, 1, false, 1.0, cam).transform_point3(Vec3::ZERO);
    assert!(
        (lifted.y - (0.5 + SHELF_ITEM_SIZE)).abs() < 1e-6,
        "y: {}",
        lifted.y
    );
}

/// Vanilla `ShelfRenderer`'s alignment: the model's box is lifted by its min y, and by
/// half its height as well when the shelf does not align its items to the bottom.
#[test]
fn shelf_item_offset_centres_or_bottoms_the_model() {
    let bbox = (Vec3::new(-0.5, -0.25, 0.0), Vec3::new(0.5, 0.75, 0.0));
    assert!((shelf_item_offset_y(bbox, true) - 0.25).abs() < 1e-6);
    // Centred: 0.25 (the lift) minus half the 1.0-tall box's height (vanilla's
    // `-(maxY - minY) / 2`).
    assert!((shelf_item_offset_y(bbox, false) - -0.25).abs() < 1e-6);
}

/// Vanilla `BrushableBlockRenderer.submit` + `translations`: each face pushes the item out
/// of it by 0.25/0.73 plus `dusted / 10 * 0.75`, and the item keeps vanilla's 75 + 11
/// degrees of yaw (90 more on the x axis).
#[test]
fn brushable_item_matrix_follows_the_dusted_progress() {
    let pos = BlockPos::new(0, 0, 0);
    let cam = glam::DVec3::ZERO;
    // North (touched from the north face): the item sits at z 0.25 at progress 0.
    let m = brushable_item_matrix(pos, Direction::North, 0, cam);
    let p = m.transform_point3(Vec3::ZERO);
    assert!((p.y - 0.5).abs() < 1e-6, "y: {}", p.y);
    assert!((p.z - 0.25).abs() < 1e-6, "z: {}", p.z);
    assert!((p.x - 0.5).abs() < 1e-6, "x: {}", p.x);
    // Progress 3 adds 3/10 * 0.75 towards the camera (-z).
    let p3 = brushable_item_matrix(pos, Direction::North, 3, cam).transform_point3(Vec3::ZERO);
    assert!((p3.z - (0.25 - 0.225)).abs() < 1e-6, "z: {}", p3.z);
    // East is pushed the other way (0.73 + offset).
    let e = brushable_item_matrix(pos, Direction::East, 0, cam).transform_point3(Vec3::ZERO);
    assert!((e.x - 0.73).abs() < 1e-6, "x: {}", e.x);
    // Down is unique: -0.23 - offset (the item hangs below the block).
    let d = brushable_item_matrix(pos, Direction::Down, 0, cam).transform_point3(Vec3::ZERO);
    assert!((d.y - (0.5 - 0.23)).abs() < 1e-6, "y: {}", d.y);
}

/// The block entity types vanilla has renderers for, and the ones we do not draw (the
/// other worker's lecterns, books, bells, pots and conduits, and the rest).
#[test]
fn kind_of_maps_the_item_and_mob_block_entities() {
    let campfire = Identifier::minecraft("campfire");
    assert_eq!(
        kind_of(&Identifier::minecraft("campfire"), &campfire, false),
        Some(BlockEntityKind::Campfire(CampfireItems::default()))
    );
    for (be, path) in [
        ("mob_spawner", "spawner"),
        ("trial_spawner", "trial_spawner"),
        ("vault", "vault"),
        ("shelf", "shelf"),
        ("brushable_block", "suspicious_sand"),
    ] {
        let block = Identifier::minecraft(path);
        let kind = kind_of(&Identifier::minecraft(be), &block, false).expect(be);
        let payload = matches!(
            kind,
            BlockEntityKind::Spawner(_)
                | BlockEntityKind::Vault(_)
                | BlockEntityKind::Shelf(_)
                | BlockEntityKind::Brushable(_)
        );
        assert!(payload, "{be} mapped to {kind:?}");
    }
    // The object-model block entities resolve too (their own renderer draws them).
    for (be, kind) in [
        ("lectern", BlockEntityKind::Lectern),
        ("enchanting_table", BlockEntityKind::EnchantTable),
        ("bell", BlockEntityKind::Bell),
        ("decorated_pot", BlockEntityKind::DecoratedPot),
        ("conduit", BlockEntityKind::Conduit),
        ("beacon", BlockEntityKind::Beacon),
        ("end_portal", BlockEntityKind::EndPortal),
        ("end_gateway", BlockEntityKind::EndGateway),
        ("piston", BlockEntityKind::MovingPiston),
    ] {
        let block = Identifier::minecraft(be);
        assert_eq!(kind_of(&block, &block, false), Some(kind), "{be}");
    }
    // Not drawn (no vanilla renderer we have).
    for be in ["furnace", "hopper"] {
        assert_eq!(
            kind_of(
                &Identifier::minecraft(be),
                &Identifier::minecraft("stone"),
                false
            ),
            None,
            "{be}"
        );
    }
}

/// Vanilla `ItemClusterRenderState.getRenderedAmount`: one copy for a single item, then
/// one more per full 16 of the count, up to five.
#[test]
fn vault_copies_match_vanilla() {
    let vault = |count: u32| {
        VaultDisplay {
            item: haste_core::RegistryIndex(0),
            count,
            spin: TickPair::fixed(0.0),
        }
        .copies()
    };
    assert_eq!(vault(0), 0);
    assert_eq!(vault(1), 1);
    assert_eq!(vault(16), 2);
    assert_eq!(vault(17), 3);
    assert_eq!(vault(32), 3);
    assert_eq!(vault(33), 4);
    assert_eq!(vault(48), 4);
    assert_eq!(vault(49), 5);
    assert_eq!(vault(1000), 5);
}

/// Vanilla `TrialSpawnerRenderer.extractSpawnerData`'s `Mth.lerp(partialTicks, oSpin, spin)
/// * 10`: the accumulating spin is only scaled for the pose.
#[test]
fn spawner_spin_is_scaled_by_ten() {
    let spawner = SpawnerDisplay {
        model: None,
        spin: TickPair {
            start: 3.0,
            end: 4.0,
        },
        scale: SPAWNER_SCALE,
    };
    assert!((spawner_spin_at(&spawner, 0.5) - 35.0).abs() < 1e-6);
}

/// Vanilla `VaultRenderer`'s `Mth.rotLerp`: the vault's displayed item keeps turning the
/// short way when the 10-degrees-a-tick spin rolls over 360, unlike a plain lerp.
#[test]
fn vault_spin_wraps_through_the_short_arc() {
    let vault = |start: f32, end: f32| VaultDisplay {
        item: haste_core::RegistryIndex(0),
        count: 1,
        spin: TickPair { start, end },
    };
    // Halfway between 350 and 0 (a 10 degree turn through 360): the item is at 355, where a
    // plain lerp would be at 175 and send the item backwards three quarters of a turn.
    assert!((vault(350.0, 360.0).spin_at(0.5) - 355.0).abs() < 1e-6);
    assert!((vault(350.0, 0.0).spin_at(0.5) - 355.0).abs() < 1e-6);
    // A forward 10 degrees well inside the range interpolates plainly.
    assert!((vault(20.0, 30.0).spin_at(0.5) - 25.0).abs() < 1e-6);
    // A backwards turn (a spin the vault can never make) still takes the short arc.
    assert!((vault(100.0, 80.0).spin_at(0.5) - 90.0).abs() < 1e-6);
}

/// The spawner scale: vanilla's 0.53125, divided by the larger of the display entity's
/// box width and height when it exceeds one block.
#[test]
fn spawner_scale_divides_big_models() {
    let small = CubeModel {
        texture: Identifier::minecraft("haste:test"),
        tex_width: 16,
        tex_height: 16,
        root: ModelPart {
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: vec![Cube {
                origin: Vec3::new(-4.0, -4.0, -4.0),
                size: Vec3::new(8.0, 8.0, 8.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            children: Vec::new(),
        },
    };
    // 0.5 blocks at most: no division.
    assert!((spawner_scale(&small) - SPAWNER_SCALE).abs() < 1e-6);
    let tall = CubeModel {
        texture: small.texture.clone(),
        tex_width: 16,
        tex_height: 16,
        root: ModelPart {
            cubes: vec![Cube {
                origin: Vec3::new(-2.0, -48.0, -2.0),
                size: Vec3::new(4.0, 48.0, 4.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            name: "root".into(),
            id: parts::ROOT,
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            children: Vec::new(),
        },
    };
    // 3 blocks tall: 0.53125 / 3.
    assert!(
        (spawner_scale(&tall) - SPAWNER_SCALE / 3.0).abs() < 1e-6,
        "{}",
        spawner_scale(&tall)
    );
}

/// The item block entity payloads fold out of their kinds (the emitter splits them off
/// from the block entity's own texture).
#[test]
fn item_kind_splits_the_item_payloads() {
    let items = CampfireItems {
        items: [Some(haste_core::RegistryIndex(1)), None, None, None],
    };
    match item_kind(&BlockEntityKind::Campfire(items)) {
        Some(ItemBlockEntity::Campfire(c)) => assert_eq!(c.items[0].unwrap().0, 1),
        other => panic!("{other:?}"),
    }
    assert!(item_kind(&BlockEntityKind::Chest(ChestMaterial::Normal)).is_none());
    assert!(item_kind(&BlockEntityKind::Sign { hanging: true }).is_none());
}

fn lec(pos: BlockPos, kind: BlockEntityKind) -> BlockEntityInstance {
    BlockEntityInstance {
        pos,
        facing: Direction::North,
        half: ChestHalf::Single,
        kind,
        open: TickPair::fixed(0.0),
        light: (15, 0),
        sign_angle: 0.0,
        text: None,
        banner_phase: 0.0,
        banner: None,
        skull_anim: TickPair::fixed(0.0),
        anim: None,
        sherds: None,
        beam: None,
        beam_time: 0.0,
        gateway_beam: None,
        portal_faces: 0,
        piston: None,
    }
}

/// The five object-model block entities resolve from their type names, and unknown types
/// still do not.
#[test]
fn object_model_kinds_resolve() {
    for (name, kind) in [
        ("lectern", BlockEntityKind::Lectern),
        ("enchanting_table", BlockEntityKind::EnchantTable),
        ("bell", BlockEntityKind::Bell),
        ("decorated_pot", BlockEntityKind::DecoratedPot),
        ("conduit", BlockEntityKind::Conduit),
    ] {
        let block = Identifier::minecraft(name);
        assert_eq!(kind_of(&block, &block, false), Some(kind), "{name}");
    }
}

/// Vanilla `BookModel.State.forAnimation`: `(sin(progress * 0.02) * 0.1 + 1.25) * openness`.
/// At progress 0 the sine is 0, so the base is 1.25; the lectern's fixed state lifts it by
/// its `openness` of 1.2.
#[test]
fn book_openness_matches_vanilla() {
    assert!((book_openness(0.0, 1.0) - 1.25).abs() < 1e-6);
    assert!((book_openness(0.0, 1.2) - 1.5).abs() < 1e-6);
    // A quarter turn of progress (PI/2 / 0.02 ticks) puts the sine at 1.
    let full = std::f32::consts::PI / 2.0 / 0.02;
    assert!(
        (book_openness(full, 1.0) - 1.35).abs() < 1e-5,
        "{}",
        book_openness(full, 1.0)
    );
}

/// Vanilla `EnchantTableRenderer.submit`'s page flips: `frac(flip + 0.25) * 1.6 - 0.3`
/// clamped to 0..1 (and the same at 0.75 for the second page).
#[test]
fn enchant_page_flip_matches_vanilla() {
    // flip 0: frac(0.25) = 0.25 -> 0.4 - 0.3 = 0.1; frac(0.75) -> 1.2 - 0.3 = 0.9.
    assert!((enchant_page_flip(0.0, 0.25) - 0.1).abs() < 1e-6);
    assert!((enchant_page_flip(0.0, 0.75) - 0.9).abs() < 1e-6);
    // Clamped at both ends: a raw value below 0 clamps to 0 (flip 0.9 -> frac(1.15) = 0.15,
    // 0.24 - 0.3 < 0), and one above 1 clamps up (flip 0.5625 -> frac(0.8125) = 0.8125,
    // 1.3 - 0.3 > 1).
    assert_eq!(enchant_page_flip(0.9, 0.25), 0.0);
    assert_eq!(enchant_page_flip(0.5625, 0.25), 1.0);
    // `frac` wraps: flip 1.25 is flip 0.25.
    assert!((enchant_page_flip(1.25, 0.0) - 0.1).abs() < 1e-6);
}

/// Vanilla `ConduitRenderer.submit`'s bob: `sin(animTime * 0.1) / 2 + 0.5` in 0..1, squared
/// plus itself, so it runs from 0.75 (sine 0) up to 2 (sine at 1, 5 PI ticks) and down to 0
/// (sine at -1, 15 PI ticks).
#[test]
fn conduit_bob_matches_vanilla() {
    assert!(
        (conduit_bob(0.0) - 0.75).abs() < 1e-6,
        "{}",
        conduit_bob(0.0)
    );
    let peak = conduit_bob(std::f32::consts::PI * 5.0);
    assert!((peak - 2.0).abs() < 1e-4, "{peak}");
    let trough = conduit_bob(std::f32::consts::PI * 15.0);
    assert!(trough.abs() < 1e-4, "{trough}");
}

/// Vanilla `ConduitBlockEntity.getActiveRotation`: the counter times -0.0375, in degrees.
#[test]
fn conduit_spin_matches_vanilla() {
    assert_eq!(conduit_spin(0.0), 0.0);
    assert_eq!(conduit_spin(1.0), -0.0375);
    // A full turn takes 9600 ticks (360 / 0.0375).
    assert!((conduit_spin(9600.0) + 360.0).abs() < 1e-2);
}

/// Vanilla `BellModel.setupAnim`: the shake is `sin(ticks / PI) / (4 + ticks / 3)`, and the
/// click direction picks the axis and its sign (north/south tilt about x, west/east about
/// z); without a click direction the bell is at rest.
#[test]
fn bell_shake_matches_vanilla() {
    assert_eq!(bell_shake_rotation(0.0, None), (0.0, 0.0));
    // ticks 0: sin(0) = 0, so the shake is 0 whichever direction.
    assert_eq!(bell_shake_rotation(0.0, Some(Direction::North)), (0.0, 0.0));
    // ticks 1: sin(1 / PI) ~ 0.3138, over 4 + 1/3.
    let base = (1.0 / std::f32::consts::PI).sin() / (4.0 + 1.0 / 3.0);
    assert!((bell_shake_rotation(1.0, Some(Direction::South)).0 - base).abs() < 1e-4);
    assert!((bell_shake_rotation(1.0, Some(Direction::North)).0 + base).abs() < 1e-4);
    assert!(bell_shake_rotation(1.0, Some(Direction::North)).1.abs() < 1e-9);
    assert!((bell_shake_rotation(1.0, Some(Direction::East)).1 + base).abs() < 1e-4);
    assert!((bell_shake_rotation(1.0, Some(Direction::West)).1 - base).abs() < 1e-4);
    // The sine's peak at the first ticks dwarfs the decay; by tick 10 the sine has swung
    // past its peak and the growing denominator has taken over.
    let (a, _) = bell_shake_rotation(10.0, Some(Direction::South));
    assert!(a.abs() < base.abs(), "{a} vs {base}");
}

/// Vanilla's partial-tick interpolation of the object-model block entities' counters:
/// `BellRenderState.ticks` is `ticks + partialTicks`, the pot's wobble progress runs over
/// `(gameTime - wobbleStartedAtTick + partialTicks) / duration`, the enchanting table lerps
/// its `flip`/`open`/`rot`, and the conduit's `getActiveRotation(partialTicks)` adds the
/// partial tick to the spin counter.
#[test]
fn anims_lerp_by_the_partial_tick() {
    let bell = BlockEntityAnim::Bell {
        ticks: TickPair {
            start: 3.0,
            end: 4.0,
        },
        click: Some(Direction::North),
    };
    for partial in [0.0, 0.25, 0.5, 1.0] {
        match bell.at(partial) {
            BlockEntityAnim::Bell { ticks, click } => {
                assert!((ticks.end - (3.0 + partial)).abs() < 1e-6, "{ticks:?}");
                assert_eq!(click, Some(Direction::North));
            }
            other => panic!("{other:?}"),
        }
    }
    let pot = BlockEntityAnim::Pot {
        wobble: TickPair {
            start: 2.0,
            end: 3.0,
        },
        duration: 7,
    };
    match pot.at(0.5) {
        BlockEntityAnim::Pot { wobble, duration } => {
            assert!((wobble.end - 2.5).abs() < 1e-6);
            assert_eq!(duration, 7, "the style's duration travels with the wobble");
        }
        other => panic!("{other:?}"),
    }
    let enchant = BlockEntityAnim::EnchantTable {
        time: 10.0,
        flip: TickPair {
            start: 1.0,
            end: 2.0,
        },
        open: TickPair {
            start: 0.0,
            end: 0.5,
        },
        rot: TickPair {
            start: -1.0,
            end: 1.0,
        },
        t_rot: 0.0,
        flip_t: 0.0,
        flip_a: 0.0,
    };
    match enchant.at(0.25) {
        BlockEntityAnim::EnchantTable {
            time,
            flip,
            open,
            rot,
            ..
        } => {
            assert!((time - 10.25).abs() < 1e-6);
            assert!((flip.end - 1.25).abs() < 1e-6);
            assert!((open.end - 0.125).abs() < 1e-6);
            assert!((rot.end - (-0.5)).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
    let conduit = BlockEntityAnim::Conduit {
        tick_count: 5.0,
        rotation: 100.0,
        active: true,
        hunting: true,
    };
    match conduit.at(0.5) {
        BlockEntityAnim::Conduit {
            tick_count,
            rotation,
            active,
            hunting,
        } => {
            assert!((tick_count - 5.5).abs() < 1e-6);
            assert!((rotation - 100.5).abs() < 1e-6);
            assert!(active && hunting);
        }
        other => panic!("{other:?}"),
    }
}

/// Vanilla `ConduitRenderer.submit`'s active cage: the spin is taken in radians where the
/// shell's is degrees (`rotateDegrees(Axis.YP, activeRotation)` against
/// `rotationAxis(activeRotation * 180/PI * PI/180)`), so only a counter of
/// `2*PI / -0.0375` ticks brings the cage back to a whole turn, while the shell's own 360
/// degrees worth of counter (`360 / -0.0375`) does not move the cage by a full turn.
#[test]
fn conduit_cage_spins_in_radians() {
    let cage_at = |rotation: f32| {
        let mut inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Conduit);
        inst.anim = Some(BlockEntityAnim::Conduit {
            tick_count: 0.0,
            rotation,
            active: true,
            hunting: false,
        });
        emit_object_vertices(&inst, glam::Quat::IDENTITY)
    };
    // Cage (36 quads' worth of vertices), wind twice and the eye: four cubes of six quads.
    let at_rest = cage_at(0.0);
    assert_eq!(at_rest.len(), 4 * 6 * 6);
    // One whole turn of the cage: 2*PI radians at 0.0375 radians per tick.
    let turn = cage_at(std::f32::consts::TAU / -0.0375);
    let moved = at_rest
        .iter()
        .zip(&turn)
        .map(|(a, b)| (a.pos[0] - b.pos[0]).abs() + (a.pos[1] - b.pos[1]).abs())
        .fold(0.0f32, f32::max);
    assert!(moved < 1e-3, "one turn did not return the cage: {moved}");
    // The same counter in degrees (a full shell turn, 360 degrees) is only 360 radians of
    // cage spin, so the cage is somewhere else entirely.
    let degrees_turn = cage_at(360.0 / -0.0375);
    let moved = at_rest
        .iter()
        .zip(&degrees_turn)
        .map(|(a, b)| (a.pos[0] - b.pos[0]).abs() + (a.pos[1] - b.pos[1]).abs())
        .fold(0.0f32, f32::max);
    assert!(moved > 1e-3, "the cage turned with the shell: {moved}");
    // The cage is one block wide, centred on (0.5, 0.3 + hh * 0.2, 0.5) with hh = 0.75.
    for v in at_rest.iter().take(6 * 6) {
        let c = [v.pos[0] - 0.5, v.pos[1] - 0.45, v.pos[2] - 0.5];
        assert!(
            c.iter().all(|a| a.abs() <= 0.2501),
            "cage outside its cube: {c:?}"
        );
    }
}

/// Vanilla `Camera.setRotation`'s orientation: the conduit's eye is billboarded through it,
/// so its plane's normal comes out facing the camera (`ConduitRenderer.submit`'s eye pose
/// ends with the half turn that flips the model's +z onto the view forward).
#[test]
fn camera_orientation_faces_forward_and_up() {
    let forward = |yaw: f32, pitch: f32| {
        let (sy, cy) = (yaw.to_radians().sin(), yaw.to_radians().cos());
        let (sp, cp) = (pitch.to_radians().sin(), pitch.to_radians().cos());
        Vec3::new(-cp * sy, -sp, cp * cy)
    };
    for (yaw, pitch) in [(0.0, 0.0), (45.0, -30.0), (180.0, 10.0), (270.0, -80.0)] {
        let q = camera_orientation(yaw, pitch);
        // The eye's plane normal (+z) through the renderer's pose: the 0.5 scale, the
        // camera orientation and the `rotationZ(PI).rotateY(PI)` half turn about x.
        let n = (Mat4::from_quat(q) * Mat4::from_rotation_x(std::f32::consts::PI))
            .transform_vector3(Vec3::Z);
        let want = forward(yaw, pitch);
        assert!(
            n.distance(want) < 1e-4,
            "yaw {yaw} pitch {pitch}: eye normal {n:?}, want {want:?}"
        );
    }
}

/// Vanilla `ConduitRenderer.submit`'s inactive shell: one cube inside the block, centred.
#[test]
fn conduit_shell_fits_its_block() {
    let mut inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Conduit);
    inst.anim = Some(BlockEntityAnim::Conduit {
        tick_count: 0.0,
        rotation: 0.0,
        active: false,
        hunting: false,
    });
    let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
    assert_eq!(out.len(), 6 * 6);
    for v in &out {
        // One 6x6x6 cube inside the block, centred: 3/16 to every side of the centre.
        let c = [v.pos[0] - 0.5, v.pos[1] - 0.5, v.pos[2] - 0.5];
        assert!(c.iter().all(|a| a.abs() <= 0.1876), "shell outside: {c:?}");
    }
}

/// Vanilla `BellModel.createBodyLayer`: the body hangs from its pivot at (8, 12, 8) model
/// units and the base sits on the block's corner, so the bell occupies the block's lower
/// three quarters, centred.
#[test]
fn bell_model_places_the_body_and_base() {
    let inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Bell);
    let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
    // Two cubes.
    assert_eq!(out.len(), 2 * 6 * 6);
    let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
    let min_y = out.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
    // The body's top is at 12/16 + 1/16 = 0.8125; the base's bottom at 4/16 = 0.25.
    assert!((max_y - 0.8125).abs() < 1e-4, "top {max_y}");
    assert!((min_y - 0.25).abs() < 1e-4, "bottom {min_y}");
    for v in &out {
        assert!((-1e-4..=1.0 + 1e-4).contains(&v.pos[0]), "x {}", v.pos[0]);
        assert!((-1e-4..=1.0 + 1e-4).contains(&v.pos[2]), "z {}", v.pos[2]);
    }
}

/// Vanilla `BookModel.createBodyLayer`: the book is 12 model units wide (0.75 blocks) with
/// the two lids either side of the seam, and the lectern's transform puts it at the
/// lectern's top (y 1.0625) turned by the block's `getClockWise` facing.
#[test]
fn lectern_book_sits_on_the_lectern_top() {
    let inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Lectern);
    let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
    // Seven parts, six faces each.
    assert_eq!(out.len(), 7 * 6 * 6);
    let min_y = out.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
    let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
    // The book's 10-tall lids span 5/16 around the pivot's y: 1.0625 - 0.125 - 5/16 ..
    // 1.0625 - 0.125 + 5/16.
    // The book's lids are 10 model units tall (0.625 blocks) around the pivot at
    // 1.0625 - 0.125, so nothing reaches below 0.6 even turned by the 67.5-degree z tilt.
    assert!(min_y > 0.6, "book hangs below the lectern top: {min_y}");
    assert!(max_y < 1.3, "book rises too high: {max_y}");
    // The book's width: 12 pixels wide and 10 deep, well inside a block.
    let max_x = out.iter().map(|v| v.pos[0]).fold(0.0f32, f32::max);
    let min_x = out.iter().map(|v| v.pos[0]).fold(f32::MAX, f32::min);
    assert!(
        (max_x - min_x) < 1.0,
        "book wider than a block: {max_x} {min_x}"
    );
}

/// The lectern's facing turns the book: `LecternRenderer.extractRenderState` reads
/// `facing.getClockWise().toYRot()`, so a north-facing lectern (whose clockwise is east,
/// yaw 270) turns its book a quarter turn from a south-facing one (west, 90).
#[test]
fn lectern_facing_turns_the_book() {
    let mut north = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Lectern);
    north.facing = Direction::North;
    let mut south = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Lectern);
    south.facing = Direction::South;
    let a = emit_object_vertices(&north, glam::Quat::IDENTITY);
    let b = emit_object_vertices(&south, glam::Quat::IDENTITY);
    // The two poses are different, and each keeps the book inside the block's column.
    let differs = a
        .iter()
        .zip(b.iter())
        .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-4 || (p.pos[2] - q.pos[2]).abs() > 1e-4);
    assert!(differs, "the facing does not turn the book");
}

/// Vanilla `DecoratedPotRenderer.createBaseLayer`: the base's neck ring sits above the
/// block's top and its flare is narrower than the body's 14-pixel width.
#[test]
fn decorated_pot_neck_rises_above_the_block() {
    let inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::DecoratedPot);
    let out = emit_object_vertices(&inst, glam::Quat::IDENTITY);
    // The base layer's neck carries two cubes (the deflated ring and the inflated flare),
    // plus the top and bottom planes and the four side quads.
    assert_eq!(out.len(), 8 * 6 * 6);
    let max_y = out.iter().map(|v| v.pos[1]).fold(0.0f32, f32::max);
    // The neck's flare tops out at 37/16 - 20/16 + 1/16 = 1.1875 plus its 0.2 inflation.
    assert!(max_y > 1.15, "neck does not rise above the block: {max_y}");
    assert!(max_y < 1.35, "neck rises too high: {max_y}");
    // Everything else is inside the block.
    for v in &out {
        if v.pos[1] < max_y - 1e-3 {
            assert!(
                (-0.2..=1.2).contains(&v.pos[0]) && (-0.2..=1.2).contains(&v.pos[2]),
                "pot part outside the block: {:?}",
                v.pos
            );
        }
    }
}

/// The decorated pot's direction turn (vanilla `modelTransformation`): the sides are four
/// separate quads around the block, and the block's `facing` rotates them.
#[test]
fn decorated_pot_facing_turns_the_sides() {
    let mut north = lec(BlockPos::new(0, 0, 0), BlockEntityKind::DecoratedPot);
    north.facing = Direction::North;
    let mut east = lec(BlockPos::new(0, 0, 0), BlockEntityKind::DecoratedPot);
    east.facing = Direction::East;
    let a = emit_object_vertices(&north, glam::Quat::IDENTITY);
    let b = emit_object_vertices(&east, glam::Quat::IDENTITY);
    let differs = a
        .iter()
        .zip(b.iter())
        .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-4 || (p.pos[2] - q.pos[2]).abs() > 1e-4);
    assert!(differs, "the facing does not turn the pot");
}

/// The enchanting table's book animates: the bob lifts it off the (0.5, 0.75, 0.5)
/// transform by up to a hundredth, and the openness closes when no player is near.
#[test]
fn enchant_table_book_animates() {
    let mut inst = lec(BlockPos::new(0, 0, 0), BlockEntityKind::EnchantTable);
    inst.anim = Some(BlockEntityAnim::EnchantTable {
        time: 0.0,
        flip: TickPair::fixed(0.0),
        open: TickPair::fixed(0.5),
        rot: TickPair::fixed(0.0),
        t_rot: 0.0,
        flip_t: 0.0,
        flip_a: 0.0,
    });
    let closed = emit_object_vertices(&inst, glam::Quat::IDENTITY);
    let mut later = inst.clone();
    later.anim = Some(BlockEntityAnim::EnchantTable {
        time: 25.0,
        flip: TickPair::fixed(0.5),
        open: TickPair::fixed(0.9),
        rot: TickPair::fixed(0.4),
        t_rot: 0.4,
        flip_t: 1.0,
        flip_a: 0.1,
    });
    let open = emit_object_vertices(&later, glam::Quat::IDENTITY);
    let differs = closed.iter().zip(open.iter()).any(|(p, q)| {
        (p.pos[0] - q.pos[0]).abs() > 1e-3
            || (p.pos[1] - q.pos[1]).abs() > 1e-3
            || (p.pos[2] - q.pos[2]).abs() > 1e-3
    });
    assert!(differs, "the book does not animate");
}

/// A ringing bell tilts its body (the base stays put): vanilla `BellModel.setupAnim` rotates
/// only `bell_body`.
#[test]
fn ringing_bell_tilts_its_body() {
    let rest = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Bell);
    let mut ringing = lec(BlockPos::new(0, 0, 0), BlockEntityKind::Bell);
    ringing.anim = Some(BlockEntityAnim::Bell {
        ticks: TickPair::fixed(1.0),
        click: Some(Direction::South),
    });
    let a = emit_object_vertices(&rest, glam::Quat::IDENTITY);
    let b = emit_object_vertices(&ringing, glam::Quat::IDENTITY);
    let differs = a
        .iter()
        .zip(b.iter())
        .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-4 || (p.pos[2] - q.pos[2]).abs() > 1e-4);
    assert!(differs, "the bell does not shake");
}

/// One object-model block entity's vertices, posed through the same emitter the frame path
/// uses (the texture rect is a unit square; only the geometry is checked here).
fn emit_object_vertices(inst: &BlockEntityInstance, orientation: glam::Quat) -> Vec<EntityVertex> {
    let mut out = Vec::new();
    let rect = [0, 0, 64, 64];
    emit_model_block_entity(
        &mut out,
        inst,
        glam::DVec3::ZERO,
        orientation,
        rect,
        &mut |_| rect,
    );
    out
}

#[test]
fn chest_and_shulker_textures_match_legacy_format() {
    let materials = [
        (ChestMaterial::Normal, "normal"),
        (ChestMaterial::Trapped, "trapped"),
        (ChestMaterial::Christmas, "christmas"),
    ];
    for (m, prefix) in materials {
        for (half, suffix) in [
            (ChestHalf::Single, ""),
            (ChestHalf::Left, "_left"),
            (ChestHalf::Right, "_right"),
        ] {
            assert_eq!(
                chest_texture(m, half).path(),
                format!("entity/chest/{prefix}{suffix}")
            );
        }
    }
    for half in [ChestHalf::Single, ChestHalf::Left, ChestHalf::Right] {
        assert_eq!(
            chest_texture(ChestMaterial::Ender, half).path(),
            "entity/chest/ender"
        );
    }

    let names = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    assert_eq!(shulker_texture(None).path(), "entity/shulker/shulker");
    for (i, name) in names.iter().enumerate() {
        assert_eq!(
            shulker_texture(Some(i as u8)).path(),
            format!("entity/shulker/shulker_{name}")
        );
    }
}

#[test]
#[should_panic]
fn shulker_texture_out_of_range_panics() {
    let _ = shulker_texture(Some(16));
}
