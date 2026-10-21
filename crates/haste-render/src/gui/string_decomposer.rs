//! A chat component flattened to styled characters: vanilla `FormattedText.visit` (each part
//! with its style applied over its parent's) followed by `StringDecomposer.iterateFormatted`
//! (legacy `§` codes inside a part's text, `§r` back to the part's style). Styles are the
//! font's [`TextFormat`]; translation keys print their fallback or the key with their
//! arguments substituted (the renderer holds no language table).

use haste_assets::{for_each_formatted, TextFormat, LEGACY_COLORS};
use haste_core::text::Content;
use haste_core::Component;

/// The `ChatFormatting` colour names in legacy-code order (`§0`..`§f`).
const COLOR_NAMES: [&str; 16] = [
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
];

/// `TextColor.parseColor`: a `ChatFormatting` colour name or `#RRGGBB`; anything else
/// leaves the parent's colour.
pub fn parse_color(name: &str) -> Option<u32> {
    if let Some(hex) = name.strip_prefix('#') {
        return u32::from_str_radix(hex, 16)
            .ok()
            .filter(|c| *c <= 0xFF_FFFF);
    }
    COLOR_NAMES
        .iter()
        .position(|n| *n == name)
        .map(|i| LEGACY_COLORS[i])
}

/// `Style.applyTo(parent)`: the component's own fields where set, the parent's otherwise.
fn style_of(c: &Component, parent: TextFormat) -> TextFormat {
    TextFormat {
        color: c.color.as_deref().and_then(parse_color).or(parent.color),
        bold: c.bold.unwrap_or(parent.bold),
        italic: c.italic.unwrap_or(parent.italic),
        underline: c.underlined.unwrap_or(parent.underline),
        strikethrough: c.strikethrough.unwrap_or(parent.strikethrough),
        obfuscated: c.obfuscated.unwrap_or(parent.obfuscated),
    }
}

/// Appends `component`'s characters with their styles to `out`, depth first like
/// `FormattedText.visit`.
pub fn flatten(component: &Component, parent: TextFormat, out: &mut Vec<(char, TextFormat)>) {
    let style = style_of(component, parent);
    let mut push = |text: &str| for_each_formatted(text, style, |ch, fmt| out.push((ch, fmt)));
    match &component.content {
        Content::Text { text } => push(text),
        Content::Keybind { keybind } => push(keybind),
        Content::Selector { selector } => push(selector),
        Content::Translate {
            translate,
            with,
            fallback,
        } => {
            let format = fallback.as_deref().unwrap_or(translate);
            let args: Vec<String> = with.iter().map(|c| c.to_plain(&|_| None)).collect();
            push(&haste_core::text::format_translation(format, &args));
        }
        Content::Score { .. } | Content::Nbt { .. } => {}
    }
    for child in &component.extra {
        flatten(child, style, out);
    }
}
