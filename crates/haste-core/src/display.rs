//! The render state of the display entities (`minecraft:block_display`, `item_display`,
//! `text_display`): what vanilla's `DisplayRenderer.extractRenderState` and its Block/Item/Text
//! subclasses collect each frame (`DisplayEntityRenderState` and its subclasses), with the
//! `Display.RenderState` interpolators already evaluated at the frame's progress. The entity
//! crate builds it (`haste_entity::display`), the renderer draws it
//! (`haste_render::entity::display`).

use std::sync::Arc;

use crate::{BlockStateId, Component, RegistryIndex};

/// `Display.BillboardConstraints`: which of the entity's rotations the camera's replace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Billboard {
    #[default]
    Fixed,
    Vertical,
    Horizontal,
    Center,
}

impl Billboard {
    /// `BillboardConstraints.BY_ID` (`ByIdMap.continuous` with `OutOfBoundsStrategy.ZERO`: an
    /// unknown id is `FIXED`).
    pub fn from_id(id: i8) -> Billboard {
        match id {
            1 => Billboard::Vertical,
            2 => Billboard::Horizontal,
            3 => Billboard::Center,
            _ => Billboard::Fixed,
        }
    }
}

/// `ItemDisplayContext`, by its network id (`ItemDisplayContext.BY_ID`, out-of-range ids are
/// `NONE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemDisplayContext {
    #[default]
    None,
    ThirdPersonLeftHand,
    ThirdPersonRightHand,
    FirstPersonLeftHand,
    FirstPersonRightHand,
    Head,
    Gui,
    Ground,
    Fixed,
    OnShelf,
}

impl ItemDisplayContext {
    /// `ItemDisplayContext.BY_ID` (`ByIdMap.continuous`, `OutOfBoundsStrategy.ZERO`).
    pub fn from_id(id: i8) -> ItemDisplayContext {
        use ItemDisplayContext::*;
        match id {
            1 => ThirdPersonLeftHand,
            2 => ThirdPersonRightHand,
            3 => FirstPersonLeftHand,
            4 => FirstPersonRightHand,
            5 => Head,
            6 => Gui,
            7 => Ground,
            8 => Fixed,
            9 => OnShelf,
            _ => None,
        }
    }
}

/// The state `DisplayRenderer.extractRenderState` collects for one display entity, the
/// interpolators evaluated at this frame's `interpolationProgress`.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayDecoration {
    /// `Display.RenderState.transformation().get(progress)`, composed into its matrix
    /// (`Transformation.getMatrix`: translate, left rotation, scale, right rotation).
    pub transformation: glam::Mat4,
    /// `Display.RenderState.billboardConstraints`.
    pub billboard: Billboard,
    /// `DisplayEntityRenderState.entityYRot` / `entityXRot` (`Entity.getYRot/getXRot` at the
    /// partial tick), degrees.
    pub entity_y_rot: f32,
    pub entity_x_rot: f32,
    /// `Display.RenderState.brightnessOverride` unpacked as (block, sky); `None` for
    /// `NO_BRIGHTNESS_OVERRIDE` (the light sampled at the entity is used).
    pub brightness: Option<(u8, u8)>,
    /// `Display.DATA_VIEW_RANGE_ID` (`shouldRenderAtSqrDistance`: `viewRange * 64 *
    /// viewScale` blocks).
    pub view_range: f32,
    /// `Display.DATA_WIDTH_ID` / `DATA_HEIGHT_ID`: the culling box (`updateCulling`); either
    /// one zero disables frustum culling.
    pub width: f32,
    pub height: f32,
    /// `Display.RenderState.glowColorOverride` (`-1`: the team colour), the glowing outline's
    /// colour (`Display.getTeamColor`).
    pub glow_color_override: i32,
    /// The subclass state (`hasSubState`).
    pub content: DisplayContent,
}

/// The subclass part of a display's render state.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayContent {
    /// `BlockDisplay.BlockRenderState`: the block state drawn as its block model, and its
    /// light emission (`BlockModelResolver.update`'s `blockLightCoords`: the state's light
    /// emission as block light, 15 for emissive states), filled in by the client where the
    /// block tables are.
    Block {
        state: BlockStateId,
        light_emission: u8,
    },
    /// `ItemDisplay.ItemRenderState`: the item (`None`: an empty stack, nothing drawn) and its
    /// display context.
    Item {
        item: Option<RegistryIndex>,
        context: ItemDisplayContext,
    },
    /// `TextDisplay.TextRenderState`.
    Text(TextDisplayDecoration),
}

/// `TextDisplay.TextRenderState` at this frame's progress.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDisplayDecoration {
    /// The text. The `Arc` is replaced whenever the render state is rebuilt, so the renderer
    /// can key its line-split cache (`TextDisplay.clientDisplayCache`) on the pointer.
    pub text: Arc<Component>,
    /// `DATA_LINE_WIDTH_ID`: the wrap width in text pixels.
    pub line_width: i32,
    /// `textOpacity().get(progress)` (`DATA_TEXT_OPACITY_ID`, a signed byte: -1 is opaque).
    pub opacity: i8,
    /// `backgroundColor().get(progress)` (ARGB, `DATA_BACKGROUND_COLOR_ID`).
    pub background: u32,
    /// `DATA_STYLE_FLAGS_ID`: shadow 1, see-through 2, default background 4, align left 8,
    /// align right 16.
    pub flags: u8,
}

impl TextDisplayDecoration {
    pub const FLAG_SHADOW: u8 = 1;
    pub const FLAG_SEE_THROUGH: u8 = 2;
    pub const FLAG_USE_DEFAULT_BACKGROUND: u8 = 4;
    pub const FLAG_ALIGN_LEFT: u8 = 8;
    pub const FLAG_ALIGN_RIGHT: u8 = 16;
}
