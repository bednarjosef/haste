//! The game loop (vanilla `Minecraft.runTick`'s thread): the 20 Hz schedule, the packet
//! drain that also runs on a packet's arrival between ticks, the HUD's JVM arriving on its
//! own thread, and the live option changes the vanilla screens make.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use haste_jvm::JvmHost;
use haste_protocol::haste_core::game::TICK_SECONDS;
use haste_protocol::haste_core::view::DirtyTracker;
use haste_protocol::haste_core::SectionPos;

use super::shared::{PackReload, Shared};
use super::skins;
use super::tick_once;
use super::tracked_world::TrackedWorld;
use crate::net::NetDriver;
use crate::ui::Ui;

/// Runs the 20 Hz game loop until the connection closes or `shared.stop` is set, then closes
/// the connection. Must run inside the tokio runtime's context (the connection's tasks).
/// `existing_ui`: a UI over an already-started JVM (menu mode joins through the title screen's
/// multiplayer list, whose JVM and UI stay for the whole run); without one the HUD's JVM starts
/// on its own thread in parallel with joining.
pub fn run(mut driver: NetDriver, shared: Arc<Shared>, existing_ui: Option<Ui>) {
    let tick = Duration::from_secs_f64(TICK_SECONDS);
    let mut next = Instant::now();
    let mut gone = Vec::new();
    // F3's tx/rx rates: the connection's counters sampled once per second (vanilla's
    // `Connection.getAverageSentPackets` is a 1 s window too).
    let mut stats_second = Instant::now();
    let mut last_in = 0u64;
    let mut last_out = 0u64;
    // The JVM for the HUD takes tens of seconds to start: on its own thread, in parallel with
    // joining (the connection would time out if we waited here). One warning, no retries: a
    // client without a JDK or cache keeps playing without a HUD. With the HUD off (`--hud
    // off`, vanilla's F1), it is never started.
    let mut jvm = existing_ui
        .map(|ui| {
            // The menu already ticked this Ui; the game thread takes it over from here.
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(Some(HudArrival::Menu(ui)));
            HudSlot { rx: Some(rx) }
        })
        .unwrap_or_else(|| {
            if shared.hud_enabled.load(Ordering::Relaxed) {
                spawn_jvm(&shared)
            } else {
                HudSlot { rx: None }
            }
        });
    // The server pack set version this thread last applied (starts at the
    // connection's: the configuration-phase packs were applied at session build).
    let mut server_packs_version = driver.connection().server_packs_version();
    // Packets are drained on arrival, not only at ticks: vanilla runs the packet
    // queue every frame (`Minecraft.runTick` -> `processQueuedPackets`), so a
    // `container_set_slot` shows on the frame after it arrives instead of up to
    // one tick (50 ms) later. Between ticks the thread sleeps on the wake below
    // and only drains packets (no game logic, no input): the 20 Hz tick order
    // and rate are unchanged.
    let mut drained_wakes = 0u64;
    let mut failure: Option<String> = None;
    // The block entity render data's extractor (per-tick caches; see the module).
    let mut extractor = crate::app::block_entity_extract::BlockEntityExtractor::new();
    let reason = loop {
        if shared.stop.load(Ordering::Relaxed) {
            break "left the game".to_string();
        }
        if let Some(f) = failure.take() {
            break f;
        }
        if let Err(e) = tick_once(
            &mut driver,
            &shared,
            &mut gone,
            &mut jvm,
            &mut server_packs_version,
            &mut extractor,
        ) {
            break format!("tick failed: {e:#}");
        }
        if stats_second.elapsed() >= Duration::from_secs(1) {
            let s = driver.stats();
            shared.debug_tx.store(
                ((s.packets_out - last_out) as f64).to_bits(),
                Ordering::Relaxed,
            );
            shared.debug_rx.store(
                ((s.packets_in - last_in) as f64).to_bits(),
                Ordering::Relaxed,
            );
            let c = &driver.session.counters;
            shared
                .chunks_loaded
                .store(c.chunks_loaded, Ordering::Relaxed);
            shared
                .chunks_unloaded
                .store(c.chunks_unloaded, Ordering::Relaxed);
            last_out = s.packets_out;
            last_in = s.packets_in;
            stats_second = Instant::now();
        }
        if driver.is_closed() {
            break driver
                .connection()
                .close_reason()
                .unwrap_or_else(|| "connection closed".into());
        }
        next += tick;
        let now = Instant::now();
        if next > now {
            // Sleep until the next tick, but wake the moment a packet arrives
            // (the reader bumps the counter and notifies). The counter, not the
            // notify, carries the signal, so a wake that arrived while we were
            // awake is still seen on the next lock.
            while Instant::now() < next && failure.is_none() {
                let left = next.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let mut seen = shared.packet_wake.0.lock();
                if *seen == drained_wakes {
                    let _ = shared.packet_wake.1.wait_for(&mut seen, left);
                }
                let arrived = *seen != drained_wakes;
                drop(seen);
                if arrived {
                    if let Err(e) =
                        drain_packets(&mut driver, &shared, &mut gone, &mut server_packs_version)
                    {
                        failure = Some(format!("tick failed: {e:#}"));
                        break;
                    }
                    drained_wakes = *shared.packet_wake.0.lock();
                }
            }
        } else if now - next > tick * 10 {
            // Far behind (debugger, suspend): skip instead of bursting ticks.
            next = now;
        }
    };
    tracing::info!("game thread stopping: {reason}");
    *shared.closed.lock() = Some(reason);
    driver.disconnect();
}

/// Applies the option changes a vanilla screen made to the running client: the shared live
/// options (FOV, GUI scale, mouse sensitivity, volumes, ... the render thread reads them), the
/// render distance the mesher uses (capped by the server's view distance, like at join) and the
/// sound volumes. One info log line per applied option, the way vanilla applies each change the
/// moment the player makes it (`OptionInstance.set` -> `onValueUpdate`).
pub(super) fn apply_option_changes(
    shared: &Shared,
    driver: &NetDriver,
    changes: Vec<(String, String)>,
) {
    let mut sound_changed = false;
    let mut render_distance = None;
    let mut applied = false;
    {
        let mut live = shared.live_options.lock();
        for (key, value) in changes {
            if !live.apply_one(&key, &value) {
                continue;
            }
            tracing::info!("applied option {key} = {value}");
            applied = true;
            sound_changed |= key.starts_with("soundCategory_");
            if key == "renderDistance" {
                render_distance = Some(live.render_distance as i32);
            }
        }
    }
    if !applied {
        return;
    }
    // Bumped after the lock is released: a reader that sees the new version will see the new
    // options when it locks.
    shared.options_version.fetch_add(1, Ordering::Relaxed);
    if let Some(rd) = render_distance {
        // The server's view distance still caps what we mesh, like at join.
        shared.view.lock().view_distance = driver.session.view_distance.min(rd);
    }
    if sound_changed {
        let volumes = shared.live_options.lock().sound;
        if let Some(sounds) = shared.sound.lock().as_mut() {
            sounds.set_volumes(&volumes);
        }
    }
}

/// Starts the JVM for the HUD on its own thread (tens of seconds; the connection would time/// out if the game thread waited). On failure: one warning and `shared.hud_failed` set, then
/// the client keeps running without a HUD.
fn spawn_jvm(shared: &Arc<Shared>) -> HudSlot {
    let (tx, rx) = std::sync::mpsc::channel();
    let shared = shared.clone();
    std::thread::Builder::new()
        .name("haste-jvm".into())
        .spawn(move || {
            let t = Instant::now();
            let result =
                haste_jvm::JvmConfig::from_cache(&haste_protocol::haste_core::paths::cache_dir())
                    .map_err(anyhow::Error::from)
                    .map(|mut cfg| {
                        // the same game directory the menu's JVM and the asset pack
                        // selection use (vanilla's options.txt and resourcepacks/ live there)
                        cfg.game_dir = crate::app::game_dir();
                        cfg
                    })
                    .and_then(JvmHost::start);
            match result {
                Ok(host) => {
                    tracing::info!(
                        "HUD JVM ready in {:.1} s: {}",
                        t.elapsed().as_secs_f32(),
                        host.proxy_summary()
                    );
                    let _ = tx.send(Some(HudArrival::Fresh(host)));
                }
                Err(e) => {
                    tracing::warn!("no HUD: the JVM did not start: {e:#}");
                    shared.hud_failed.store(true, Ordering::Relaxed);
                }
            }
        })
        .expect("spawning the JVM thread");
    HudSlot { rx: Some(rx) }
}

/// What the HUD's slot delivers: a freshly started JVM, or menu mode's already-built UI.
pub(super) enum HudArrival {
    /// The JVM thread started one (spawn_jvm): the `Ui` is built around it here.
    Fresh(JvmHost),
    /// Menu mode's UI (title screen, registries already loaded in Java).
    Menu(Ui),
}

/// The JVM (or menu UI) arriving on its own thread: `take()` once, at the start of a game tick.
pub(super) struct HudSlot {
    rx: Option<std::sync::mpsc::Receiver<Option<HudArrival>>>,
}

/// Takes `item` from a slot that a background thread fills once: `None` while it is still
/// working (the slot stays), the item when it arrived, `None` for good after a failure or once
/// taken (the slot is dropped, so later calls are `None` without touching the channel).
pub(super) fn take_slot<T>(rx: &mut Option<std::sync::mpsc::Receiver<Option<T>>>) -> Option<T> {
    let r = rx.take()?;
    match r.try_recv() {
        Ok(item) => item,
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            *rx = Some(r);
            None
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
    }
}

impl HudSlot {
    /// The JVM (or menu UI), if it arrived since the last tick. `None` while it is still
    /// starting (or after it failed or was taken).
    pub(super) fn take_ui(&mut self) -> Option<HudArrival> {
        take_slot(&mut self.rx)
    }
}

/// Drains the packets that arrived since the last call into the session, world,
/// entities and UI, and publishes the dirty sections. Runs both at the start of
/// a tick and whenever a packet wakes the game thread between ticks (vanilla
/// runs the packet queue every frame, `Minecraft.runTick` ->
/// `packetProcessor.processQueuedPackets`, before the tick's own work).
pub(super) fn drain_packets(
    driver: &mut NetDriver,
    shared: &Shared,
    gone: &mut Vec<SectionPos>,
    server_packs_version: &mut u64,
) -> anyhow::Result<()> {
    let mut world = shared.world.write();
    let mut entities = shared.entities.write();
    let mut tracked = TrackedWorld {
        world: &mut world,
        gone,
    };
    let ui = shared.ui.lock().clone();
    driver.receive(&mut tracked, &mut *entities, ui.as_ref())?;
    // The server's view distance (login, then `set_chunk_cache_radius`) caps what we mesh,
    // like vanilla's `Options.setServerRenderDistance`. Read the option before taking the
    // view lock so the two locks are never held together.
    let render_distance = shared.live_options.lock().render_distance as i32;
    let view_distance = driver.session.view_distance.min(render_distance);
    {
        let mut view = shared.view.lock();
        if view.view_distance != view_distance {
            view.view_distance = view_distance;
        }
    }
    drop(entities);
    skins::request_profiles(&shared.skins, &mut driver.session.profiles);
    let mut dirty = world.take_dirty_sections();
    // The sections of forgotten chunks ride the same dirty set: the render thread's
    // mesh workers find no chunk for them and free the resident meshes (vanilla drops
    // a chunk's render data with the chunk, `ClientChunkCache.drop`).
    dirty.append(gone);
    if !dirty.is_empty() {
        shared.dirty.lock().append(&mut dirty);
        // Bumped with every world change so the render thread's per-tick caches (the weather
        // columns' heights and precipitation) know to refresh.
        shared.world_version.fetch_add(1, Ordering::Relaxed);
    }
    // The server's live pack set changed (vanilla ticks its ServerPackManager the
    // same way: a download finished, a re-push replaced one, or a pop arrived):
    // apply the whole set on top of the stack and reload, like the pack screen's
    // Done. The connection's reply path already sent statuses.
    let version = driver.connection().server_packs_version();
    if version != *server_packs_version {
        *server_packs_version = version;
        let packs = driver.connection().server_packs();
        for pack in &packs {
            tracing::info!(
                "server resource pack {} downloaded to {}",
                pack.id,
                pack.path.display()
            );
        }
        *shared.applied_server_packs.lock() = packs.iter().map(|p| p.path.clone()).collect();
        let ids = crate::app::selected_resource_packs();
        let server = shared.applied_server_packs.lock().clone();
        *shared.resource_pack_reload.lock() = Some(PackReload { ids, server });
    }
    Ok(())
}
