//! M1: headless join. Owner: WS1.
//!
//! `haste --headless --server 127.0.0.1:25565 --seconds 60`: joins (offline
//! account `--username`), runs the 20 Hz network tick for `--seconds`,
//! prints chunks, non-empty sections, entities, player position, packets/s
//! and unknown packet ids once per second, then leaves. Exit status is 0 only
//! if the connection stayed up the whole time, the login packet arrived and
//! at least one chunk loaded.
//!
//! `net` lives in this file's directory but is declared here until the lead
//! adds `mod net;` to main.rs.

use std::time::{Duration, Instant};

use haste_protocol::haste_core::game::TICK_SECONDS;
use haste_protocol::{Account, ConnectOptions};

use crate::net::{EntitySink, NetDriver, StubEntities, StubWorld, WorldSink};
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
    let opts = ConnectOptions::new(address.clone(), Account::Offline { username });
    let t0 = Instant::now();
    let biome_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(64));
    let bc = biome_count.clone();
    let make_world = Box::new(
        move |dim: &haste_protocol::haste_core::game::DimensionInfo| {
            StubWorld::new(dim, bc.load(std::sync::atomic::Ordering::Relaxed))
        },
    );
    let mut driver: NetDriver<StubWorld, StubEntities> =
        NetDriver::connect(opts, make_world, StubEntities::default()).await?;
    // The stub world needs the synced biome registry size for direct biome palettes.
    let biomes = driver
        .connection()
        .registries()
        .entries(&haste_protocol::haste_core::Identifier::minecraft(
            "worldgen/biome",
        ))
        .map_or(64, |e| e.len());
    biome_count.store(biomes, std::sync::atomic::Ordering::Relaxed);
    driver.world = StubWorld::new(&driver.login.dimension, biomes);
    println!(
        "joined {address} as {} in {:.0} ms: entity {:?}, {:?}, dimension {} (min_y {}, height {}), view distance {}",
        driver.connection().profile().name,
        t0.elapsed().as_secs_f64() * 1000.0,
        driver.login.entity_id,
        driver.login.game_mode,
        driver.login.dimension.name,
        driver.login.dimension.min_y,
        driver.login.dimension.height,
        driver.login.view_distance,
    );

    let mut interval = tokio::time::interval(Duration::from_secs_f64(TICK_SECONDS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let total_ticks = seconds * 20;
    let mut last_packets = 0u64;
    let mut failure: Option<String> = None;
    for tick in 1..=total_ticks {
        interval.tick().await;
        if let Err(e) = driver.tick() {
            failure = Some(format!("tick failed: {e:#}"));
            break;
        }
        if driver.is_closed() {
            // Drain what is left so the disconnect reason is applied.
            let _ = driver.tick();
            failure = Some(
                driver
                    .connection()
                    .close_reason()
                    .unwrap_or_else(|| "connection closed".into()),
            );
            break;
        }
        if tick % 20 == 0 {
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
            let p = driver.player.pos;
            println!(
                "[{:>3}s] chunks {:>4} sections {:>5} entities {:>4} pos ({:.2}, {:.2}, {:.2}) packets/s {:>5} unknown [{}]{}",
                tick / 20,
                driver.world.chunk_count(),
                driver.world.non_empty_sections(),
                driver.entities.len(),
                p.x,
                p.y,
                p.z,
                pps,
                unknown.join(", "),
                if stats.decode_failures.is_empty() {
                    String::new()
                } else {
                    format!(" decode failures [{}]", stats.decode_failures.iter().map(|(n, c, _)| format!("{n}={c}")).collect::<Vec<_>>().join(", "))
                },
            );
        }
    }

    let stats = driver.stats();
    let counters = driver.counters.clone();
    let chunks = driver.world.chunk_count();
    println!("--- summary after {:.1} s", t0.elapsed().as_secs_f64());
    println!(
        "packets in {} ({} bytes), out {}; chunks loaded {} (unloaded {}, refused by world {}), teleports {}",
        stats.packets_in, stats.bytes_in, stats.packets_out, counters.chunks_loaded, counters.chunks_unloaded, counters.chunk_errors, counters.teleports
    );
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
    println!("ok: stayed connected {seconds} s");
    Ok(())
}
