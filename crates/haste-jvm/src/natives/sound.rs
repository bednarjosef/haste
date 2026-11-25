//! The `SoundManager` proxy's natives (`haste.gui.SoundImpl`): whether a sound event
//! listener is registered, and the frame's drain of the listener transform and the played
//! sounds (see [`crate::sound_manager`]).

use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JDoubleArray, JObject, JObjectArray};
use jni::sys::jboolean;
use jni::EnvUnowned;

use crate::sound_manager::with_queue;

/// `soundListening(boolean)`: Java registered its first sound event listener or removed
/// its last one (vanilla `SoundManager.addListener` / `removeListener`).
pub(crate) extern "system" fn sound_listening<'l>(
    _env: EnvUnowned<'l>,
    _c: JClass<'l>,
    listening: jboolean,
) {
    with_queue(|q| q.set_listening(listening));
}

/// `takePlayedSounds(double[] listener)`: fills the listener transform and returns the
/// sounds played since the last call as `{String[] names, double[] values}` (the layout of
/// `SoundEventQueue::take`), or null when there are none: one call per frame while a
/// listener is registered, allocating Java arrays only when a sound was played.
pub(crate) extern "system" fn take_played_sounds<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    listener: JDoubleArray<'l>,
) -> JObjectArray<'l> {
    let mut names = Vec::new();
    let mut values = Vec::new();
    let transform = with_queue(|q| q.take(&mut names, &mut values));
    env.with_env(|env| -> jni::errors::Result<JObjectArray<'l>> {
        listener.set_region(env, 0, &transform)?;
        if names.is_empty() {
            return Ok(JObjectArray::<JObject>::null());
        }
        let string_class = env.find_class(jni::strings::JNIString::from("java/lang/String"))?;
        let name_array = env.new_object_array(names.len() as i32, string_class, JObject::null())?;
        for (i, name) in names.iter().enumerate() {
            let s = env.new_string(&**name)?;
            name_array.set_element(env, i, &s)?;
            env.delete_local_ref(s);
        }
        let value_array = env.new_double_array(values.len())?;
        value_array.set_region(env, 0, &values)?;
        let object_class = env.find_class(jni::strings::JNIString::from("java/lang/Object"))?;
        let both = env.new_object_array(2, object_class, JObject::null())?;
        both.set_element(env, 0, &name_array)?;
        both.set_element(env, 1, &value_array)?;
        Ok(both)
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}
