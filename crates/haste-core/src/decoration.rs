//! The special-entity decorations: the render state each entity renderer carries that draws
//! itself rather than a cube model (`Decoration` and the state struct per vanilla renderer).
//! Extracted from `model.rs` to keep that file within the size limit; `model` re-exports
//! everything, so the `haste_core::model::...` paths keep working.

use crate::ids::RegistryIndex;
use crate::{BlockStateId, Direction, Identifier};

/// A decoration the renderer draws itself rather than from a cube model.
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    ItemFrame(ItemFrameDecoration),
    Painting(PaintingDecoration),
    Lightning(LightningDecoration),
    ExperienceOrb(ExperienceOrbDecoration),
    FallingBlock(FallingBlockDecoration),
    PrimedTnt(PrimedTntDecoration),
    Minecart(MinecartDecoration),
    ThrownItem(ThrownItemDecoration),
    DragonFireball,
    Arrow(ArrowDecoration),
    /// `DisplayRenderer` (block, item and text displays).
    Display(crate::display::DisplayDecoration),
}

/// The state `ArrowRenderer.extractRenderState` collects for an arrow (vanilla
/// `ArrowRenderState` plus `TippableArrowRenderState.isTipped`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrowDecoration {
    /// `ArrowRenderState.xRot` (`ArrowRenderer.extractRenderState`:
    /// `entity.getXRot(partialTicks)`).
    pub x_rot: f32,
    /// `ArrowRenderState.yRot` (`entity.getYRot(partialTicks)`).
    pub y_rot: f32,
    /// `ArrowRenderState.shake`: `entity.shakeTime - partialTicks`, the stuck-in-ground
    /// shake (`AbstractArrow.shakeTime`, set to 7 by `onSyncedDataUpdated`).
    pub shake: f32,
    /// `TippableArrowRenderState.isTipped` (`Arrow.getColor() > 0`): the tipped texture.
    pub tipped: bool,
    /// The entity is a `minecraft:spectral_arrow` (`SpectralArrowRenderer`'s own texture,
    /// never the tipped one).
    pub spectral: bool,
}

/// The state `FallingBlockRenderer.extractRenderState` collects for a falling block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FallingBlockDecoration {
    /// The entity's block state (`FallingBlockEntity.getBlockState`, sent as the spawn
    /// packet's data field: `Block.getId` of the state).
    pub block_state: BlockStateId,
    /// The world's block at the entity's position is the falling state: the block has
    /// landed, and `FallingBlockRenderer.shouldRender` draws nothing (set by the client,
    /// which owns the world view).
    pub landed: bool,
}

/// The state `TntRenderer.extractRenderState` collects for a primed TNT.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimedTntDecoration {
    /// The entity's block state (`PrimedTnt.DATA_BLOCK_STATE_ID`; the TNT block's default
    /// state when the server sent none).
    pub block_state: BlockStateId,
    /// `TntRenderState.fuseRemainingInTicks`: `getFuse() - partialTicks + 1`.
    pub fuse: f32,
}

/// A minecart's display block (`AbstractMinecart.getDisplayBlockState`), resolved to the block
/// and its properties once, when the synched data or the fuel flag changes; the renderer bakes
/// its quads once per state id.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDisplay {
    /// The block state id (the renderer's quad-cache key, and air's check).
    pub state: BlockStateId,
    /// The state's block.
    pub block: Identifier,
    /// The state's properties, for the blockstate file's variant choice.
    pub props: Vec<(String, String)>,
}

/// The state `AbstractMinecartRenderer.extractRenderState` collects for a minecart
/// (`MinecartRenderState`), old-behavior rendering only (the new-style track packet is a
/// separate task).
#[derive(Clone, Debug, PartialEq)]
pub struct MinecartDecoration {
    /// The display block, `None` for air (nothing is drawn in the cart).
    pub display: Option<std::sync::Arc<BlockDisplay>>,
    /// `AbstractMinecart.getDisplayOffset` (the custom offset when the synched data set one).
    pub display_offset: i32,
    /// `getHurtTime() - partialTicks` (0 when unhurt).
    pub hurt_time: f32,
    /// `getHurtDir()` (1 or -1).
    pub hurt_dir: f32,
    /// `max(getDamage() - partialTicks, 0)`.
    pub damage_time: f32,
    /// The TNT cart's `getFuse() - partialTicks + 1`, or -1 when not primed.
    pub fuse: f32,
    /// `OldMinecartBehavior.getPos`/`getPosOffs`: the rail-snapped position plus the points
    /// 0.3 blocks front and back the cart's rotation is derived from, filled per frame from
    /// the world; `None` off the rails.
    pub rail: Option<(glam::DVec3, glam::DVec3, glam::DVec3)>,
}

/// The state `ExperienceOrbRenderer.submit` collects for an experience orb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExperienceOrbDecoration {
    /// The orb's `Value` metadata, which picks the icon tier (`ExperienceOrb.getIcon`).
    pub value: i32,
    /// The orb's age in ticks (plus the partial tick), driving the colour cycle.
    pub age: f32,
}

/// The state `ThrownItemRenderer` draws: which item (from `ItemSupplier.getItem`, the
/// synched stack or the type's default), at which scale, and whether the renderer lights it
/// full-bright (`ThrownItemRenderer`'s constructor arguments and
/// `ThrownItemRenderState`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrownItemDecoration {
    pub item: RegistryIndex,
    /// The renderer's `scale` constructor argument (1.0 for most, 3.0 for the large fireball,
    /// 0.75 for the small one).
    pub scale: f32,
    /// `ThrownItemRenderer.getBlockLightLevel` returns 15 for the eye of ender and the
    /// fireballs.
    pub full_bright: bool,
}

/// The state `ItemFrameRenderer.extractRenderState` collects for an item frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemFrameDecoration {
    /// The direction the frame faces (`ItemFrame.getDirection`, from the spawn packet's data).
    pub facing: Direction,
    /// `ItemFrame.getRotation` 0..7; the item turns `rotation * 45` degrees.
    pub rotation: u8,
    /// Glow item frames light the frame at least block-light 5 and the item full-bright.
    pub glow: bool,
    /// The framed item (`DATA_ITEM`), by item registry index.
    pub item: Option<RegistryIndex>,
    /// The framed stack's glint (`ItemStack.hasFoil`).
    pub item_foil: crate::model::FoilType,
    /// The framed stack's `minecraft:map_id` component (`ItemFrame.getFramedMapId`): the map
    /// whose contents the frame draws instead of the item model, when its data has arrived.
    pub map_id: Option<i32>,
}

/// The state `PaintingRenderer.extractRenderState` collects for a painting.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintingDecoration {
    /// The direction the painting faces (`Painting.getDirection`).
    pub facing: Direction,
    /// The variant's size in blocks (`PaintingVariant.width`/`height`).
    pub width: u8,
    pub height: u8,
    /// The front sprite's texture id, `None` for a variant we do not know (draw the back only).
    pub front: Option<Identifier>,
}

/// The state `LightningBoltRenderer.extractRenderState` collects for a lightning bolt: the
/// seed its geometry is generated from. Vanilla's seed is the entity's own random (not
/// synced, re-rolled per flash), so a seed derived from the entity id stands in — the bolt's
/// shape is procedural either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightningDecoration {
    pub seed: u64,
}
