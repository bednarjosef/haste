//! The tick's block entity steps (vanilla `Level.tickBlockEntities` on the client): the lids'
//! open targets, the skulls' and object-model block entities' animation counters, and the
//! vaults' and spawners' spins, each published for the renderer to interpolate.

use glam::DVec3;
use haste_core::view::BlockView;
use haste_protocol::haste_core::BlockPos;
use haste_world::World;

use super::block_entity_ticks::{tick_model_anims, TickModelAnims, MODEL_RNG};
use super::shared::Shared;
use crate::net::NetDriver;

/// The lids' open targets (vanilla routes `block_event` to the block entity).
pub(super) fn tick_lids(driver: &mut NetDriver, shared: &Shared) {
    let mut lids = shared.block_entity_lids.lock();
    lids.clear();
    driver.session.lid_states(&mut lids);
}

/// The skulls' animation counters (vanilla ticks each `SkullBlockEntity.animation`:
/// while the block state is powered the counter grows, powering the jaw/ear swing).
pub(super) fn tick_skulls_and_model_anims(driver: &mut NetDriver, shared: &Shared) {
    let world = shared.world.read();
    let blocks = &shared.data.blocks;
    let names = &shared.block_entity_names;
    let mut anims = shared.skull_anims.lock();
    anims.retain_mut(|(pos, anim)| {
        // Gone (chunk unloaded, block replaced): drop. Otherwise a skull keeps its
        // entry; powered ones advance, unpowered ones freeze at the last value (vanilla's
        // counter stops but keeps its value).
        let Some(be) = world.block_entity(*pos) else {
            return false;
        };
        let is_skull = names
            .get(be.kind as usize)
            .is_some_and(|n| n.namespace() == "minecraft" && n.path() == "skull");
        if !is_skull {
            return false;
        }
        let powered = blocks
            .property_value(world.block_state(*pos), "powered")
            .is_some_and(|v| v == "true");
        anim.0 = anim.1;
        if powered {
            anim.1 += 1.0;
        }
        true
    });
    // The object-model block entities' animation state: the bell shakes, the decorated pot
    // wobbles and the conduit and enchanting-table ticks (vanilla's block entity tickers for
    // `BellBlockEntity`, `DecoratedPotBlockEntity`, `ConduitBlockEntity` and
    // `EnchantingTableBlockEntity`). The events come from the session's `block_event` handling;
    // everything else is driven from the world each tick, like the skulls above.
    {
        let tags = driver.session.player.int.block_tags();
        let mut bell_events = Vec::new();
        let mut pot_events = Vec::new();
        driver
            .session
            .block_event_anims(&mut bell_events, &mut pot_events);
        let view = shared.view.lock();
        let world_age = view.world_age;
        let feet = view.feet;
        drop(view);
        let mut anims = shared.block_entity_anims.lock();
        let mut rng = MODEL_RNG.lock();
        // The guard derefs to `&World` through the binding's type annotation.
        let world: &World = &world;
        tick_model_anims(TickModelAnims {
            anims: &mut anims,
            world,
            blocks,
            names,
            tags,
            world_age,
            feet: DVec3::new(feet.x, feet.y, feet.z),
            rng: &mut rng,
            bell_events: &bell_events,
            pot_events: &pot_events,
        });
    }
    // Newly powered skulls (loaded with powered=true, or toggled since) start at 0.
    'outer: for chunk in world.loaded_chunks() {
        for (pos, be) in world.block_entities_in(chunk) {
            let Some(name) = names.get(be.kind as usize) else {
                continue;
            };
            if name.namespace() != "minecraft" || name.path() != "skull" {
                continue;
            }
            let powered = blocks
                .property_value(world.block_state(pos), "powered")
                .is_some_and(|v| v == "true");
            if powered && !anims.iter().any(|(p, _)| p == &pos) {
                anims.push((pos, (0.0, 0.0)));
                if anims.len() > 1024 {
                    break 'outer;
                }
            }
        }
    }
}

/// The vaults' and spawners' spins (vanilla `VaultClientData.updateDisplayItemSpin` and
/// the spawners' client tick): every vault's displayed item turns 10 degrees a tick, a
/// spawner's display mob turns at the rate its state's spinning mob speed and its remaining
/// spawn delay give (a plain mob spawner only while a player is near it).
pub(super) fn tick_spins(driver: &mut NetDriver, shared: &Shared) {
    let world = shared.world.read();
    let blocks = &shared.data.blocks;
    let names = &shared.block_entity_names;
    let player = driver.session.player.state.pos;
    let game_time = driver.session.world_age;
    let mut spins = shared.block_entity_spins.lock();
    spins.retain_mut(|(pos, spin)| {
        let Some(be) = world.block_entity(*pos) else {
            return false;
        };
        let Some(name) = names.get(be.kind as usize) else {
            return false;
        };
        let minecraft = name.namespace() == "minecraft";
        let rate = if minecraft && name.path() == "vault" {
            // `VaultClientData.ROTATION_SPEED` (10 degrees a tick, wrapped).
            Some(10.0)
        } else if minecraft && matches!(name.path(), "mob_spawner" | "trial_spawner") {
            let trial = name.path() == "trial_spawner";
            if trial || mob_spawner_spins(&be.nbt, *pos, player) {
                Some(spawner_spin_rate(
                    &be.nbt,
                    trial,
                    game_time,
                    blocks
                        .property_value(world.block_state(*pos), "trial_spawner_state")
                        .unwrap_or(""),
                ))
            } else {
                // No player in range: vanilla holds the spin (`oSpin = spin`).
                Some(0.0)
            }
        } else {
            None
        };
        let Some(rate) = rate else {
            return false;
        };
        spin.0 = spin.1;
        if rate > 0.0 {
            // Vanilla's `(spin + rate) % 360`; a trial spawner in a state with no spinning
            // mob holds its spin.
            spin.1 = (spin.1 + rate).rem_euclid(360.0);
        }
        true
    });
    // New vaults and spawners start their spin at 0.
    'outer: for chunk in world.loaded_chunks() {
        for (pos, be) in world.block_entities_in(chunk) {
            let Some(name) = names.get(be.kind as usize) else {
                continue;
            };
            if name.namespace() != "minecraft"
                || !matches!(name.path(), "vault" | "mob_spawner" | "trial_spawner")
                || spins.iter().any(|(p, _)| p == &pos)
            {
                continue;
            }
            spins.push((pos, (0.0, 0.0)));
            if spins.len() > 1024 {
                break 'outer;
            }
        }
    }
}

/// Vanilla `BaseSpawner.isNearPlayer`: an alive player within the spawner's required player
/// range of its centre. The client has only its own player, so a spawner spins its mob exactly
/// when the camera is close, like vanilla's single-player client. Only `BaseSpawner.clientTick`
/// checks this; a trial spawner's `tickClient` spins its mob in every state that has one.
fn mob_spawner_spins(nbt: &[u8], pos: BlockPos, player: DVec3) -> bool {
    let centre = DVec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    // `BaseSpawner.requiredPlayerRange` is the block entity's `RequiredPlayerRange` tag, not a
    // block state property; vanilla writes it as a short and reads it back leniently.
    let range = read_block_entity_nbt(nbt)
        .and_then(|doc| doc.root_tag.get_numeric_short("RequiredPlayerRange"))
        .unwrap_or(16) as f64;
    player.distance_squared(centre) <= range * range
}

/// The degrees a spawner's display mob turns in one tick, vanilla `BaseSpawner.clientTick`'s
/// `1000 / (spawnDelay + 200)` and `TrialSpawner.tickClient`'s
/// `spinningMobSpeed / (spawnDelay + 200)`. A trial spawner's spinning mob speed comes from
/// its block state (`TrialSpawnerState`: 200 waiting for players, 1000 active, -1 for the
/// states with no spinning mob) and its remaining spawn delay from `next_mob_spawns_at`.
fn spawner_spin_rate(nbt: &[u8], trial: bool, game_time: i64, state: &str) -> f32 {
    // The trial spawner's state decides whether a mob spins at all (vanilla
    // `TrialSpawnerState.hasSpinningMob`).
    let speed = if trial {
        trial_spinning_mob_speed(state)
    } else {
        1000.0
    };
    if speed <= 0.0 {
        return 0.0;
    }
    let delay = if trial {
        trial_spawn_delay(nbt, game_time)
    } else {
        spawner_delay(nbt)
    };
    (speed / (delay.max(0) as f64 + 200.0)) as f32
}

/// Vanilla `BaseSpawner.spawnDelay`, from the block entity's `Delay` tag (a short, default 20).
fn spawner_delay(nbt: &[u8]) -> i32 {
    read_block_entity_nbt(nbt)
        .and_then(|doc| doc.root_tag.get_short("Delay"))
        .unwrap_or(20) as i32
}

/// Vanilla `TrialSpawnerStateData.nextMobSpawnsAt` as the remaining delay
/// (`max(0, nextMobSpawnsAt - level.getGameTime())`).
fn trial_spawn_delay(nbt: &[u8], game_time: i64) -> i32 {
    read_block_entity_nbt(nbt)
        .and_then(|doc| doc.root_tag.get_long("next_mob_spawns_at"))
        .map(|at| (at - game_time).max(0) as i32)
        .unwrap_or(0)
}

/// Vanilla `TrialSpawnerState.spinningMobSpeed` for the state the block is in: waiting for
/// players 200, active 1000, every other state has no spinning mob (-1).
fn trial_spinning_mob_speed(state: &str) -> f64 {
    match state {
        "waiting_for_players" => 200.0,
        "active" => 1000.0,
        _ => -1.0,
    }
}

/// A block entity's NBT payload (the network variant `read_unnamed` reads), or `None`.
fn read_block_entity_nbt(nbt: &[u8]) -> Option<haste_protocol::nbt::Nbt> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()
}
