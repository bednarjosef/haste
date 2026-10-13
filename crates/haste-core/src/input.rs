//! Input as the physics tick sees it: a snapshot per tick, independent of winit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputState {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
    /// Look angles in degrees, vanilla convention.
    pub yaw: f32,
    pub pitch: f32,
}

/// Interaction input for one tick (mouse buttons and hotbar), next to [`InputState`] so the
/// movement tick keeps its own small type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractInput {
    /// Left mouse button held (attack / break).
    pub attack: bool,
    /// Right mouse button held (use item / place).
    pub use_item: bool,
    /// Middle click: true on the press tick only.
    pub pick_block: bool,
    /// Q: true on the press tick only.
    pub drop_item: bool,
    /// Ctrl+Q: true on the press tick only.
    pub drop_stack: bool,
    /// F: true on the press tick only.
    pub swap_offhand: bool,
    /// Hotbar slot (0..=8) selected this tick by a number key or the scroll wheel.
    pub hotbar: Option<u8>,
}
