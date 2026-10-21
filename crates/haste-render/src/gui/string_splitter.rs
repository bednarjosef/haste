//! Vanilla `StringSplitter.splitLines` over styled characters: the line breaks `Font.split`
//! produces for a component at a maximum width.

use std::ops::Range;

use haste_assets::{BitmapFont, TextFormat};

/// The width provider `Font` gives its splitter: the glyph's advance, plus its bold offset
/// for bold text.
pub fn char_width(font: &BitmapFont, ch: char, style: TextFormat) -> f32 {
    let glyph = font.glyph(ch);
    glyph.advance + if style.bold { glyph.bold_offset } else { 0.0 }
}

/// `Font.width` of a run of styled characters: the ceiling of the summed advances.
pub fn width(text: &[(char, TextFormat)], width_of: impl Fn(char, TextFormat) -> f32) -> i32 {
    let w: f32 = text.iter().map(|&(ch, style)| width_of(ch, style)).sum();
    w.ceil() as i32
}

/// `StringSplitter.splitLines`: the lines of `text` at `max_width` (at least 1), as ranges
/// into `text` appended to `out`. A line ends at a newline (dropped); otherwise once the
/// summed width passes the limit — after the first character of nonzero width, which always
/// stays — at the last space seen (dropped), or before the character that overflowed when
/// there was none. Each line restarts the count. A newline at the very end yields one more,
/// empty, line; an empty text yields none.
pub fn split_lines(
    text: &[(char, TextFormat)],
    max_width: i32,
    width_of: impl Fn(char, TextFormat) -> f32,
    out: &mut Vec<Range<usize>>,
) {
    let max_width = max_width.max(1) as f32;
    let mut start = 0;
    let mut force_new_line = false;
    // `LineBreakFinder`, run again from the tail after each break.
    while let Some(split) = find_line_break(&text[start..], max_width, &width_of) {
        let split = start + split;
        let tail_first = text[split].0;
        let new_line = tail_first == '\n';
        force_new_line = new_line;
        out.push(start..split);
        start = split + usize::from(new_line || tail_first == ' ');
    }
    if start < text.len() {
        out.push(start..text.len());
    } else if force_new_line {
        out.push(text.len()..text.len());
    }
}

/// One `LineBreakFinder` pass: the index of the first character of the tail, `None` when the
/// whole text fits.
fn find_line_break(
    text: &[(char, TextFormat)],
    max_width: f32,
    width_of: &impl Fn(char, TextFormat) -> f32,
) -> Option<usize> {
    let mut width = 0.0;
    let mut had_non_zero_width = false;
    let mut last_space = None;
    for (i, &(ch, style)) in text.iter().enumerate() {
        if ch == '\n' {
            return Some(i);
        }
        if ch == ' ' {
            last_space = Some(i);
        }
        let char_width = width_of(ch, style);
        width += char_width;
        if !had_non_zero_width || width <= max_width {
            had_non_zero_width |= char_width != 0.0;
        } else {
            return Some(last_space.unwrap_or(i));
        }
    }
    None
}
