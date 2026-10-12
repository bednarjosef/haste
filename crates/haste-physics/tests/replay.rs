//! Trace replay. The `#[ignore]` test replays every trace in `cache/oracle/26.3/traces/`
//! (recorded from the real game by WS8) and reports the first divergence per trace:
//! `cargo test -p haste-physics --test replay -- --ignored --nocapture`.
use haste_core::input::InputState;
use haste_core::DVec3;
use haste_physics::oracle::{default_oracle_dir, OracleBlocks};
use haste_physics::replay::{
    load_trace, replay, FlatWorld, InputAlignment, Trace, TraceInput, TraceTick, DEFAULT_TOLERANCE,
};
use haste_physics::testing::{states::*, TestShapes, TestWorld};
use haste_physics::{tick, PlayerAttributes, PlayerState};

/// Records a trace with our own tick, in the FORMATS.md v1 shape.
fn record(world: &TestWorld, inputs: &[InputState]) -> Trace {
    let shapes = TestShapes::default();
    let attrs = PlayerAttributes::default();
    let mut s = PlayerState::at(DVec3::new(0.5, 0.0, 0.5));
    s.on_ground = true;
    let tick_of = |i: usize, s: &PlayerState, inp: &InputState| TraceTick {
        tick: i as u64,
        input: TraceInput {
            forward: inp.forward as i32 as f32 - inp.back as i32 as f32,
            strafe: inp.left as i32 as f32 - inp.right as i32 as f32,
            jump: inp.jump,
            sneak: inp.sneak,
            sprint: inp.sprint,
            yaw: Some(inp.yaw),
            pitch: Some(inp.pitch),
        },
        pos: s.pos.to_array(),
        velocity: Some(s.velocity.to_array()),
        on_ground: Some(s.on_ground),
        pose: Some(s.pose.name().to_string()),
        yaw: None,
        pitch: None,
    };
    let mut ticks = vec![tick_of(0, &s, &InputState::default())];
    for (i, inp) in inputs.iter().enumerate() {
        tick(&mut s, inp, &attrs, world, &shapes, &shapes);
        ticks.push(tick_of(i + 1, &s, inp));
    }
    Trace {
        version: 1,
        name: "synthetic".into(),
        world: None,
        player: None,
        ticks,
    }
}

fn inputs() -> Vec<InputState> {
    let mut v = Vec::new();
    for i in 0..200 {
        v.push(InputState {
            forward: i % 50 < 40,
            left: (60..90).contains(&i),
            jump: i % 37 < 3,
            sprint: i > 20,
            sneak: (120..140).contains(&i),
            yaw: (i as f32 * 1.7) % 360.0 - 180.0,
            ..Default::default()
        });
    }
    v
}

fn terrain() -> TestWorld {
    let mut w = TestWorld::flat(0, STONE);
    w.fill((3, 0, -10), (5, 0, 10), BOTTOM_SLAB);
    w.fill((-8, 0, 4), (8, 1, 4), STONE);
    w.fill((-6, 0, -6), (-4, 0, -4), ICE);
    w
}

#[test]
fn own_trace_replays_without_divergence() {
    let w = terrain();
    let trace = record(&w, &inputs());
    let shapes = TestShapes::default();
    let r = replay(
        &trace,
        &PlayerAttributes::default(),
        &w,
        &shapes,
        &shapes,
        DEFAULT_TOLERANCE,
        InputAlignment::SameTick,
    );
    assert_eq!(r.first_divergence, None, "{r}");
    assert_eq!(r.max_error, 0.0);
    assert_eq!(r.ticks, 201);
    assert_eq!(r.on_ground_mismatches, 0);
}

#[test]
fn perturbed_trace_reports_first_divergence() {
    let w = terrain();
    let mut trace = record(&w, &inputs());
    trace.ticks[57].pos[0] += 1e-3;
    let shapes = TestShapes::default();
    let r = replay(
        &trace,
        &PlayerAttributes::default(),
        &w,
        &shapes,
        &shapes,
        DEFAULT_TOLERANCE,
        InputAlignment::SameTick,
    );
    let d = r.first_divergence.expect("divergence");
    assert_eq!(d.tick, 57);
    assert!((d.error - 1e-3).abs() < 1e-9);
    assert!((r.max_error - 1e-3).abs() < 1e-9);
}

#[test]
fn trace_json_parses() {
    // The example from crates/haste-testkit/FORMATS.md (WS8), trimmed.
    let json = r#"{"version":1,"minecraft":"26.3","name":"walk-10s",
        "world":{"seed":12345,"spawn":[0.5,-60.0,0.5],"dimension":"minecraft:overworld"},
        "player":{"yaw":0.0,"pitch":0.0,"initial_pos":[8.5,-60.0,8.5]},
        "ticks":[{"tick":0,"input":{"forward":1.0,"strafe":0.0,"jump":false,"sneak":false,"sprint":false},
        "pos":[8.5,-60.0,8.5],"velocity":[0.0,0.0,0.0],"on_ground":true,"pose":"standing","dimensions":[0.6,1.8]}]}"#;
    let t: Trace = serde_json::from_str(json).unwrap();
    assert_eq!(t.ticks.len(), 1);
    assert_eq!(t.player.unwrap().initial_pos, Some([8.5, -60.0, 8.5]));
}

/// Default superflat (bedrock, 2 dirt, grass; surface y = -60). Assumption: the traces are
/// recorded on a default superflat world (FORMATS.md's example spawn is y = -60).
fn superflat(o: &OracleBlocks) -> FlatWorld {
    let bedrock = o.state("minecraft:bedrock", &[]).expect("bedrock");
    let dirt = o.state("minecraft:dirt", &[]).expect("dirt");
    let grass = o
        .state("minecraft:grass_block", &[("snowy", "false")])
        .expect("grass");
    FlatWorld {
        bottom: -64,
        layers: vec![bedrock, dirt, dirt, grass],
        min_y: -64,
        height: 384,
    }
}

#[test]
#[ignore = "needs cache/oracle/26.3/traces/*.json (WS8 recorder) and the oracle dumps"]
fn replay_all_recorded_traces() {
    let dir = default_oracle_dir().join("traces");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    if paths.is_empty() {
        eprintln!(
            "no traces in {}: replay harness ready, nothing to replay yet",
            dir.display()
        );
        return;
    }
    let o = OracleBlocks::load(&default_oracle_dir()).expect("oracle dumps");
    let world = superflat(&o);
    let attrs = PlayerAttributes::default();
    let mut diverged = 0;
    for p in &paths {
        let trace = match load_trace(p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}: {e:#}", p.display());
                diverged += 1;
                continue;
            }
        };
        let r = replay(
            &trace,
            &attrs,
            &world,
            &o,
            &o,
            DEFAULT_TOLERANCE,
            InputAlignment::SameTick,
        );
        eprintln!("{r}");
        if r.first_divergence.is_some() {
            let alt = replay(
                &trace,
                &attrs,
                &world,
                &o,
                &o,
                DEFAULT_TOLERANCE,
                InputAlignment::PreviousTick,
            );
            eprintln!("  (with PreviousTick alignment: {alt})");
            diverged += 1;
        }
    }
    assert_eq!(
        diverged,
        0,
        "{diverged} of {} traces diverge (details above)",
        paths.len()
    );
}
