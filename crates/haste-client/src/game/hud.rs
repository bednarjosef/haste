//! The tick's HUD steps: the JVM (or menu UI) arriving on its own thread, and the HUD and
//! chat's tick (screens, inventory and option changes, the pack prompt) with this tick's UI
//! output packets.

use std::time::Instant;

use haste_protocol::ServerboundPlay;

use super::run::{apply_option_changes, HudArrival, HudSlot};
use super::shared::{PackReload, Shared};
use crate::net::{NetDriver, WorldSink};
use crate::ui::Ui;

/// Once the JVM arrives (the session joined before the game thread started): build the
/// `Ui` and give the JVM the registries, tags and the local player. Menu mode's UI arrives
/// pre-built (its JVM opened the title screen); only `joined` runs for it.
pub(super) fn take_arrival(driver: &mut NetDriver, shared: &Shared, jvm: &mut HudSlot) {
    if let Some(ui) = jvm.take_ui() {
        let mut created = false;
        {
            let mut slot = shared.ui.lock();
            if slot.is_none() {
                let ui = match ui {
                    HudArrival::Fresh(host) => Ui::new(host),
                    HudArrival::Menu(ui) => ui,
                };
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
                        *slot = Some(ui);
                        created = true;
                    }
                    Err(e) => tracing::warn!("no HUD: joining the UI failed: {e:#}"),
                }
            }
        }
        // Play packets arrived while the JVM booted; give them to the HUD now, in order, so a
        // fresh join's tab list, boss bars and scoreboard are not lost.
        if created {
            let backlog = driver.take_ui_backlog();
            if !backlog.is_empty() {
                tracing::info!(
                    "HUD catching up on {} packets from the JVM boot",
                    backlog.len()
                );
                if let Some(ui) = shared.ui.lock().as_ref() {
                    let world = shared.world.read();
                    for p in backlog {
                        ui.handle(&p, &|pos| WorldSink::block_state_at(&*world, pos));
                    }
                }
            }
        }
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
        // The hotbar authority is the player (the client picks with 1-9 and the wheel and tells
        // the server itself; `set_selected_slot` also marks it sent so the UI does not send it
        // a second time). The server's `set_held_slot` reaches both sides alike.
        ui.set_selected_slot(driver.session.player.held_slot);
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
