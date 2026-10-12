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
