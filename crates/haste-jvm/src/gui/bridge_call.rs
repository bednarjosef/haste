//! Named bridge calls outside the pre-resolved method table: one-off `call_method`s used by the
//! configuration and session methods.

use jni::objects::JObject;
use jni::{jni_sig, Env, JavaVM};

use super::GuiPipeline;
use crate::error::Result;
use crate::host::caught;

impl GuiPipeline {
    /// Runs `f` with the JNI environment and the bridge object, serialised like every other call;
    /// a Java exception is returned as an error (cleared).
    pub(super) fn with_bridge<T>(
        &self,
        f: impl FnOnce(&mut Env, &JObject) -> std::result::Result<T, jni::errors::Error>,
    ) -> Result<T> {
        let _g = self.lock.lock();
        let vm = JavaVM::singleton()?;
        vm.attach_current_thread(|env| -> Result<T> {
            let bridge: &JObject = &self.bridge;
            f(env, bridge).map_err(|e| caught(env, e))
        })
    }

    /// Calls a `void` bridge method taking one `byte[]` (and optionally a boolean).
    pub(super) fn call_bytes(
        &self,
        name: &'static str,
        bytes: &[u8],
        flag: Option<bool>,
    ) -> Result<()> {
        self.with_bridge(|env, bridge| {
            let arr = env.byte_array_from_slice(bytes)?;
            let name = jni::strings::JNIString::new(name);
            match flag {
                None => env.call_method(bridge, &name, jni_sig!("([B)V"), &[(&arr).into()])?,
                Some(z) => env.call_method(
                    bridge,
                    &name,
                    jni_sig!("([BZ)V"),
                    &[(&arr).into(), jni::objects::JValue::Bool(z)],
                )?,
            };
            env.delete_local_ref(arr);
            Ok(())
        })
    }
}
