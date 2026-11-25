//! Read-only queries the client and tests make of the Java side: chat, commands, options,
//! completions and registry contents.

use jni::objects::JString;
use jni::{jni_sig, jni_str};

use super::GuiPipeline;
use crate::error::Result;

impl GuiPipeline {
    /// The chat log as plain text, oldest first (vanilla's `ChatComponent`: what the chat shows,
    /// after decoration such as "<name> message").
    pub fn chat_history(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .call_string("chatHistory")?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// The server's commands the local player may use (the root of the client-side command tree
    /// built from the `commands` packet).
    pub fn command_names(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .call_string("commandNames")?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    #[doc(hidden)]
    pub fn debug_suggestions(&self) -> anyhow::Result<String> {
        Ok(self.call_string("debugSuggestions")?)
    }

    /// The value of a vanilla option by its accessor name (`"autoSuggestions"`, `"guiScale"`, ...)
    /// as Java prints it.
    pub fn option(&self, name: &str) -> anyhow::Result<String> {
        let s = self.with_bridge(|env, bridge| {
            let l = JString::from_str(env, name)?;
            let v = env
                .call_method(
                    bridge,
                    jni_str!("option"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[(&l).into()],
                )?
                .l()?;
            // Safety: option returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            v.try_to_string(env)
        })?;
        Ok(s)
    }

    /// Brigadier's completions of a command line (without "/") from the server's command tree, as
    /// the chat screen computes them (server-side suggestions are not waited for).
    pub fn completions(&self, line: &str) -> anyhow::Result<Vec<String>> {
        let s = self.with_bridge(|env, bridge| {
            let l = JString::from_str(env, line)?;
            let v = env
                .call_method(
                    bridge,
                    jni_str!("completions"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[(&l).into()],
                )?
                .l()?;
            // Safety: completions returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            v.try_to_string(env)
        })?;
        Ok(s.lines().map(str::to_owned).collect())
    }

    /// Every registry Java sees with its size (built-in from vanilla's bootstrap, plus the
    /// synchronised ones after [`Self::finish_configuration`]).
    pub fn registry_sizes(&self) -> anyhow::Result<std::collections::BTreeMap<String, usize>> {
        let s = self.call_string("registrySizes")?;
        Ok(s.lines()
            .filter_map(|l| l.split_once('='))
            .filter_map(|(k, v)| Some((k.to_owned(), v.parse().ok()?)))
            .collect())
    }

    /// The name of entry `id` of `registry` as Java sees it (`None` if absent).
    pub fn registry_name(&self, registry: &str, id: i32) -> anyhow::Result<Option<String>> {
        let s = self.with_bridge(|env, bridge| {
            let r = JString::from_str(env, registry)?;
            let v = env
                .call_method(
                    bridge,
                    jni_str!("registryName"),
                    jni_sig!("(Ljava/lang/String;I)Ljava/lang/String;"),
                    &[(&r).into(), jni::objects::JValue::Int(id)],
                )?
                .l()?;
            if v.is_null() {
                return Ok(None);
            }
            // Safety: registryName returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            Ok(Some(v.try_to_string(env)?))
        })?;
        Ok(s)
    }

    fn call_string(&self, name: &'static str) -> Result<String> {
        self.with_bridge(|env, bridge| {
            let name = jni::strings::JNIString::new(name);
            let v = env
                .call_method(bridge, &name, jni_sig!("()Ljava/lang/String;"), &[])?
                .l()?;
            if v.is_null() {
                return Ok(String::new());
            }
            // Safety: the method returns a String.
            let v = unsafe { JString::from_raw(env, v.into_raw()) };
            v.try_to_string(env)
        })
    }
}
