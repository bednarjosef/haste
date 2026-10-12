//! Headless join (client-app). No window.
//!
//! `haste --headless --server 127.0.0.1:25565 --seconds 60`: joins (offline account
//! `--username`), runs the 20 Hz network tick for `--seconds` into the real
//! `haste_world::World` and `haste_entity::EntityStore`, prints chunks, non-empty sections,
//! entities, player position, packets/s and unknown packet ids once per second, then leaves.
//! Exit status is 0 only if the connection stayed up the whole time, the login packet arrived,
//! at least one chunk loaded and every chunk decoded.

use std::time::{Duration, Instant};

use haste_protocol::haste_core::game::TICK_SECONDS;
use haste_protocol::haste_core::BlockPos;
use haste_protocol::{Account, ConnectOptions};
use haste_world::World;

use crate::net::{self, EntitySink, NetDriver, WorldSink};
use crate::Args;

/// Runs the headless join and returns an error if it did not hold.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let address = args
        .server
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--headless needs --server <host[:port]>"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()?;
    rt.block_on(run_async(address, args.username.clone(), args.seconds))
}

async fn run_async(address: String, username: String, seconds: u64) -> anyhow::Result<()> {
    let data = net::load_game_data()?;
    let opts = ConnectOptions::new(address.clone(), Account::Offline { username });
    let t0 = Instant::now();
    let mut driver = NetDriver::connect(opts).await?;
    let regs = driver.connection().registries();
    let biomes = net::synced_biome_count(regs);
    let entity_types = regs
        .entries(&haste_protocol::haste_core::Identifier::minecraft(
            "entity_type",
        ))
        .map_or_else(|| data.registries.len("minecraft:entity_type"), |e| e.len());
    let mut world = World::new_with_biomes(
        driver.login().dimension.clone(),
        data.clone(),
        biomes.unwrap_or(64),
    );
    let mut entities = net::new_entity_store(entity_types);
    let login = driver.login();
    println!(
        "joined {address} as {} in {:.0} ms: entity {:?}, {:?}, dimension {} (min_y {}, height {}), view distance {}, biomes {} ({} direct bits)",
        driver.connection().profile().name,
        t0.elapsed().as_secs_f64() * 1000.0,
        login.entity_id,
        login.game_mode,
        login.dimension.name,
        login.dimension.min_y,
        login.dimension.height,
        login.view_distance,
        biomes.map_or("not synced".into(), |n| n.to_string()),
        world.biome_bits(),
    );

    // HASTE_TICK_RATE (test hook): run the client at the rate a slowed test server runs at
    // (`/tick rate N`), so tick-counted checks such as break progress compare like for like.
    let tick_seconds = std::env::var("HASTE_TICK_RATE")
        .ok()
        .and_then(|r| r.parse::<f64>().ok())
        .map_or(TICK_SECONDS, |r| 1.0 / r);
    let mut interval = tokio::time::interval(Duration::from_secs_f64(tick_seconds));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let total_ticks = seconds * 20;
    let mut last_packets = 0u64;
    let mut failure: Option<String> = None;
    let mut walk = crate::player::script::Runner::from_env()?;
    for tick in 1..=total_ticks {
        interval.tick().await;
        let mut input = Default::default();
        if let Some(w) = walk.as_mut() {
            let (i, cmd) = w.before_tick(tick, &mut driver.session.player, &world);
            input = i;
            if let Some(c) = cmd {
                driver.queue(c);
            }
        }
        let ticked = driver.tick(&input, &mut world, &mut entities);
        if walk
            .as_mut()
            .is_some_and(|w| w.after_tick(tick, &driver.session.player))
        {
            break;
        }
        if let Err(e) = ticked {
            failure = Some(format!("tick failed: {e:#}"));
            break;
        }
        if driver.is_closed() {
            // Drain what is left so the disconnect reason is applied.
            let _ = driver.receive(&mut world, &mut entities);
            failure = Some(
                driver
                    .connection()
                    .close_reason()
                    .unwrap_or_else(|| "connection closed".into()),
            );
            break;
        }
        if tick % 20 == 0 && walk.is_none() {
            let stats = driver.stats();
            let pps = stats.packets_in - last_packets;
            last_packets = stats.packets_in;
            let unknown: Vec<String> = stats
                .unknown
                .iter()
                .map(|(id, name, n)| {
                    format!("{}={n}", name.as_deref().unwrap_or(&format!("#{id}")))
                })
                .collect();
            let p = driver.session.player.state.pos;
            let c = &driver.session.counters;
            println!(
                "[{:>3}s] chunks {:>4} sections {:>5} entities {:>4} pos ({:.2}, {:.2}, {:.2}) packets/s {:>5} chunk errors {} unknown [{}]{}",
                tick / 20,
                world.chunk_count(),
                world.non_empty_sections(),
                EntitySink::len(&entities),
                p.x,
                p.y,
                p.z,
                pps,
                c.chunk_errors,
                unknown.join(", "),
                if stats.decode_failures.is_empty() {
                    String::new()
                } else {
                    format!(
                        " decode failures [{}]",
                        stats
                            .decode_failures
                            .iter()
                            .map(|(n, c, _)| format!("{n}={c}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            );
        }
    }

    let stats = driver.stats();
    let counters = driver.session.counters.clone();
    let chunks = world.chunk_count();
    println!("--- summary after {:.1} s", t0.elapsed().as_secs_f64());
    println!(
        "packets in {} ({} bytes), out {}; chunks loaded {} (unloaded {}, refused by world {}), now loaded {} with {} non-empty sections, entities {}, teleports {}",
        stats.packets_in,
        stats.bytes_in,
        stats.packets_out,
        counters.chunks_loaded,
        counters.chunks_unloaded,
        counters.chunk_errors,
        chunks,
        world.non_empty_sections(),
        EntitySink::len(&entities),
        counters.teleports
    );
    {
        // Sanity check of the decode: the column of blocks under the player.
        use haste_protocol::haste_core::view::BlockView;
        let p = driver.session.player.state.pos.floor().as_ivec3();
        let column: Vec<String> = (1..=5)
            .map(|d| {
                let s = world.block_state(BlockPos::new(p.x, p.y - d, p.z));
                data.blocks.block_of(s).to_string()
            })
            .collect();
        println!("blocks below the player: {}", column.join(", "));
        let light: Vec<String> = (-1..=3)
            .map(|d| {
                let b = BlockPos::new(p.x, p.y + d, p.z);
                format!(
                    "y{} sky {} block {}",
                    b.y,
                    world.sky_light(b),
                    world.block_light(b)
                )
            })
            .collect();
        println!("light at the player: {}", light.join(", "));
    }
    if let Some(e) = &counters.last_chunk_error {
        println!("last chunk error: {e}");
    }
    if stats.unknown.is_empty() {
        println!("unknown packet ids: none");
    } else {
        println!("unknown packet ids (no decoder yet):");
        for (id, name, n) in &stats.unknown {
            println!("  {id:>3} {:<40} {n}", name.as_deref().unwrap_or("?"));
        }
    }
    if !stats.decode_failures.is_empty() {
        println!("decode failures:");
        for (name, n, err) in &stats.decode_failures {
            println!("  {name:<40} {n}  last error: {err}");
        }
    }
    driver.disconnect();
    // Give the writer a moment to flush and close the socket.
    tokio::time::sleep(Duration::from_millis(100)).await;

    if let Some(f) = failure {
        anyhow::bail!("disconnected before {seconds} s: {f}");
    }
    if !counters.got_login {
        anyhow::bail!("no login packet");
    }
    if chunks == 0 && counters.chunks_loaded == 0 {
        anyhow::bail!("no chunk received");
    }
    if counters.chunk_errors > 0 {
        anyhow::bail!("{} chunks failed to decode", counters.chunk_errors);
    }
    println!("ok: stayed connected {seconds} s");
    Ok(())
}
