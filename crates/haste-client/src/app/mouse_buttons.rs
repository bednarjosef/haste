//! Mouse buttons in vanilla's numbering (1 left, 2 middle, 3 right, the same as SDL's): the
//! window's buttons and the script's both go to the game's input under these ids.

use winit::event::MouseButton;

use crate::script;

/// A winit mouse button in vanilla's numbering (1 left, 2 middle, 3 right, the same as SDL's).
pub(super) fn vanilla_button(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Back => 4,
        MouseButton::Forward => 5,
        _ => 0,
    }
}

/// A script mouse button in vanilla's numbering (1 left, 2 middle, 3 right).
pub(super) fn mouse_button_id(button: script::MouseButton) -> i32 {
    match button {
        script::MouseButton::Left => 1,
        script::MouseButton::Middle => 2,
        script::MouseButton::Right => 3,
    }
}
