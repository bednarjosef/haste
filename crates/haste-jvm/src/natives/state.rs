//! Player state / inventory mirror / outgoing natives on [`crate::GameState`].
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JByteArray, JClass, JDoubleArray, JIntArray, JString};
use jni::sys::{jboolean, jint};
use jni::EnvUnowned;

use crate::proxies::{decode_stack, with_state, ChatMessage};

/// Finished server-list pings, oldest first: the address, its status JSON (empty when the ping
/// failed) and the round-trip milliseconds. Filled by [`start_ping`]'s threads, drained by
/// `takePingResults`.
static PING_RESULTS: parking_lot::Mutex<Vec<PingResult>> = parking_lot::Mutex::new(Vec::new());

struct PingResult {
    address: String,
    /// The status JSON, or empty when the server could not be pinged.
    json: String,
    millis: i64,
}

pub(crate) extern "system" fn selected_slot<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) -> jint {
    with_state(|s| s.selected_slot)
}

pub(crate) extern "system" fn permission_level<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) -> jint {
    with_state(|s| jint::from(s.permission_level))
}

pub(crate) extern "system" fn game_mode<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) -> jint {
    with_state(|s| s.game_mode as jint)
}

pub(crate) extern "system" fn player_name<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
) -> JString<'l> {
    let name = with_state(|s| s.player_name.clone());
    env.with_env(|env| -> jni::errors::Result<JString<'l>> { JString::from_str(env, name) })
        .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn inventory_slot<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    slot: jint,
    stack: JByteArray<'l>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let b = env.convert_byte_array(&stack)?;
        match decode_stack(&b) {
            Some(st) => with_state(|s| s.set_mirror(slot, st)),
            None => tracing::warn!("inventory slot {slot}: malformed stack from Java"),
        }
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn send_packet<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    packet: JByteArray<'l>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let b = env.convert_byte_array(&packet)?;
        with_state(|s| s.outgoing.push(b));
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn send_chat<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    msg: JString<'l>,
    command: jboolean,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let text = msg.try_to_string(env)?;
        with_state(|s| s.sent.push(ChatMessage { text, command }));
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Copies the primitive player/debug state into Java's mirror arrays (one call per frame and tick).
pub(crate) extern "system" fn mirror<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    ints: JIntArray<'l>,
    doubles: JDoubleArray<'l>,
) {
    let (i, d) = with_state(|s| s.mirror());
    env.with_env(|env| -> jni::errors::Result<()> {
        ints.set_region(env, 0, &i)?;
        doubles.set_region(env, 0, &d)?;
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn request_disconnect<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) {
    with_state(|s| s.disconnect_requested = true);
}

pub(crate) extern "system" fn request_join<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    address: JString<'l>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let address = address.try_to_string(env)?;
        with_state(|s| {
            s.join_requested = Some(address);
            s.join_cancelled = false;
        });
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn cancel_join<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) {
    with_state(|s| s.join_cancelled = true);
}

pub(crate) extern "system" fn request_quit<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) {
    with_state(|s| s.quit_requested = true);
}

/// The world list's "Play Selected World": the world folder to start the local server for.
pub(crate) extern "system" fn request_open_world<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    level_id: JString<'l>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let level_id = level_id.try_to_string(env)?;
        tracing::info!("singleplayer world requested by the world list: {level_id}");
        with_state(|s| {
            s.open_world_requested = Some(level_id);
            s.join_cancelled = false;
        });
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// The create screen's "Create New World": the world to make with these settings.
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn request_create_world<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    folder: JString<'l>,
    name: JString<'l>,
    game_mode: jint,
    difficulty: jint,
    hardcore: jboolean,
    allow_commands: jboolean,
    seed: JString<'l>,
    generate_structures: jboolean,
    level_type: JString<'l>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let request = crate::proxies::CreateWorldRequest {
            folder: folder.try_to_string(env)?,
            name: name.try_to_string(env)?,
            game_mode,
            difficulty,
            hardcore,
            allow_commands,
            seed: seed.try_to_string(env)?,
            generate_structures,
            level_type: level_type.try_to_string(env)?,
        };
        tracing::info!("world creation requested by the create screen: {request:?}");
        with_state(|s| {
            s.create_world_requested = Some(request);
            s.join_cancelled = false;
        });
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Whether the local singleplayer server is running (the pause screen's button label).
pub(crate) extern "system" fn local_world_running<'l>(
    _env: EnvUnowned<'l>,
    _c: JClass<'l>,
) -> jboolean {
    with_state(|s| s.local_world_running)
}

/// Server-list ping through haste-protocol on its own thread (vanilla pings from a thread pool;
/// the result reaches Java on the next `takePingResults`). One thread per address, like the
/// vanilla pinger's one connection per address; a screen holds at most a handful of entries.
pub(crate) extern "system" fn start_ping<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    address: JString<'l>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let address = address.try_to_string(env)?;
        let spawned = std::thread::Builder::new()
            .name(format!("haste-ping-{address}"))
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let result = match rt {
                    Ok(rt) => match rt.block_on(haste_protocol::ping(&address, None)) {
                        Ok(status) => PingResult {
                            address,
                            json: status.json.to_string(),
                            millis: status.latency.as_millis() as i64,
                        },
                        Err(e) => {
                            tracing::debug!("server list ping {address} failed: {e}");
                            PingResult {
                                address,
                                json: String::new(),
                                millis: 0,
                            }
                        }
                    },
                    Err(e) => {
                        tracing::warn!("ping runtime: {e}");
                        PingResult {
                            address,
                            json: String::new(),
                            millis: 0,
                        }
                    }
                };
                PING_RESULTS.lock().push(result);
            });
        if let Err(e) = spawned {
            // The screen's own refresh on the next tick shows the failure; the ping is lost.
            tracing::warn!("spawning the ping thread: {e}");
        }
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

/// Drains the finished pings into Java as one "address\njson\nmillis" string each.
pub(crate) extern "system" fn take_ping_results<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
) -> jni::objects::JObjectArray<'l> {
    let drained: Vec<String> = PING_RESULTS
        .lock()
        .drain(..)
        .map(|r| format!("{}\n{}\n{}", r.address, r.json, r.millis))
        .collect();
    env.with_env(
        |env| -> jni::errors::Result<jni::objects::JObjectArray<'l>> {
            let string_class = env.find_class(jni::strings::JNIString::from("java/lang/String"))?;
            let empty = env.new_string("")?;
            let array = env.new_object_array(drained.len() as i32, string_class, empty)?;
            for (i, line) in drained.iter().enumerate() {
                let s = env.new_string(line)?;
                array.set_element(env, i, &s)?;
            }
            Ok(array)
        },
    )
    .resolve::<ThrowRuntimeExAndDefault>()
}
