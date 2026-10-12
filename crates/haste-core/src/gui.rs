//! The batched GUI pipeline. Tier-3 Java screens (and the Rust HUD) emit a `GuiFrame` of commands
//! once per frame; `haste-render` draws it. One JNI crossing per frame, never per draw call.
use crate::Identifier;

/// Packed ARGB like vanilla (`0xAARRGGBB`).
pub type Argb = u32;

#[derive(Clone, Debug, PartialEq)]
pub enum GuiCommand {
    /// Solid rectangle, x1/y1 inclusive, x2/y2 exclusive, in GUI pixels (scaled).
    Fill { x1: i32, y1: i32, x2: i32, y2: i32, color: Argb },
    /// Vertical gradient.
    FillGradient { x1: i32, y1: i32, x2: i32, y2: i32, top: Argb, bottom: Argb },
    /// Text in the default font at GUI scale. `shadow` draws the vanilla drop shadow.
    Text { x: i32, y: i32, text: String, color: Argb, shadow: bool },
    /// A region of a texture from the assets (`textures/...png` by identifier) drawn at x,y with
    /// width w,h, sampling u,v..u+uw,v+vh in texture pixels of a texture `tex_w` x `tex_h`.
    Texture { texture: Identifier, x: i32, y: i32, w: i32, h: i32, u: f32, v: f32, uw: f32, vh: f32, tex_w: u32, tex_h: u32, tint: Argb },
    /// A GUI sprite from the `gui` atlas (`minecraft:hud/hotbar` etc.), nine-slice handled by the renderer.
    Sprite { sprite: Identifier, x: i32, y: i32, w: i32, h: i32, tint: Argb },
    /// Item icon by item registry index (renderer looks up the model).
    Item { item: crate::RegistryIndex, x: i32, y: i32, count: i32 },
    /// Scissor rectangle on/off, in GUI pixels.
    ScissorPush { x1: i32, y1: i32, x2: i32, y2: i32 },
    ScissorPop,
    /// Translate subsequent commands by (dx, dy) (pushed), or pop.
    TranslatePush { dx: f32, dy: f32, dz: f32 },
    TranslatePop,
}

#[derive(Clone, Debug, Default)]
pub struct GuiFrame {
    pub commands: Vec<GuiCommand>,
    /// GUI scale factor used to lay the frame out (vanilla "GUI scale").
    pub scale: u32,
    pub width: u32,
    pub height: u32,
}

/// Text measurement the renderer provides to anyone laying out text (HUD, tier-3 `Font.width`).
pub trait TextMetrics: Send + Sync {
    fn width(&self, text: &str) -> i32;
    fn line_height(&self) -> i32 { 9 }
}
