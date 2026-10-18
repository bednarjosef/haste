//! The server resource pack prompt through the bridge: vanilla's
//! `ClientCommonPacketListenerImpl.PackConfirmScreen` (a real vanilla
//! `ConfirmScreen`, built in `haste.gui.Bridge.showPackPrompt`).

use jni::objects::{JObject, JString, JValue};
use jni::{jni_sig, jni_str};

use super::GuiPipeline;

impl GuiPipeline {
    /// Opens the pack confirm screen over the open screen (or rebuilds an open
    /// one, keeping the screen it returns to): `required` is the newest push's
    /// required flag (vanilla's required wording, Proceed / Disconnect),
    /// `prompt_json` its prompt text as a text component in JSON form. The
    /// player's answer arrives through [`Self::take_pack_prompt_answer`].
    pub fn show_pack_prompt(&self, required: bool, prompt_json: Option<&str>) {
        // An answer to an earlier screen must not answer this one.
        self.with_state(|s| s.pack_prompt_answer = None);
        if let Err(e) = self.with_bridge(|env, bridge| {
            let prompt = match prompt_json {
                Some(json) => JObject::from(JString::from_str(env, json)?),
                None => JObject::null(),
            };
            env.call_method(
                bridge,
                jni_str!("showPackPrompt"),
                jni_sig!("(ZLjava/lang/String;)V"),
                &[JValue::Bool(required), JValue::Object(&prompt)],
            )?;
            Ok(())
        }) {
            self.failed("show_pack_prompt", e);
        }
    }

    /// The player's answer to the pack confirm screen (true = yes / proceed),
    /// once, if the screen's callback ran since the last call.
    pub fn take_pack_prompt_answer(&self) -> Option<bool> {
        self.with_state(|s| s.pack_prompt_answer.take())
    }
}
