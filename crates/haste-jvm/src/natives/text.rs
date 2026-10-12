//! `Font` natives: text widths from the renderer's [`TextMetrics`].
use std::sync::Arc;

use haste_core::gui::TextMetrics;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JString};
use jni::sys::jint;
use jni::EnvUnowned;
use parking_lot::RwLock;

/// The metrics `Font.width` uses: the renderer's, or [`FallbackMetrics`] until one is set.
pub(crate) type Metrics = Arc<dyn TextMetrics>;

static METRICS: RwLock<Option<Metrics>> = RwLock::new(None);

/// Installs the renderer's text metrics (called by `GuiPipeline::set_text_metrics`).
pub(crate) fn set_metrics(m: Metrics) {
    *METRICS.write() = Some(m);
}

/// Used until the renderer provides real metrics: 6 px per character (the advance of most
/// characters in the default font: 5 px glyph + 1 px spacing), 9 px lines. Approximate.
pub(crate) struct FallbackMetrics;
impl TextMetrics for FallbackMetrics {
    fn width(&self, text: &str) -> i32 {
        text.chars().count() as i32 * 6
    }
}

fn with_metrics<T>(f: impl FnOnce(&dyn TextMetrics) -> T) -> T {
    match METRICS.read().as_ref() {
        Some(m) => f(m.as_ref()),
        None => f(&FallbackMetrics),
    }
}

pub(crate) extern "system" fn text_width<'l>(
    mut env: EnvUnowned<'l>,
    _c: JClass<'l>,
    s: JString<'l>,
) -> jint {
    env.with_env(|env| -> jni::errors::Result<jint> {
        if s.is_null() {
            return Ok(0);
        }
        let s = s.try_to_string(env)?;
        Ok(with_metrics(|m| m.width(&s)))
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

pub(crate) extern "system" fn char_width<'l>(
    _env: EnvUnowned<'l>,
    _c: JClass<'l>,
    cp: jint,
) -> jint {
    let Some(ch) = u32::try_from(cp).ok().and_then(char::from_u32) else {
        return 0;
    };
    let mut buf = [0u8; 4];
    with_metrics(|m| m.width(ch.encode_utf8(&mut buf)))
}

pub(crate) extern "system" fn line_height<'l>(_env: EnvUnowned<'l>, _c: JClass<'l>) -> jint {
    with_metrics(|m| m.line_height())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_widths() {
        assert_eq!(FallbackMetrics.width("abc"), 18);
        assert_eq!(FallbackMetrics.line_height(), 9);
    }
}
