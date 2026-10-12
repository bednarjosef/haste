//! Key numbering of vanilla 26.3 (`com.mojang.blaze3d.platform.InputConstants`, SDL3 based).
//!
//! A 26.3 `KeyEvent` carries two numbers: `key`, the SDL3 *scancode* (physical key, USB HID usage id;
//! the `InputConstants.KEY_*` values that key mappings, Escape and Enter match) and `keycode`, the SDL
//! *keycode* (layout-dependent; observed: `EditBox` matches Backspace and the arrow keys on it).
//! The client must pass both: [`crate::GuiPipeline::key`]'s `key` and `scancode` arguments are
//! `KeyEvent.key` and `KeyEvent.keycode`. [`keycode`] gives the US-layout keycode for a scancode.
//! Modifiers are SDL keymod bits, mouse buttons SDL button numbers.
#![allow(missing_docs)]

pub const A: i32 = 4;
pub const E: i32 = 8;
pub const Q: i32 = 20;
pub const T: i32 = 23;
pub const NUM_1: i32 = 30;
pub const NUM_0: i32 = 39;
pub const RETURN: i32 = 40;
pub const ESCAPE: i32 = 41;
pub const BACKSPACE: i32 = 42;
pub const TAB: i32 = 43;
pub const SPACE: i32 = 44;
pub const SLASH: i32 = 56;
pub const HOME: i32 = 74;
pub const DELETE: i32 = 76;
pub const END: i32 = 77;
pub const RIGHT: i32 = 79;
pub const LEFT: i32 = 80;
pub const DOWN: i32 = 81;
pub const UP: i32 = 82;
pub const F1: i32 = 58;
pub const F3: i32 = 60;
pub const LCTRL: i32 = 224;
pub const LSHIFT: i32 = 225;

pub const MOD_SHIFT: i32 = 3;
pub const MOD_CONTROL: i32 = 192;
pub const MOD_ALT: i32 = 768;

pub const MOUSE_LEFT: i32 = 1;
pub const MOUSE_MIDDLE: i32 = 2;
pub const MOUSE_RIGHT: i32 = 3;

/// Scancode of letter key `c` (`'a'..='z'`, case-insensitive).
pub const fn letter(c: u8) -> i32 {
    A + (c.to_ascii_lowercase() - b'a') as i32
}

/// SDL3 "scancode mask": keycodes of keys without a character are `scancode | MASK`.
pub const SCANCODE_MASK: i32 = 1 << 30;

/// SDL3 keycode of a scancode on a US layout (SDL's default keymap): characters for printable
/// keys, ASCII control codes for Backspace/Tab/Return/Escape/Delete, `scancode | SCANCODE_MASK`
/// for the rest (arrows, Home/End, F-keys, modifiers).
pub fn keycode(scancode: i32) -> i32 {
    match scancode {
        4..=29 => i32::from(b'a') + scancode - 4,
        30..=38 => i32::from(b'1') + scancode - 30,
        39 => i32::from(b'0'),
        RETURN => 13,
        ESCAPE => 27,
        BACKSPACE => 8,
        TAB => 9,
        SPACE => 32,
        45 => i32::from(b'-'),
        46 => i32::from(b'='),
        47 => i32::from(b'['),
        48 => i32::from(b']'),
        49 => i32::from(b'\\'),
        51 => i32::from(b';'),
        52 => i32::from(b'\''),
        53 => i32::from(b'`'),
        54 => i32::from(b','),
        55 => i32::from(b'.'),
        SLASH => i32::from(b'/'),
        DELETE => 127,
        _ => scancode | SCANCODE_MASK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycodes() {
        assert_eq!(keycode(letter(b'E')), i32::from(b'e'));
        assert_eq!(keycode(BACKSPACE), 8);
        assert_eq!(keycode(UP), UP | SCANCODE_MASK);
        assert_eq!(keycode(NUM_0), i32::from(b'0'));
    }
}
