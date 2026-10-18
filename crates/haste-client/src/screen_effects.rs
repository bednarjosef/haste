//! Screen effects: what vanilla's `ScreenEffectRenderer` and `Hud.extractCameraOverlays` draw
//! over the world, and the warp `GameRenderer.renderLevel` puts on the level's projection
//! while the player is nauseous or in a portal.
//!
//! The first three overlays belong to the level pass (`ScreenEffectRenderer.submit`: the block
//! the camera is inside, water when the eyes are in it, fire when the player burns), the rest
//! to the HUD (`Hud.extractCameraOverlays`: the worn item's camera overlay, the freeze outline,
//! the spyglass scope, the portal tint and the confusion overlay). All of them are appended to
//! the frame's `GuiFrame` command list and drawn by the HUD's batched GUI pass
//! (`haste_core::gui`), so they cost a handful of vertices in one draw list rather than a pass
//! of their own.
//!
//! State comes from what the client already owns: the fluid at the eye position, the own
//! entity's shared flags and freeze ticks from `set_entity_data`, the worn head item from
//! `set_equipment`, the mob effects from `update_mob_effect`, and the block around the eye.
//! Every constant names the vanilla source it comes from; nothing is guessed.

use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;

mod camera_overlay;
mod confusion;
mod draw;
mod fire;
mod in_block;
mod mob_effect;
mod portal;
mod powder_snow;
mod probe;
mod spyglass;
#[cfg(test)]
mod tests;
mod water;

pub use camera_overlay::camera_overlay;
#[cfg(test)]
use draw::argb;
use draw::white;
#[allow(unused_imports)]
pub use in_block::view_probe_points;
use in_block::BLOCK_OVERLAY_TINT;
pub use mob_effect::Effect;
#[allow(unused_imports)]
pub use portal::is_portal_block;
pub use portal::PortalEffects;
use powder_snow::powder_snow_outline;
pub use powder_snow::{percent_frozen, tick_frozen};
pub use probe::{Probe, WorldProbe};
pub use spyglass::is_spyglass;
#[cfg(test)]
pub use water::light_brightness;

/// Vanilla's `Entity.getTicksRequiredToFreeze` (`getPercentFrozen` divides by it).
pub const FREEZE_TICKS: i32 = 140;
/// Vanilla's `MobEffects.NAUSEA` blend durations (`setBlendDuration(150, 20, 60)`).
pub const NAUSEA_BLEND_IN: i32 = 150;
pub const NAUSEA_BLEND_OUT: i32 = 20;
pub const NAUSEA_BLEND_OUT_ADVANCE: i32 = 60;
/// Vanilla's `MobEffects.DARKNESS` blend duration (`setBlendDuration(22)`).
pub const DARKNESS_BLEND: i32 = 22;
/// Vanilla's `options.screenEffectScale` default (`screenEffectScale` runs 0..=1 and is 1.0
/// unless the player lowers it): the nausea texture overlay is only drawn below it, and it
/// scales the projection warp. Haste has no screen-effect-scale option yet, so it takes the
/// vanilla default.
pub const SCREEN_EFFECT_SCALE: f32 = 1.0;

/// The projection warp's strength and angle for one frame
/// (`GameRenderer.renderLevel`'s `spinningEffectIntensity`). The renderer applies it to the
/// level's projection matrix.
pub use haste_render::renderer::SpinWarp;

/// Vanilla's `Mth.lerp(delta, start, end)`.
pub fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + (end - start) * delta
}

/// The HUD field of view the screen effects are projected with
/// (`Camera.calculateHudFov`: 70, shrunk underwater by `modifyFovBasedOnDeathOrFluid`'s
/// `lerp(fovEffectScale, 1, 0.857)`, whose default `fovEffectScale` is 1).
pub fn hud_fov(eye_in_water: bool) -> f32 {
    if eye_in_water {
        70.0 * 0.857_142_87
    } else {
        70.0
    }
}

/// Our own entity's bits the screen effects need, from our own `set_entity_data` (vanilla's
/// `Entity.entityData`): the shared flags byte's on-fire flag and `DATA_TICKS_FROZEN`.
#[derive(Clone, Debug, Default)]
pub struct OwnFlags {
    /// `Entity.isOnFire` on the client: shared flag 0 (`DATA_SHARED_FLAGS_ID`, metadata index 0).
    pub on_fire: bool,
    /// `Entity.getTicksFrozen` (metadata index 7, `DATA_TICKS_FROZEN`), the powder-snow
    /// outline's alpha.
    pub frozen_ticks: i32,
}

/// The screen-effect state for one tick, copied out of the game thread's state.
#[derive(Clone, Debug, Default)]
pub struct ViewEffects {
    /// The camera is inside a view-blocking block: its particle texture over the whole screen
    /// (`ScreenEffectRenderer.submitBlockSprite`). Vanilla skips it in a bed, where the sleep
    /// overlay takes over instead.
    pub block_overlay: Option<Identifier>,
    /// The eyes are in water, and the brighter of the sky and block light at the eye
    /// (`LevelExtractor.extractPlayerState`'s water overlay colour).
    pub eye_in_water: bool,
    pub eye_light: u8,
    /// The player is on fire (the own entity's shared flag, `Entity.isOnFire`).
    pub on_fire: bool,
    /// The worn head item's camera overlay (vanilla's equippable component, only
    /// `carved_pumpkin` in vanilla).
    pub camera_overlay: Option<Identifier>,
    /// Freeze ticks, the powder-snow outline's alpha (`Hud.extractCameraOverlays`).
    pub frozen_ticks: i32,
    /// Holding a spyglass to the eye (`Player.isScoping`), in first person.
    pub scoping: bool,
    /// The portal intensity pair: (`oPortalEffectIntensity`, `portalEffectIntensity`).
    pub portal: (f32, f32),
    /// The nausea blend factor pair (`BlendState.factorPreviousFrame`, `.factor`).
    pub nausea: (f32, f32),
    /// The spinning effect time and speed (`LocalPlayer.getSpinningEffectAngle`).
    pub spinning_time: f32,
    pub spinning_speed: f32,
}

impl ViewEffects {
    /// The projection warp this frame (`GameRenderer.renderLevel`): `max(portal, nausea) *
    /// screenEffectScale²`, `None` when nothing is spinning.
    pub fn spin_warp(&self, partial_tick: f32) -> Option<SpinWarp> {
        let nausea = self.nausea(partial_tick);
        let intensity = (lerp(partial_tick, self.portal.0, self.portal.1).max(nausea))
            * SCREEN_EFFECT_SCALE
            * SCREEN_EFFECT_SCALE;
        (intensity > 0.0).then_some(SpinWarp {
            angle_deg: self.spinning_time + partial_tick * self.spinning_speed,
            intensity,
        })
    }

    /// The nausea factor between two ticks, for the spinning effect.
    pub fn nausea(&self, partial_tick: f32) -> f32 {
        lerp(partial_tick, self.nausea.0, self.nausea.1)
    }
}

/// One frame's screen effects with what they are drawn into: the GUI-pixel size of the
/// window, the interpolated look and the frame's partial tick.
pub struct Frame<'a> {
    /// The GUI frame's size in GUI pixels.
    pub width: u32,
    pub height: u32,
    /// The camera is our own eyes (`options.getCameraType().isFirstPerson()`): vanilla draws
    /// water, fire and the worn item's overlay in first person only.
    pub first_person: bool,
    pub effects: &'a ViewEffects,
    /// This frame's fraction of the tick.
    pub partial_tick: f32,
    /// The look angles (yaw, pitch) the water overlay scrolls by.
    pub look: (f32, f32),
    /// The spyglass scope's growth, moved per frame by the caller (`Hud.scopeScale`).
    pub scope_scale: f32,
}

impl Frame<'_> {
    /// This frame's screen-effect commands, in vanilla's draw order: the level pass's block,
    /// water and fire overlays (`ScreenEffectRenderer.submit`) first, then the HUD's camera
    /// overlays (`Hud.extractCameraOverlays`: the item overlay, the freeze outline, the
    /// spyglass scope, then the portal tint or the confusion overlay).
    ///
    /// They are inserted one by one at the front of the HUD frame's command list, so this walks
    /// them back to front and the HUD's own commands (the vignette, crosshair, hotbar) stay on
    /// top. Inserting rather than a scratch buffer keeps a frame free of both passes.
    pub fn append_to(&self, out: &mut Vec<GuiCommand>) {
        // Every overlay is prepended, so this walks vanilla's draw order back to front and the
        // HUD's own commands (the vignette, crosshair, hotbar) stay on top.

        // `Hud.extractCameraOverlays`, drawn last: the portal tint or the confusion overlay.
        let portal = lerp(
            self.partial_tick,
            self.effects.portal.0,
            self.effects.portal.1,
        );
        let nausea = lerp(
            self.partial_tick,
            self.effects.nausea.0,
            self.effects.nausea.1,
        );
        if portal > 0.0 {
            out.insert(0, self.portal_overlay(portal));
        } else if nausea > 0.0 && SCREEN_EFFECT_SCALE < 1.0 {
            let strength = nausea * (1.0 - SCREEN_EFFECT_SCALE);
            if strength > 0.0 {
                out.insert(0, self.confusion_overlay(strength));
            }
        }
        // The powder-snow freeze outline, whenever we are freezing at all, before the portal.
        if self.effects.frozen_ticks > 0 {
            let frozen = percent_frozen(self.effects.frozen_ticks);
            out.insert(
                0,
                self.full_screen(powder_snow_outline(), white(frozen), GuiBlend::Normal),
            );
        }
        // The spyglass scope or the worn item's camera overlay, in first person. Vanilla draws
        // the scope when scoping, otherwise the head item's overlay (never both).
        if self.first_person {
            if self.effects.scoping {
                // `extractSpyglassOverlay`: the scope texture, then the four black fills.
                for c in self.spyglass_overlay().into_iter().rev() {
                    out.insert(0, c);
                }
            } else if let Some(texture) = &self.effects.camera_overlay {
                out.insert(
                    0,
                    self.full_screen(texture.clone(), white(1.0), GuiBlend::Normal),
                );
            }
        }
        // `ScreenEffectRenderer.submit`, the level pass: fire and water in first person, then
        // the block the camera is inside.
        if self.effects.on_fire && self.first_person {
            for c in self.fire_overlay() {
                out.insert(0, c);
            }
        }
        if self.effects.eye_in_water && self.first_person {
            self.water_overlay(out);
        }
        if let Some(texture) = &self.effects.block_overlay {
            out.insert(
                0,
                self.full_screen(texture.clone(), BLOCK_OVERLAY_TINT, GuiBlend::Normal),
            );
        }
    }
}
