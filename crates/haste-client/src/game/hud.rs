//! The tick's HUD steps: the JVM (or menu UI) arriving on its own thread, and the HUD and
//! chat's tick (screens, inventory and option changes, the pack prompt) with this tick's UI
//! output packets.

use std::sync::Arc;
use std::time::{Duration, Instant};

use haste_protocol::ServerboundPlay;

use super::run::{apply_option_changes, HudArrival, HudSlot};
use super::shared::{PackReload, Shared};
use crate::net::{NetDriver, WorldSink};
use crate::ui::{HudWarmup, Ui};

/// Once the JVM arrives (the session joined before the game thread started): build the
/// `Ui` and give the JVM the registries, tags and the local player. Menu mode's UI arrives
/// pre-built (its JVM opened the title screen); only `joined` runs for it. A fresh JVM's UI
/// then warms up on the game thread's ticks before it goes live (see [`HudWarmup`]); a menu
/// UI is already warm and goes live at once.
pub(super) fn take_arrival(driver: &mut NetDriver, shared: &Arc<Shared>, jvm: &mut HudSlot) {
    if let Some(ui) = jvm.take_ui() {
        let slot = shared.ui.lock();
        if slot.is_none() {
            drop(slot);
            let fresh = matches!(ui, HudArrival::Fresh(_));
            let ui = match ui {
                HudArrival::Fresh(host) => Ui::new(host),
                HudArrival::Menu(ui) => ui,
            };
            // The UI's input events (a container click) send at once from here on, the
            // way vanilla's `Connection.send` runs inside the input handler.
            super::run::wire_immediate_sender(&ui, shared);
            let t = Instant::now();
            match ui.joined(
                driver.connection(),
                driver.login(),
                driver.session.player.game_mode,
            ) {
                Ok(()) => {
                    tracing::info!(
                        "HUD joined in {:.0} ms; the vanilla HUD and chat are live",
                        t.elapsed().as_secs_f64() * 1000.0
                    );
                    // The boot backlog goes in before the warm-up: its state (chat, tab
                    // list, boss bars) is what the HUD renders, and the warm-up's frames
                    // must render it or they miss the classes its rendering loads (the
                    // first real frame paid that: ~200-300 ms on the render thread).
                    catch_up(driver, shared, &ui);
                    if fresh {
                        jvm.warmup = Some(HudWarmup::new(ui));
                    } else {
                        publish(shared, ui);
                    }
                }
                Err(e) => tracing::warn!("no HUD: joining the UI failed: {e:#}"),
            }
        }
    }
    // The fresh HUD's warm-up, one budgeted batch per tick; once warm, the JVM's boot GCs
    // and class loading are behind it and the render thread never stalls on them.
    if let Some(mut warmup) = jvm.warmup.take() {
        if warmup.step(WARMUP_TICK_BUDGET) {
            // Packets for the UI kept arriving while it warmed (it was not published, so
            // the driver backlogged them): hand them over before it goes live.
            let ui = warmup.into_ui();
            catch_up(driver, shared, &ui);
            publish(shared, ui);
        } else {
            jvm.warmup = Some(warmup);
        }
    }
}

/// How long one tick's warm-up batch may run (the tick itself has 50 ms; warm-up that
/// overruns it once — the class-loading first frame — just borrows from the next tick).
const WARMUP_TICK_BUDGET: Duration = Duration::from_millis(12);

/// Puts a joined (and, for a fresh JVM, warmed) UI live.
fn publish(shared: &Shared, ui: Ui) {
    *shared.ui.lock() = Some(ui);
}

/// Hands the UI the play packets that arrived while the JVM booted, in order, so a fresh
/// join's tab list, boss bars and scoreboard are not lost.
fn catch_up(driver: &mut NetDriver, shared: &Shared, ui: &Ui) {
    let backlog = driver.take_ui_backlog();
    if backlog.is_empty() {
        return;
    }
    tracing::info!(
        "HUD catching up on {} packets from the JVM boot",
        backlog.len()
    );
    let world = shared.world.read();
    for p in backlog {
        ui.handle(&p, &|pos| WorldSink::block_state_at(&*world, pos));
    }
}

/// The HUD and chat's tick, then this tick's UI output (packets vanilla code sent, chat and
/// commands typed) and a pushed pack waiting for the player (vanilla's `PackConfirmScreen`
/// in play).
pub(super) fn tick_ui(driver: &mut NetDriver, shared: &Shared, ui_out: &mut Vec<ServerboundPlay>) {
    if let Some(ui) = shared.ui.lock().as_ref() {
        // The mount HUD's values for this tick (the vehicle's hearts and the charged jump bar),
        // from the player's ride, the entity store and the synced attributes.
        ui.set_mount_hud(driver.session.mount_hud(&*shared.entities.read()));
        // The damage cooldown (vanilla `LivingEntity.damageCooldownTime`): the HUD's heart
        // blink gates on it, so it is mirrored every tick.
        ui.set_damage_cooldown(driver.session.damage_cooldown as i32);
        // The hotbar authority is the player (the client picks with 1-9 and the wheel and tells
        // the server itself; `set_selected_slot` also marks it sent so the UI does not send it
        // a second time). The server's `set_held_slot` reaches both sides alike.
        ui.set_selected_slot(driver.session.player.held_slot);
        // The spectator menu's head textures (the entity store's resolved skins), no-op
        // while the menu is closed.
        ui.refresh_spectator_skins(&shared.entities.read());
        // A use opened a screen (command block, writable book): the UI opens it, gated like
        // vanilla's `Player.canUseGameMasterBlocks` (creative + operator level >= 2) for the
        // command block.
        match driver.session.player.take_used_screen() {
            Some(crate::player::interact::UsedScreen::CommandBlock(pos)) => {
                let (creative, level) = ui.game_master_state();
                if creative && level >= 2 {
                    // The editor's entity is built from the real state (the use itself opens
                    // no packet the block state would ride on; vanilla's chunk has it).
                    let state = WorldSink::block_state_at(&*shared.world.read(), pos);
                    ui.record_block_state(pos, state);
                    ui.open_used_command_block(pos);
                }
            }
            Some(crate::player::interact::UsedScreen::ItemGui(hand)) => ui.open_item_gui(hand),
            None => {}
        }
        ui.tick();
        // Inventory edits vanilla Java code predicted client-side (creative slot sets, menu
        // clicks, pick block): the server does not echo them back, so the client takes them
        // from the Java mirror. Without this the hand keeps the old item (the reported
        // creative-inventory bug: the hotbar shows the item, the hand does not).
        for (slot, stack) in ui.drain_inventory_changes() {
            driver.session.player.set_inventory_slot(slot, &stack);
        }
        // Every option a vanilla screen changed this tick reaches the running client live
        // (vanilla's `OptionInstance.onValueUpdate` callbacks, reported through the bridge).
        let changes = ui.drain_option_changes();
        if !changes.is_empty() {
            apply_option_changes(shared, driver, changes);
        }
        // The pack screen's Done: vanilla reloads its resource manager (Minecraft.
        // reloadResourcePacks); the renderer rebuilds from the new selection on its next
        // frame (AtlasBuilder + bake + GUI textures read the new asset stack).
        if let Some(ids) = ui.take_resource_pack_selection() {
            tracing::info!("resource pack selection changed: [{}]", ids.join(", "));
            let server = shared.applied_server_packs.lock().clone();
            *shared.resource_pack_reload.lock() = Some(PackReload { ids, server });
        }
        ui.drain_outgoing(ui_out);
    }
    // A pushed pack waiting for the player (vanilla's `PackConfirmScreen` in play).
    shared
        .pack_prompt
        .lock()
        .tick(driver.connection(), shared.ui.lock().as_ref());
    for p in std::mem::take(ui_out) {
        driver.queue(p);
    }
}

/// The attack indicator's inputs for this tick (the crosshair bar and the hotbar icon,
/// `Hud.extractCrosshair` / `extractItemHotbar`): published after the hand's tick so the HUD sees
/// this tick's charge like vanilla's render, plus whether the crosshair picks a living, alive
/// entity (the full icon's condition).
pub(super) fn publish_attack_indicator(driver: &NetDriver, shared: &Shared) {
    if let Some(ui) = shared.ui.lock().as_ref() {
        let player = &driver.session.player;
        let (ticker, delay) = player.hand.attack_strength_charge();
        ui.set_attack_strength(haste_jvm::AttackStrengthHud { ticker, delay });
        ui.set_crosshair_pick(player.crosshair_pick_living(&*shared.entities.read()));
    }
}

/// This tick's item cooldowns (vanilla `Player.cooldowns`), published for the HUD's item
/// overlay: the Java `GuiGraphicsExtractor.itemCooldown` asks for a stack's percent per frame
/// through the `itemCooldown` native, which reads this snapshot.
pub(super) fn publish_item_cooldowns(driver: &NetDriver) {
    let (tick_count, cooldowns) = driver.session.player.int.cooldowns.snapshot();
    haste_jvm::set_item_cooldowns(haste_jvm::ItemCooldownsHud {
        tick_count,
        cooldowns: cooldowns
            .into_iter()
            .map(|(group, c)| {
                (
                    group,
                    haste_jvm::ItemCooldownHud {
                        start_tick: c.start_tick,
                        end_tick: c.end_tick,
                    },
                )
            })
            .collect(),
    });
}
