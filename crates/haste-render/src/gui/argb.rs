//! Colour helpers for GUI vertex colours (vanilla `ARGB`).

/// ARGB (vanilla) to RGBA bytes.
pub(super) fn rgba(argb: u32) -> [u8; 4] {
    [
        (argb >> 16) as u8,
        (argb >> 8) as u8,
        argb as u8,
        (argb >> 24) as u8,
    ]
}

/// Multiply two RGBA colours.
pub(super) fn mul(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    std::array::from_fn(|i| ((a[i] as u32 * b[i] as u32 + 127) / 255) as u8)
}

/// Vanilla's text shadow colour: RGB scaled by 0.25, alpha kept.
pub(super) fn shadow_color(argb: u32) -> u32 {
    let c = |s: u32| (((argb >> s) & 0xFF) / 4) << s;
    (argb & 0xFF00_0000) | c(16) | c(8) | c(0)
}

/// Sprite/texture tints: 0 (unset) draws untinted.
pub(super) fn tint_or_white(tint: u32) -> u32 {
    if tint == 0 {
        0xFFFF_FFFF
    } else {
        tint
    }
}
