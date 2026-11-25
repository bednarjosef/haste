//! The batched GUI pipeline. Tier-3 Java screens (and the Rust HUD) emit a `GuiFrame` of commands
//! once per frame; `haste-render` draws it. One JNI crossing per frame, never per draw call.
use crate::Identifier;

/// Packed ARGB like vanilla (`0xAARRGGBB`).
pub type Argb = u32;

/// How a texture is blended over what is already there. Vanilla's GUI pipelines choose one per
/// draw call; almost everything is `Normal` alpha blending, but the vignette multiplies (only
/// darkens the edges, src ZERO / dst ONE_MINUS_SRC_COLOR), the crosshair inverts the colour
/// behind it (src ONE_MINUS_DST_COLOR / dst ONE_MINUS_SRC_COLOR) and the text selection highlight
/// adds to what is already there (src ONE / dst ONE).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GuiBlend {
    /// Vanilla's `gui_textured`: straight alpha blending.
    #[default]
    Normal,
    /// Vanilla's `vignette`: the result is the product of source and destination (darkens).
    Multiply,
    /// Vanilla's `crosshair`: the source inverts the colour behind it.
    Invert,
    /// Vanilla's `gui_text_highlight`: the source is added over the destination (brightens).
    Additive,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GuiCommand {
    /// Solid rectangle, x1/y1 inclusive, x2/y2 exclusive, in GUI pixels (scaled).
    Fill {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: Argb,
    },
    /// Vertical gradient.
    FillGradient {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        top: Argb,
        bottom: Argb,
    },
    /// Text in the default font at GUI scale. `shadow` draws the vanilla drop shadow.
    Text {
        x: i32,
        y: i32,
        text: String,
        color: Argb,
        shadow: bool,
    },
    /// A region of a texture from the assets (`textures/...png` by identifier) drawn at x,y with
    /// width w,h, sampling u,v..u+uw,v+vh in texture pixels of a texture `tex_w` x `tex_h`.
    Texture {
        texture: Identifier,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        u: f32,
        v: f32,
        uw: f32,
        vh: f32,
        tex_w: u32,
        tex_h: u32,
        tint: Argb,
        /// The blend mode this blit was drawn with (vanilla picks per pipeline).
        blend: GuiBlend,
    },
    /// A GUI sprite from the `gui` atlas (`minecraft:hud/hotbar` etc.), nine-slice handled by the renderer.
    Sprite {
        sprite: Identifier,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        tint: Argb,
        /// The blend mode this sprite was drawn with (the crosshair inverts).
        blend: GuiBlend,
    },
    /// Item icon by item registry index (renderer looks up the model). `count` is informational:
    /// vanilla draws the count as a `Text` command itself, so the renderer must not draw it again.
    Item {
        item: crate::RegistryIndex,
        x: i32,
        y: i32,
        count: i32,
        /// The stack's glint (`ItemStack.hasFoil` and the foil type its model picks).
        foil: crate::model::FoilType,
    },
    /// A live entity drawn into a GUI box (inventory player model). The renderer renders the
    /// entity's model looking at the mouse position, clipped to the box.
    Entity {
        entity: crate::EntityId,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        scale: f32,
        mouse_x: f32,
        mouse_y: f32,
    },
    /// A sub-region (u,v,w,h in sprite pixels of a sprite_w x sprite_h sprite) of a GUI sprite.
    SpriteRegion {
        sprite: Identifier,
        sprite_w: i32,
        sprite_h: i32,
        u: i32,
        v: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        tint: Argb,
        /// The blend mode this sprite was drawn with (the crosshair inverts).
        blend: GuiBlend,
    },
    /// Text with vanilla style flags (one command per style run). `Text` is the unstyled form.
    StyledText {
        x: i32,
        y: i32,
        text: String,
        color: Argb,
        shadow: bool,
        style: TextStyle,
    },
    /// Scissor rectangle on/off, in GUI pixels.
    ScissorPush {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
    },
    ScissorPop,
    /// Translate subsequent commands by (dx, dy) (pushed), or pop.
    TranslatePush {
        dx: f32,
        dy: f32,
        dz: f32,
    },
    TranslatePop,
}

/// Scissor and translate commands apply to the commands that follow them; scissor rectangles are
/// given in the translated space in effect when they are pushed.
#[derive(Clone, Debug, Default)]
pub struct GuiFrame {
    pub commands: Vec<GuiCommand>,
    /// GUI scale factor used to lay the frame out (vanilla "GUI scale").
    pub scale: u32,
    pub width: u32,
    pub height: u32,
    /// Cursor shape the screen asked for this frame (`None` = default arrow).
    pub cursor: Option<CursorShape>,
    /// Command index the open screen's blurred background goes after: the commands before it
    /// draw over the world, the blur runs (vanilla: `GameRenderer.processBlurEffect`, the
    /// `blur` post chain), then the rest draw over the blurred result. `-1`: no blur (no
    /// screen open, or the screen does not blur — chat, the HUD). Vanilla:
    /// `Screen.extractBlurredBackground` / `GuiRenderer.draw`.
    pub blur_after: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Arrow,
    IBeam,
    Crosshair,
    PointingHand,
    ResizeNs,
    ResizeEw,
    ResizeAll,
    NotAllowed,
}

/// Text measurement the renderer provides to anyone laying out text (HUD, tier-3 `Font.width`).
pub trait TextMetrics: Send + Sync {
    fn width(&self, text: &str) -> i32;
    fn line_height(&self) -> i32 {
        9
    }
}
