use serde::{Deserialize, Serialize};

/// A chat component. Tier-3 Java owns the full semantics (translation, styles, events); Rust keeps
/// a faithful but minimal representation so the HUD and headless tools can show text and so the
/// protocol crate can round-trip both the JSON and NBT encodings.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Component {
    #[serde(flatten)]
    pub content: Content,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlined: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obfuscated: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<Component>,
    /// Anything we do not model (click/hover events, font, insertion) is kept verbatim.
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text { text: String },
    Translate {
        translate: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        with: Vec<Component>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<String>,
    },
    Keybind { keybind: String },
    Score { score: serde_json::Value },
    Selector { selector: String },
    Nbt { nbt: String },
}
impl Default for Content {
    fn default() -> Self { Content::Text { text: String::new() } }
}

impl Component {
    pub fn text(s: impl Into<String>) -> Self { Self { content: Content::Text { text: s.into() }, ..Default::default() } }
    pub fn translate(key: impl Into<String>, with: Vec<Component>) -> Self {
        Self { content: Content::Translate { translate: key.into(), with, fallback: None }, ..Default::default() }
    }
    /// Plain text with translation keys resolved by `translate` (language file lookup) and legacy
    /// `%s`/`%1$s` substitution. Pass `|k| None` for a raw dump.
    pub fn to_plain(&self, translate: &dyn Fn(&str) -> Option<String>) -> String {
        let mut out = String::new();
        self.append_plain(&mut out, translate);
        out
    }
    fn append_plain(&self, out: &mut String, translate: &dyn Fn(&str) -> Option<String>) {
        match &self.content {
            Content::Text { text } => out.push_str(text),
            Content::Translate { translate: key, with, fallback } => {
                let fmt = translate(key).or_else(|| fallback.clone()).unwrap_or_else(|| key.clone());
                let args: Vec<String> = with.iter().map(|c| c.to_plain(translate)).collect();
                out.push_str(&format_translation(&fmt, &args));
            }
            Content::Keybind { keybind } => out.push_str(keybind),
            Content::Score { .. } | Content::Nbt { .. } => {}
            Content::Selector { selector } => out.push_str(selector),
        }
        for e in &self.extra { e.append_plain(out, translate); }
    }
}

/// Java `String.format`-style `%s` / `%1$s` / `%%` substitution as used by vanilla translations.
pub fn format_translation(fmt: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut next = 0usize;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' { out.push(c); continue; }
        match chars.peek() {
            Some('%') => { chars.next(); out.push('%'); }
            Some('s') => { chars.next(); if let Some(a) = args.get(next) { out.push_str(a); } next += 1; }
            Some(d) if d.is_ascii_digit() => {
                let mut n = 0usize;
                while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) { n = n * 10 + d.to_digit(10).unwrap() as usize; chars.next(); }
                if chars.peek() == Some(&'$') { chars.next(); }
                if chars.peek() == Some(&'s') { chars.next(); }
                if n >= 1 { if let Some(a) = args.get(n - 1) { out.push_str(a); } }
            }
            _ => out.push('%'),
        }
    }
    out
}
