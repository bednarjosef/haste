//! The bridge's pre-resolved method table and the low-level calls through it: every other module
//! goes through [`GuiPipeline::call`] and its typed wrappers.

use std::sync::atomic::AtomicU32;

use jni::objects::JObject;
use jni::refs::Global;
use jni::signature::{JavaType, Primitive};
use jni::sys::jvalue;
use jni::{jni_sig, jni_str, Env, JavaVM};
use parking_lot::Mutex;

use super::{GuiPipeline, Methods, BOOL, NO_PANORAMA};
use crate::error::{JvmError, Result};
use crate::host::caught;

impl GuiPipeline {
    pub(crate) fn new(env: &mut Env, bridge: Global<JObject<'static>>) -> Result<GuiPipeline> {
        let cls = env.get_object_class(&*bridge)?;
        let m = Methods {
            open: env.get_method_id(&cls, jni_str!("open"), jni_sig!("(I)V"))?,
            close: env.get_method_id(&cls, jni_str!("close"), jni_sig!("()V"))?,
            reset_for_test: env.get_method_id(&cls, jni_str!("resetForTest"), jni_sig!("()V"))?,
            is_open: env.get_method_id(&cls, jni_str!("isOpen"), jni_sig!("()Z"))?,
            screen_name: env.get_method_id(
                &cls,
                jni_str!("screenName"),
                jni_sig!("()Ljava/lang/String;"),
            )?,
            render: env.get_method_id(
                &cls,
                jni_str!("render"),
                jni_sig!("(IIIDDF)[Ljava/lang/Object;"),
            )?,
            tick: env.get_method_id(&cls, jni_str!("tick"), jni_sig!("()V"))?,
            key: env.get_method_id(&cls, jni_str!("key"), jni_sig!("(IIIZ)Z"))?,
            char_typed: env.get_method_id(&cls, jni_str!("charTyped"), jni_sig!("(II)Z"))?,
            mouse: env.get_method_id(&cls, jni_str!("mouse"), jni_sig!("(DDIZ)Z"))?,
            scroll: env.get_method_id(&cls, jni_str!("scroll"), jni_sig!("(DDDD)Z"))?,
            metrics_changed: env.get_method_id(
                &cls,
                jni_str!("metricsChanged"),
                jni_sig!("()V"),
            )?,
            blend_debug: env.get_method_id(
                &cls,
                jni_str!("blendDebug"),
                jni_sig!("()Ljava/lang/String;"),
            )?,
            mouse_grabbed: env.get_method_id(&cls, jni_str!("mouseGrabbed"), jni_sig!("()Z"))?,
            set_mouse_grabbed: env.get_method_id(
                &cls,
                jni_str!("setMouseGrabbed"),
                jni_sig!("(Z)V"),
            )?,
            game_key: env.get_method_id(&cls, jni_str!("gameKey"), jni_sig!("(IZI)V"))?,
            game_mouse: env.get_method_id(&cls, jni_str!("gameMouse"), jni_sig!("(IZ)V"))?,
            game_input: env.get_method_id(&cls, jni_str!("gameInput"), jni_sig!("()I"))?,
            consume_clicks: env.get_method_id(&cls, jni_str!("consumeClicks"), jni_sig!("()I"))?,
            block_entity_count: env.get_method_id(
                &cls,
                jni_str!("blockEntityCount"),
                jni_sig!("()I"),
            )?,
        };
        Ok(GuiPipeline {
            bridge,
            m,
            lock: Mutex::new(()),
            last_error: Mutex::new(None),
            panorama: AtomicU32::new(NO_PANORAMA),
        })
    }

    /// One call on the bridge with the given return type; Java exceptions become errors (cleared).
    pub(super) fn call<T>(
        &self,
        method: jni::objects::JMethodID,
        ret: JavaType,
        args: &[jvalue],
        f: impl FnOnce(&mut Env, jni::objects::JValueOwned) -> Result<T>,
    ) -> Result<T> {
        let _g = self.lock.lock();
        let vm = JavaVM::singleton()?;
        vm.attach_current_thread(|env| -> Result<T> {
            // Safety: method ids were resolved on the bridge's class with matching signatures/args.
            let v = unsafe { env.call_method_unchecked(&*self.bridge, method, ret, args) }
                .map_err(|e| caught(env, e))?;
            f(env, v)
        })
    }

    pub(super) fn call_bool(
        &self,
        what: &str,
        method: jni::objects::JMethodID,
        args: &[jvalue],
    ) -> bool {
        match self.call(method, BOOL, args, |_, v| Ok(v.z()?)) {
            Ok(b) => b,
            Err(e) => {
                self.failed(what, e);
                false
            }
        }
    }

    pub(super) fn failed(&self, what: &str, e: JvmError) {
        tracing::warn!("GuiPipeline::{what}: {e}");
    }

    pub(super) fn call_int(
        &self,
        what: &str,
        method: jni::objects::JMethodID,
        args: &[jvalue],
    ) -> i32 {
        match self.call(method, JavaType::Primitive(Primitive::Int), args, |_, v| {
            Ok(v.i()?)
        }) {
            Ok(i) => i,
            Err(e) => {
                self.failed(what, e);
                0
            }
        }
    }

    /// The last error swallowed by a method that cannot return one (input, close, tick), if any.
    pub fn take_last_error(&self) -> Option<String> {
        self.last_error.lock().take()
    }
}
