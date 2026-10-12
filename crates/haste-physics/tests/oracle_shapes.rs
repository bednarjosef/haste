//! Real 26.3 collision shapes from the oracle dump (`cache/oracle/26.3/block_shapes.json`).
//! Run with `cargo test -p haste-physics -- --ignored`.
use haste_core::{Aabb, DVec3};
use haste_physics::oracle::{default_oracle_dir, OracleBlocks};
use haste_physics::testing::TestWorld;
use haste_physics::{collide, collide_with_step, BlockPhysics, FluidKind};

fn load() -> OracleBlocks {
    OracleBlocks::load(&default_oracle_dir())
        .expect("oracle dump missing: needs cache/oracle/26.3 (oracles)")
}

fn player(x: f64, y: f64, z: f64) -> Aabb {
    Aabb::entity(DVec3::new(x, y, z), 0.6, 1.8)
}

fn close(a: DVec3, b: DVec3) {
    assert!((a - b).abs().max_element() < 1e-9, "{a:?} != {b:?}");
}

#[test]
#[ignore = "needs cache/oracle/26.3/block_shapes.json"]
fn real_shapes_slab_stairs_fence() {
    let o = load();
    let stone = o.state("minecraft:stone", &[]).unwrap();
    let slab = o
        .state(
            "minecraft:oak_slab",
            &[("type", "bottom"), ("waterlogged", "false")],
        )
        .unwrap();
    let stairs = o
        .state(
            "minecraft:oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        )
        .unwrap();
    let fence = o
        .state(
            "minecraft:oak_fence",
            &[
                ("north", "false"),
                ("south", "false"),
                ("east", "false"),
                ("west", "false"),
                ("waterlogged", "false"),
            ],
        )
        .unwrap();
    let m = DVec3::new(0.3, -0.0784, 0.0);

    let mut w = TestWorld::flat(0, stone);
    w.set(1, 0, 0, slab);
    close(
        collide_with_step(player(0.6, 0.0, 0.5), m, 0.6, true, &w, &o),
        DVec3::new(0.3, 0.5, 0.0),
    );

    // Stairs facing east: the high part is on the east side, the low step is the west half.
    let mut w = TestWorld::flat(0, stone);
    w.set(1, 0, 0, stairs);
    close(
        collide_with_step(player(0.6, 0.0, 0.5), m, 0.6, true, &w, &o),
        DVec3::new(0.3, 0.5, 0.0),
    );
    // From the low step (y 0.5) the high part (y 1.0) is another 0.5 step.
    close(
        collide_with_step(player(1.0, 0.5, 0.5), m, 0.6, true, &w, &o),
        DVec3::new(0.3, 0.5, 0.0),
    );
    // A full block next to the floor cannot be stepped.
    let mut w2 = TestWorld::flat(0, stone);
    w2.set(1, 0, 0, stone);
    close(
        collide_with_step(player(0.6, 0.0, 0.5), m, 0.6, true, &w2, &o),
        DVec3::new(0.1, 0.0, 0.0),
    );

    // Fence post: 1.5 tall, not steppable; falling onto it stops at 1.5.
    let mut w = TestWorld::flat(0, stone);
    w.set(0, 0, 0, fence);
    let post = o.collision_boxes_of(fence);
    assert!(post.iter().any(|b| b.max.y == 1.5), "fence shape {post:?}");
    close(
        collide(player(0.5, 3.0, 0.5), DVec3::new(0.0, -2.0, 0.0), &w, &o),
        DVec3::new(0.0, -1.5, 0.0),
    );
}

#[test]
#[ignore = "needs cache/oracle/26.3/block_properties.json"]
fn real_block_properties() {
    let o = load();
    let ice = o.state("minecraft:ice", &[]).unwrap();
    let blue = o.state("minecraft:blue_ice", &[]).unwrap();
    let soul = o.state("minecraft:soul_sand", &[]).unwrap();
    let honey = o.state("minecraft:honey_block", &[]).unwrap();
    assert_eq!(o.friction(ice), 0.98);
    assert_eq!(o.friction(blue), 0.989);
    assert_eq!(o.speed_factor(soul), 0.4);
    assert_eq!(o.jump_factor(honey), 0.5);
    let water = o.state("minecraft:water", &[("level", "0")]).unwrap();
    let f = o.fluid(water).unwrap();
    assert_eq!(f.kind, FluidKind::Water);
    assert!((f.height - 8.0 / 9.0).abs() < 1e-6);
    let lava3 = o.state("minecraft:lava", &[("level", "3")]).unwrap();
    assert!((o.fluid(lava3).unwrap().height - 5.0 / 9.0).abs() < 1e-6);
    let logged = o
        .state("minecraft:oak_stairs", &[("waterlogged", "true")])
        .unwrap();
    assert_eq!(o.fluid(logged).map(|f| f.kind), Some(FluidKind::Water));
}
