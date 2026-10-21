//! The display entities (vanilla `Display` and its `BlockDisplay`, `ItemDisplay` and
//! `TextDisplay` subclasses): their synched data, the client-side render state `Display.tick`
//! rebuilds when that data changes, and the per-frame state `DisplayRenderer.extractRenderState`
//! reads from it (the interpolators evaluated at `calculateInterpolationProgress`).
//!
//! Synched data indices follow `defineSynchedData` order: `Entity` owns 0-7, `Display` 8-22,
//! then each subclass from 23.

mod text_display;
pub mod transformation;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU32, Ordering};

use haste_core::display::{Billboard, DisplayContent, DisplayDecoration, ItemDisplayContext};
use haste_core::entity::{MetadataEntry, MetadataValue};
use haste_core::{BlockStateId, RegistryIndex};

use crate::lerp::LerpState;
use crate::metadata::Metadata;
use crate::store::Entity;
pub use text_display::TextRenderState;
pub use transformation::Transformation;

/// `Display.DATA_TRANSFORMATION_INTERPOLATION_START_DELTA_TICKS_ID`.
pub const INTERPOLATION_START_DELTA: u8 = 8;
/// `Display.DATA_TRANSFORMATION_INTERPOLATION_DURATION_ID`.
pub const INTERPOLATION_DURATION: u8 = 9;
/// `Display.DATA_POS_ROT_INTERPOLATION_DURATION_ID` (the `teleport_duration` tag).
pub const POS_ROT_INTERPOLATION_DURATION: u8 = 10;
/// `Display.DATA_TRANSLATION_ID` .. `DATA_RIGHT_ROTATION_ID`.
pub const TRANSLATION: u8 = 11;
pub const SCALE: u8 = 12;
pub const LEFT_ROTATION: u8 = 13;
pub const RIGHT_ROTATION: u8 = 14;
/// `Display.DATA_BILLBOARD_RENDER_CONSTRAINTS_ID`.
pub const BILLBOARD: u8 = 15;
/// `Display.DATA_BRIGHTNESS_OVERRIDE_ID` (packed like light coords, -1 for none).
pub const BRIGHTNESS_OVERRIDE: u8 = 16;
pub const VIEW_RANGE: u8 = 17;
pub const SHADOW_RADIUS: u8 = 18;
pub const SHADOW_STRENGTH: u8 = 19;
pub const WIDTH: u8 = 20;
pub const HEIGHT: u8 = 21;
pub const GLOW_COLOR_OVERRIDE: u8 = 22;
/// `BlockDisplay.DATA_BLOCK_STATE_ID`, `ItemDisplay.DATA_ITEM_STACK_ID` and
/// `TextDisplay.DATA_TEXT_ID`: each subclass's first slot.
pub const SUBCLASS_FIRST: u8 = 23;
/// `ItemDisplay.DATA_ITEM_DISPLAY_ID`.
pub const ITEM_DISPLAY_CONTEXT: u8 = 24;

/// `Display.RENDER_STATE_IDS`: the synched fields whose change rebuilds the render state.
const RENDER_STATE_IDS: [u8; 8] = [
    TRANSLATION,
    SCALE,
    LEFT_ROTATION,
    RIGHT_ROTATION,
    BILLBOARD,
    BRIGHTNESS_OVERRIDE,
    SHADOW_RADIUS,
    SHADOW_STRENGTH,
];

/// Which `Display` subclass an entity type is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    Block,
    Item,
    Text,
}

impl DisplayKind {
    /// The subclass of a `minecraft:entity_type` name, `None` for every other type.
    pub fn for_name(name: &str) -> Option<DisplayKind> {
        match name {
            "minecraft:block_display" => Some(DisplayKind::Block),
            "minecraft:item_display" => Some(DisplayKind::Item),
            "minecraft:text_display" => Some(DisplayKind::Text),
            _ => None,
        }
    }

    pub fn of(kind: RegistryIndex) -> Option<DisplayKind> {
        match crate::registry::decoration_kind(kind)? {
            crate::registry::DecorationKind::Display(k) => Some(k),
            _ => None,
        }
    }

    /// Whether a synched field of this subclass rebuilds the render state
    /// (`BlockDisplay`/`ItemDisplay.onSyncedDataUpdated`, `TextDisplay.TEXT_RENDER_STATE_IDS`).
    fn sub_state_id(self, index: u8) -> bool {
        match self {
            DisplayKind::Block => index == SUBCLASS_FIRST,
            DisplayKind::Item => index == SUBCLASS_FIRST || index == ITEM_DISPLAY_CONTEXT,
            DisplayKind::Text => (SUBCLASS_FIRST..=text_display::STYLE_FLAGS).contains(&index),
        }
    }
}

/// `Display.createInterpolationHandler`: displays start with a zero-step position window
/// (`LinearInterpolationHandler.create(this, 0)`: packets snap) until their synched teleport
/// duration says otherwise; every other entity keeps the default window.
pub(crate) fn lerp_state(kind: RegistryIndex, mut lerp: LerpState) -> LerpState {
    if DisplayKind::of(kind).is_some() {
        lerp.set_steps(0);
    }
    lerp
}

/// `Display.RenderState`: the transformation and shadow interpolators plus the plain fields,
/// as of the last rebuild.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderState {
    /// The previous transformation (the one in force at `lastProgress` when this state was
    /// built) and the current one; `None` is `GenericInterpolator.constant(current)`.
    pub transformation_from: Option<Transformation>,
    pub transformation: Transformation,
    pub billboard: Billboard,
    pub brightness_override: i32,
    /// (previous, current): `LinearFloatInterpolator`, or a constant when both are equal.
    pub shadow_radius: (f32, f32),
    pub shadow_strength: (f32, f32),
    pub glow_color_override: i32,
}

impl RenderState {
    /// `renderState.transformation().get(progress)`: `TransformationInterpolator.get` (the
    /// current one from progress 1 on, else the part-wise slerp), or the constant.
    pub fn transformation_at(&self, progress: f32) -> Transformation {
        match &self.transformation_from {
            Some(from) if progress < 1.0 => from.slerp(&self.transformation, progress),
            _ => self.transformation,
        }
    }
}

/// The subclass render state (`BlockRenderState`, `ItemRenderState`, `TextRenderState`).
#[derive(Clone, Debug, PartialEq)]
pub enum SubState {
    Block(BlockStateId),
    Item(Option<RegistryIndex>, ItemDisplayContext),
    Text(TextRenderState),
}

/// A display entity's client state (`Display`'s non-synched fields).
#[derive(Debug)]
pub struct DisplayState {
    pub kind: DisplayKind,
    /// `interpolationStartClientTick` (starts at `Integer.MIN_VALUE`).
    pub interpolation_start_client_tick: i64,
    /// `interpolationDuration` as of the last tick that applied it.
    pub interpolation_duration: i32,
    /// `lastProgress`: the progress the last frame computed (`calculateInterpolationProgress`
    /// writes it while rendering, the next rebuild starts from it), as `f32` bits.
    last_progress: AtomicU32,
    update_render_state: bool,
    update_start_tick: bool,
    update_interpolation_duration: bool,
    pub render_state: Option<RenderState>,
    pub sub_state: Option<SubState>,
}

impl DisplayState {
    /// The state of a fresh display entity of type `kind`, `None` for other types.
    pub fn for_kind(kind: RegistryIndex) -> Option<DisplayState> {
        DisplayKind::of(kind).map(DisplayState::new)
    }

    pub fn new(kind: DisplayKind) -> DisplayState {
        DisplayState {
            kind,
            interpolation_start_client_tick: i64::from(i32::MIN),
            interpolation_duration: 0,
            last_progress: AtomicU32::new(0.0f32.to_bits()),
            update_render_state: false,
            update_start_tick: false,
            update_interpolation_duration: false,
            render_state: None,
            sub_state: None,
        }
    }

    pub fn last_progress(&self) -> f32 {
        f32::from_bits(self.last_progress.load(Ordering::Relaxed))
    }

    /// `Display.onSyncedDataUpdated` for one changed index (the position window's length is
    /// applied by [`on_metadata`], which owns the entity's interpolation handler).
    pub fn on_synced_data_updated(&mut self, index: u8) {
        if index == INTERPOLATION_START_DELTA {
            self.update_start_tick = true;
        }
        if index == INTERPOLATION_DURATION {
            self.update_interpolation_duration = true;
        }
        if RENDER_STATE_IDS.contains(&index) || self.kind.sub_state_id(index) {
            self.update_render_state = true;
        }
    }

    /// `Display.tick` on the client, `tick_count` already advanced by this tick's
    /// `commonTick`: apply a new interpolation start and duration, then rebuild the render
    /// state from the synched data — interpolating from where the last frame left it when a
    /// duration is set, fresh otherwise — and the subclass state with it.
    pub fn tick(&mut self, metadata: &Metadata, tick_count: u64) {
        if self.update_start_tick {
            self.update_start_tick = false;
            let delay = int(metadata, INTERPOLATION_START_DELTA, 0);
            self.interpolation_start_client_tick = tick_count as i64 + i64::from(delay);
        }
        if self.update_interpolation_duration {
            self.update_interpolation_duration = false;
            self.interpolation_duration = int(metadata, INTERPOLATION_DURATION, 0);
        }
        if self.update_render_state {
            self.update_render_state = false;
            let interpolate = self.interpolation_duration != 0;
            let progress = self.last_progress();
            let fresh = fresh_render_state(metadata);
            self.render_state = Some(match &self.render_state {
                Some(previous) if interpolate => RenderState {
                    transformation_from: Some(previous.transformation_at(progress)),
                    shadow_radius: (
                        lerp(progress, previous.shadow_radius),
                        fresh.shadow_radius.1,
                    ),
                    shadow_strength: (
                        lerp(progress, previous.shadow_strength),
                        fresh.shadow_strength.1,
                    ),
                    ..fresh
                },
                _ => fresh,
            });
            self.update_sub_state(metadata, interpolate, progress);
        }
    }

    /// `updateRenderSubState` of the subclass.
    fn update_sub_state(&mut self, metadata: &Metadata, interpolate: bool, progress: f32) {
        self.sub_state = Some(match self.kind {
            DisplayKind::Block => SubState::Block(match metadata.get(SUBCLASS_FIRST) {
                Some(MetadataEntry {
                    value: MetadataValue::BlockState(s),
                    ..
                }) => *s,
                _ => BlockStateId::AIR,
            }),
            DisplayKind::Item => {
                let item = match metadata.get(SUBCLASS_FIRST).map(|e| &e.value) {
                    Some(MetadataValue::ItemStack(s)) if !s.is_empty() => s.item,
                    _ => None,
                };
                let context = ItemDisplayContext::from_id(byte(metadata, ITEM_DISPLAY_CONTEXT, 0));
                SubState::Item(item, context)
            }
            DisplayKind::Text => {
                let previous = match &self.sub_state {
                    Some(SubState::Text(t)) if interpolate => Some(t),
                    _ => None,
                };
                SubState::Text(TextRenderState::build(metadata, previous, progress))
            }
        });
    }

    /// `Display.calculateInterpolationProgress(partialTick)`: how far the current
    /// transformation interpolation is, 1 when there is none; remembered as `lastProgress`.
    pub fn interpolation_progress(&self, tick_count: u64, partial_tick: f32) -> f32 {
        let duration = self.interpolation_duration;
        if duration <= 0 {
            return 1.0;
        }
        let since = (tick_count as i64 - self.interpolation_start_client_tick) as f32;
        let progress = ((since + partial_tick) / duration as f32).clamp(0.0, 1.0);
        self.last_progress
            .store(progress.to_bits(), Ordering::Relaxed);
        progress
    }
}

/// `Display.createFreshRenderState`: every interpolator a constant of the synched value.
fn fresh_render_state(m: &Metadata) -> RenderState {
    let shadow_radius = float(m, SHADOW_RADIUS, 0.0);
    let shadow_strength = float(m, SHADOW_STRENGTH, 1.0);
    RenderState {
        transformation_from: None,
        transformation: transformation(m),
        billboard: Billboard::from_id(byte(m, BILLBOARD, 0)),
        brightness_override: int(m, BRIGHTNESS_OVERRIDE, -1),
        shadow_radius: (shadow_radius, shadow_radius),
        shadow_strength: (shadow_strength, shadow_strength),
        glow_color_override: int(m, GLOW_COLOR_OVERRIDE, -1),
    }
}

/// `Display.createTransformation`: the four synched parts, defaults where unsent.
pub fn transformation(m: &Metadata) -> Transformation {
    let id = Transformation::IDENTITY;
    Transformation {
        translation: vector3(m, TRANSLATION).unwrap_or(id.translation),
        left_rotation: quaternion(m, LEFT_ROTATION).unwrap_or(id.left_rotation),
        scale: vector3(m, SCALE).unwrap_or(id.scale),
        right_rotation: quaternion(m, RIGHT_ROTATION).unwrap_or(id.right_rotation),
    }
}

/// `Mth.lerp(progress, previous, current)`.
fn lerp(progress: f32, (from, to): (f32, f32)) -> f32 {
    from + progress * (to - from)
}

/// `Display.onSyncedDataUpdated` for a metadata packet: flag the rebuilds, and apply a new
/// teleport duration to the position window (`setInterpolationLength`).
pub(crate) fn on_metadata(e: &mut Entity, entries: &[MetadataEntry]) {
    let Some(display) = e.display.as_mut() else {
        return;
    };
    for entry in entries {
        display.on_synced_data_updated(entry.index);
    }
    if entries
        .iter()
        .any(|entry| entry.index == POS_ROT_INTERPOLATION_DURATION)
    {
        let steps = int(&e.metadata, POS_ROT_INTERPOLATION_DURATION, 0);
        e.set_interpolation_steps(steps.clamp(0, i32::from(u8::MAX)) as u8);
    }
}

/// Each block display's light emission (`BlockModelResolver.update` reads the state's
/// `getLightEmission`), filled on the frame's instances from the block data the entity store
/// does not hold.
pub fn fill_light_emission(
    instances: &mut [haste_core::model::EntityInstance],
    blocks: &dyn haste_core::view::BlockPhysics,
) {
    for inst in instances {
        if let Some(haste_core::model::Decoration::Display(DisplayDecoration {
            content:
                DisplayContent::Block {
                    state,
                    light_emission,
                },
            ..
        })) = &mut inst.decoration
        {
            *light_emission = blocks.light_emission(*state);
        }
    }
}

/// The frame's render state of a display entity (`DisplayRenderer.extractRenderState` and
/// the subclasses'), `None` until the first rebuild produced both states (`submit` draws
/// nothing then). Also returns the interpolated shadow (`getShadowRadius`,
/// `getShadowStrength`).
pub fn decoration(e: &Entity, partial_tick: f32) -> Option<(DisplayDecoration, f32, f32)> {
    let display = e.display.as_ref()?;
    let state = display.render_state.as_ref()?;
    let progress = display.interpolation_progress(e.tick_count, partial_tick);
    let content = match display.sub_state.as_ref()? {
        SubState::Block(s) => DisplayContent::Block {
            state: *s,
            // Filled per frame by `fill_light_emission`.
            light_emission: 0,
        },
        SubState::Item(item, context) => DisplayContent::Item {
            item: *item,
            context: *context,
        },
        SubState::Text(t) => DisplayContent::Text(t.at(progress)),
    };
    let brightness = (state.brightness_override != -1).then(|| {
        // `LightCoordsUtil.block` / `sky` of the packed override.
        let packed = state.brightness_override;
        (((packed >> 4) & 15) as u8, ((packed >> 20) & 15) as u8)
    });
    let decoration = DisplayDecoration {
        transformation: state.transformation_at(progress).matrix(),
        billboard: state.billboard,
        entity_y_rot: e.yaw_at(partial_tick),
        entity_x_rot: e.pitch_at(partial_tick),
        brightness,
        view_range: float(&e.metadata, VIEW_RANGE, 1.0),
        width: float(&e.metadata, WIDTH, 0.0),
        height: float(&e.metadata, HEIGHT, 0.0),
        glow_color_override: state.glow_color_override,
        content,
    };
    Some((
        decoration,
        lerp(progress, state.shadow_radius),
        lerp(progress, state.shadow_strength),
    ))
}

pub(crate) fn int(m: &Metadata, index: u8, default: i32) -> i32 {
    match m.get(index).map(|e| &e.value) {
        Some(MetadataValue::VarInt(v)) => *v,
        _ => default,
    }
}

pub(crate) fn byte(m: &Metadata, index: u8, default: i8) -> i8 {
    match m.get(index).map(|e| &e.value) {
        Some(MetadataValue::Byte(v)) => *v,
        _ => default,
    }
}

pub(crate) fn float(m: &Metadata, index: u8, default: f32) -> f32 {
    match m.get(index).map(|e| &e.value) {
        Some(MetadataValue::Float(v)) => *v,
        _ => default,
    }
}

/// `EntityDataSerializers.VECTOR3` (wire type 39): three big-endian floats.
fn vector3(m: &Metadata, index: u8) -> Option<glam::Vec3> {
    let [x, y, z] = floats::<3>(m, index, 39)?;
    Some(glam::Vec3::new(x, y, z))
}

/// `EntityDataSerializers.QUATERNION` (wire type 40): x, y, z, w as big-endian floats.
fn quaternion(m: &Metadata, index: u8) -> Option<glam::Vec4> {
    let [x, y, z, w] = floats::<4>(m, index, 40)?;
    Some(glam::Vec4::new(x, y, z, w))
}

fn floats<const N: usize>(m: &Metadata, index: u8, wire_type: i32) -> Option<[f32; N]> {
    let raw = match &m.get(index)?.value {
        MetadataValue::Other { type_id, raw } if *type_id == wire_type => raw,
        _ => return None,
    };
    let mut out = [0.0; N];
    for (i, v) in out.iter_mut().enumerate() {
        *v = f32::from_be_bytes(raw.get(i * 4..i * 4 + 4)?.try_into().ok()?);
    }
    Some(out)
}
