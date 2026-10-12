//! Inventory / item stack / chat natives reading [`crate::GameState`].
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JObjectArray, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::EnvUnowned;

use crate::proxies::{with_state, ChatMessage, ContainerClick, ContainerInput, INVENTORY_SIZE};

pub(crate) extern "system" fn stack_item<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>, h: jlong) -> jint {
    with_state(|s| s.stack(h).map_or(-1, |x| x.item.0 as jint))
}

pub(crate) extern "system" fn stack_count<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>, h: jlong) -> jint {
    with_state(|s| s.stack(h).map_or(0, |x| x.count))
}

pub(crate) extern "system" fn stack_set_count<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>, h: jlong, count: jint) {
    with_state(|s| {
        let Some(slot) = usize::try_from(h).ok().and_then(|i| s.inventory.get_mut(i)) else { return };
        if count <= 0 {
            *slot = None;
        } else if let Some(x) = slot.as_mut() {
            x.count = count;
        }
    })
}

pub(crate) extern "system" fn item_name<'l>(mut env: EnvUnowned<'l>, _c: JClass<'l>, item: jint) -> JString<'l> {
    let name = with_state(|s| usize::try_from(item).ok().and_then(|i| s.item_names.get(i).cloned()));
    env.with_env(|env| -> jni::errors::Result<JString<'l>> {
        match name {
            Some(n) => JString::from_str(env, n),
            None => Ok(JString::default()),
        }
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn inventory_size<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) -> jint {
    INVENTORY_SIZE as jint
}

pub(crate) extern "system" fn selected_slot<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) -> jint {
    with_state(|s| s.selected_slot)
}

pub(crate) extern "system" fn chat_lines<'l>(mut env: EnvUnowned<'l>, _c: JClass<'l>) -> JObjectArray<'l, JString<'l>> {
    let lines = with_state(|s| s.chat_lines.clone());
    env.with_env(|env| -> jni::errors::Result<JObjectArray<'l, JString<'l>>> {
        let arr = JObjectArray::<JString>::new(env, lines.len(), JString::null())?;
        for (i, l) in lines.iter().enumerate() {
            let js = JString::from_str(env, l)?;
            arr.set_element(env, i, &js)?;
        }
        Ok(arr)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn send_chat<'l>(mut env: EnvUnowned<'l>, _c: JClass<'l>, msg: JString<'l>, command: jboolean) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let text = msg.try_to_string(env)?;
        with_state(|s| s.sent.push(ChatMessage { text, command }));
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn player_name<'l>(mut env: EnvUnowned<'l>, _c: JClass<'l>) -> JString<'l> {
    let name = with_state(|s| s.player_name.clone());
    env.with_env(|env| -> jni::errors::Result<JString<'l>> { JString::from_str(env, name) })
        .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn game_mode<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) -> jint {
    with_state(|s| s.game_mode as jint)
}

pub(crate) extern "system" fn container_input<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>, container: jint, slot: jint, button: jint, input: jint) {
    let input = ContainerInput::from_id(input);
    with_state(|s| s.container_clicks.push(ContainerClick { container_id: container, slot, button, input }));
}
