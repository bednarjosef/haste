//! Key and modifier numbering of vanilla 26.3 (`com.mojang.blaze3d.platform.InputConstants`):
//! SDL3 scancodes (USB HID usage ids) for keys, SDL keymod bits for modifiers, SDL button numbers
//! for the mouse. The client translates winit events into these before calling [`crate::GuiPipeline`].
#![allow(missing_docs)]

pub const A: i32 = 4;
pub const E: i32 = 8;
pub const T: i32 = 23;
pub const NUM_1: i32 = 30;
pub const RETURN: i32 = 40;
pub const ESCAPE: i32 = 41;
pub const BACKSPACE: i32 = 42;
pub const TAB: i32 = 43;
pub const SPACE: i32 = 44;
pub const SLASH: i32 = 56;
pub const DELETE: i32 = 76;
pub const RIGHT: i32 = 79;
pub const LEFT: i32 = 80;
pub const DOWN: i32 = 81;
pub const UP: i32 = 82;
pub const HOME: i32 = 74;
pub const END: i32 = 77;
pub const LSHIFT: i32 = 225;

/// Letter key `c` (`'a'..='z'`).
pub const fn letter(c: u8) -> i32 {
    A + (c.to_ascii_lowercase() - b'a') as i32
}

pub const MOD_SHIFT: i32 = 3;
pub const MOD_CONTROL: i32 = 192;
pub const MOD_ALT: i32 = 768;

pub const MOUSE_LEFT: i32 = 1;
pub const MOUSE_MIDDLE: i32 = 2;
pub const MOUSE_RIGHT: i32 = 3;
