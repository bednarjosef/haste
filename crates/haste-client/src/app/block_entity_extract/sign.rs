use std::sync::Arc;

/// One sign side from its NBT compound (vanilla `SignText.CODEC`): `messages` (4 components as
/// plain strings or `{"text": ...}` compounds), `color` (a `DyeColor` name) and
/// `has_glowing_text`. Each line is wrapped to `max_width` like vanilla's render messages
/// (`Font.split`, first wrapped line only). Missing fields keep vanilla's defaults (black,
/// not glowing, empty).
pub(super) fn parse_sign_side(
    c: &haste_protocol::nbt::NbtCompound,
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> haste_render::blockentity::SignSideText {
    let mut lines: [String; 4] = Default::default();
    if let Some(messages) = c.get_list("messages") {
        for (i, m) in messages.iter().take(4).enumerate() {
            let text = match m {
                haste_protocol::nbt::NbtTag::String(s) => s.to_string(),
                haste_protocol::nbt::NbtTag::Compound(cc) => {
                    cc.get_string("text").unwrap_or_default().to_string()
                }
                _ => String::new(),
            };
            lines[i] = first_wrapped_line(&text, metrics, max_width).to_string();
        }
    }
    const DYES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let color = c
        .get_string("color")
        .and_then(|n| DYES.iter().position(|d| *d == n))
        .map_or(15, |i| i as u8);
    let glowing = c.get_bool("has_glowing_text").unwrap_or(false);
    haste_render::blockentity::SignSideText {
        lines,
        color,
        glowing,
    }
}

/// The first wrapped line of `text` at `max_width`, vanilla `StringSplitter.splitLines` for a
/// plain string: characters accumulate their widths; past the limit the line breaks at the
/// last space (dropping it), or right before the character that overflowed when there is
/// none; a first character wider than the limit still yields one character (the
/// `hadNonZeroWidthChar` guard). Newlines break there.
pub(super) fn first_wrapped_line<'a>(
    text: &'a str,
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> &'a str {
    let max_width = max_width.max(1);
    let mut width = 0i32;
    let mut had_non_zero = false;
    let mut last_space: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        match ch {
            '\n' => return &text[..i],
            ' ' => last_space = Some(i),
            _ => {}
        }
        let char_width = metrics.width(ch.encode_utf8(&mut [0; 4]));
        width += char_width;
        if !had_non_zero || width <= max_width {
            had_non_zero |= char_width != 0;
        } else {
            return match last_space {
                // Break at the last space, dropping it.
                Some(s) => &text[..s],
                // No space: break before this character (at least one char: the guard above
                // always accepts the first non-zero-width one).
                None => &text[..i],
            };
        }
    }
    text
}

/// The whole sign's text (`front_text` / `back_text`), or `None` when the NBT does not parse.
pub(super) fn parse_sign_nbt(
    nbt: &[u8],
    metrics: &dyn haste_core::gui::TextMetrics,
    max_width: i32,
) -> Option<Arc<haste_render::blockentity::SignTextData>> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let doc = haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()?;
    let root = &doc.root_tag;
    let side = |name: &str| {
        root.get_compound(name)
            .map(|c| parse_sign_side(c, metrics, max_width))
            .unwrap_or_else(empty_sign_side)
    };
    Some(Arc::new(haste_render::blockentity::SignTextData {
        front: side("front_text"),
        back: side("back_text"),
    }))
}

/// Vanilla `SignText.EMPTY`: four empty lines, black, not glowing.
pub(super) fn empty_sign_side() -> haste_render::blockentity::SignSideText {
    haste_render::blockentity::SignSideText {
        lines: std::array::from_fn(|_| String::new()),
        color: 15,
        glowing: false,
    }
}
