//! Player state / inventory mirror / outgoing natives on [`crate::GameState`].
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JByteArray, JClass, JObjectArray, JString};
use jni::sys::{jboolean, jint};
use jni::EnvUnowned;

use crate::proxies::{decode_stack, with_state, ChatMessage, ContainerClick, ContainerInput};

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

pub(crate) extern "system" fn chat_lines<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
) -> JObjectArray<'l, JString<'l>> {
    let lines = with_state(|s| s.chat_lines.clone());
    env.with_env(
        |env| -> jni::errors::Result<JObjectArray<'l, JString<'l>>> {
            let arr = JObjectArray::<JString>::new(env, lines.len(), JString::null())?;
            for (i, l) in lines.iter().enumerate() {
                let js = JString::from_str(env, l)?;
                arr.set_element(env, i, &js)?;
            }
            Ok(arr)
        },
    )
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

pub(crate) extern "system" fn container_input<'l>(
    _env: EnvUnowned<'l>,
    _c: JClass<'l>,
    container: jint,
    slot: jint,
    button: jint,
    input: jint,
) {
    let input = ContainerInput::from_id(input);
    with_state(|s| {
        s.container_clicks.push(ContainerClick {
            container_id: container,
            slot,
            button,
            input,
        })
    });
}
