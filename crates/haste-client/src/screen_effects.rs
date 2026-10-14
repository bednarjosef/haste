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
/// Vanilla's `ModelBakery.FIRE_1`: the fire sprite of the block atlas (`fire_1`).
fn fire_sprite() -> Identifier {
    Identifier::minecraft("block/fire_1")
}
/// Vanilla's `ScreenEffectRenderer.UNDERWATER_LOCATION`.
fn underwater() -> Identifier {
    Identifier::minecraft("textures/misc/underwater.png")
}
/// Vanilla's `Hud.POWDER_SNOW_OUTLINE_LOCATION`.
fn powder_snow_outline() -> Identifier {
    Identifier::minecraft("textures/misc/powder_snow_outline.png")
}
/// Vanilla's `Hud.SPYGLASS_SCOPE_LOCATION`.
fn spyglass_scope() -> Identifier {
    Identifier::minecraft("textures/misc/spyglass_scope.png")
}
/// Vanilla's `Hud.NAUSEA_LOCATION`.
fn confusion() -> Identifier {
    Identifier::minecraft("textures/misc/nausea.png")
}
/// The nether portal's particle sprite, `Hud.extractPortalOverlay`'s
/// `getParticleMaterial(Blocks.NETHER_PORTAL)`.
fn portal() -> Identifier {
    Identifier::minecraft("block/nether_portal")
}
/// Vanilla's `ScreenEffectRenderer.submitBlockSprite` tint (`-15132391` as ARGB).
const BLOCK_OVERLAY_TINT: u32 = 0xFF19_1919;
/// Vanilla's `ScreenEffectRenderer.buildFireQuad` tint (`-436207617` as ARGB).
const FIRE_TINT: u32 = 0xE5FF_FFFF;

/// One active mob effect on us: the duration the server sent and vanilla's
/// `MobEffectInstance.BlendState`.
#[derive(Clone, Copy, Debug)]
pub struct Effect {
    /// Amplifier (level - 1).
    pub amplifier: i32,
    /// Ticks left; `-1` is infinite (`isInfiniteDuration`).
    duration: i32,
    /// `MobEffectInstance.BlendState.factor` and the one before this tick's, for the frame's
    /// `Mth.lerp(partialTicks, factorPreviousFrame, factor)`.
    factor: f32,
    prev_factor: f32,
    blend_in: i32,
    blend_out: i32,
    blend_out_advance: i32,
}

impl Effect {
    /// A new instance from `update_mob_effect`: the vanilla default blend durations are 0 and
    /// only nausea and darkness set them (`MobEffects`), which the caller resolves.
    pub fn new(
        amplifier: i32,
        duration: i32,
        blend: bool,
        blend_in: i32,
        blend_out: i32,
        blend_out_advance: i32,
    ) -> Effect {
        // A packet without the blend flag means `skipBlending`: the factor starts at its
        // target (`ClientPacketListener.handleUpdateMobEffect`).
        let instant = if blend { 0.0 } else { 1.0 };
        Effect {
            amplifier,
            duration,
            factor: instant,
            prev_factor: instant,
            blend_in,
            blend_out,
            blend_out_advance,
        }
    }

    /// Copies the previous instance's blend state (`LivingEntity.forceAddEffect` ->
    /// `newEffect.copyBlendState(previousEffect)`), so a refreshing packet does not restart the
    /// blend from scratch.
    pub fn copy_blend_from(&mut self, previous: &Effect) {
        self.factor = previous.factor;
        self.prev_factor = previous.prev_factor;
    }

    /// One client tick: `MobEffectInstance.tickClient` ticks the duration down, then the blend
    /// state eases the factor towards `!endsWithin(blendOutAdvance)`.
    pub fn tick(&mut self) {
        if self.duration > 0 {
            self.duration -= 1;
        }
        self.prev_factor = self.factor;
        let has_effect = !self.ends_within(self.blend_out_advance);
        let target = f32::from(has_effect);
        if self.factor != target {
            let blend_duration = if has_effect {
                self.blend_in
            } else {
                self.blend_out
            };
            if blend_duration == 0 {
                self.factor = target;
            } else {
                let max_delta = 1.0 / blend_duration as f32;
                self.factor += (target - self.factor).clamp(-max_delta, max_delta);
            }
        }
    }

    /// Vanilla's `MobEffectInstance.endsWithin`: an infinite duration never ends, and an ending
    /// one ends within `ticks` once its duration is at or below them.
    pub fn ends_within(&self, ticks: i32) -> bool {
        self.duration >= 0 && self.duration <= ticks
    }

    /// The blend factor between two frames (`MobEffectInstance.BlendState.getFactor`).
    pub fn blend_factor(&self, partial_tick: f32) -> f32 {
        self.prev_factor + (self.factor - self.prev_factor) * partial_tick.clamp(0.0, 1.0)
    }

    /// The frame's two endpoints of the blend (`BlendState.factorPreviousFrame`, `.factor`).
    pub fn blend_pair(&self) -> (f32, f32) {
        (self.prev_factor, self.factor)
    }
}

/// Vanilla's `LocalPlayer.portalEffectIntensity` / `oPortalEffectIntensity`, the spinning
/// effect time and speed, and `PortalProcessor.insidePortalThisTick`, ticked 20 Hz.
#[derive(Clone, Debug)]
pub struct PortalEffects {
    portal: f32,
    prev_portal: f32,
    inside_this_tick: bool,
    spinning_time: f32,
    spinning_speed: f32,
}

impl Default for PortalEffects {
    fn default() -> Self {
        PortalEffects {
            portal: 0.0,
            prev_portal: 0.0,
            inside_this_tick: false,
            spinning_time: 0.0,
            spinning_speed: 0.0,
        }
    }
}

impl PortalEffects {
    /// We stood in a nether portal block this tick (`Entity.setAsInsidePortal` through the
    /// block's `entityInside`), which is what the local transition's `insidePortalThisTick`
    /// reads. Only it matters for the confusion transition; the effect itself is drawn for any
    /// portal.
    pub fn set_inside_portal(&mut self, inside: bool) {
        self.inside_this_tick = inside;
    }

    /// One tick, vanilla's `LocalPlayer.handlePortalTransitionEffect(active)` and
    /// `tickSpinningEffect`. `active` is `getActivePortalLocalTransition() == CONFUSION`, true
    /// for a nether portal (`NetherPortalBlock.getLocalTransition`).
    pub fn tick(&mut self, active: bool, nausea: f32) {
        self.prev_portal = self.portal;
        let mut step = 0.0;
        if active && std::mem::take(&mut self.inside_this_tick) {
            step = 0.0125;
        } else if self.portal > 0.0 {
            step = -0.05;
        }
        self.portal = (self.portal + step).clamp(0.0, 1.0);
        // tickSpinningEffect: the speed is the weighted mean of the two intensities' speeds.
        // Both are clamped to [0, 1], so `<= 0` is vanilla's `!(x > 0)`.
        if self.portal <= 0.0 && nausea <= 0.0 {
            self.spinning_speed = 0.0;
        } else {
            self.spinning_speed = (self.portal * 20.0 + nausea * 7.0) / (self.portal + nausea);
            self.spinning_time += self.spinning_speed;
        }
    }

    /// `LocalPlayer.oPortalEffectIntensity` (the pair's first component).
    pub fn prev_portal(&self) -> f32 {
        self.prev_portal
    }

    /// `LocalPlayer.portalEffectIntensity` (the pair's second component).
    pub fn portal(&self) -> f32 {
        self.portal
    }

    /// `LocalPlayer.spinningEffectTime` (the accumulated spin).
    pub fn spinning_time(&self) -> f32 {
        self.spinning_time
    }

    /// `LocalPlayer.spinningEffectSpeed` (the spin's ticks per tick).
    pub fn spinning_speed(&self) -> f32 {
        self.spinning_speed
    }
}

/// The projection warp's strength and angle for one frame
/// (`GameRenderer.renderLevel`'s `spinningEffectIntensity`). The renderer applies it to the
/// level's projection matrix.
pub use haste_render::renderer::SpinWarp;

/// Where the player is standing, once per tick: the blocks the camera sees through and the
/// portal under our feet.
#[derive(Clone, Debug, Default)]
pub struct WorldProbe {
    /// The texture over the whole screen when the camera is inside a view-blocking block
    /// (the block's particle texture, `LevelExtractor.getViewBlockingState`).
    pub block_overlay: Option<Identifier>,
    /// Whether the block at our feet is a nether portal (`NetherPortalBlock.entityInside`).
    pub in_portal: bool,
    /// Whether our bounding box overlaps a fire block (`BaseFireBlock.entityInside` applies
    /// `FIRE_IGNITE`, which is what sets the local player's fire ticks on the client).
    pub in_fire: bool,
    /// Whether our bounding box overlaps powder snow (`PowderSnowBlock.entityInside` applies
    /// `FREEZE`, which is what sets the local player's freeze ticks on the client).
    pub in_powder_snow: bool,
    /// The brighter of the sky and block light at the eye position
    /// (`Level.getMaxLocalRawBrightness`, overworld: no sky darkening).
    pub eye_light: u8,
}

/// The view-blocking sample points: the eight corners of a box around the eye, as vanilla's
/// `getViewBlockingState` walks them (`0.8 * bbWidth` wide, `0.1` tall).
pub fn view_probe_points(eye: glam::DVec3, bb_width: f64, scale: f64) -> [glam::DVec3; 8] {
    std::array::from_fn(|i| {
        glam::DVec3::new(
            eye.x + ((i % 2) as f64 - 0.5) * bb_width * 0.8,
            eye.y + (((i >> 1) % 2) as f64 - 0.5) * 0.1 * scale,
            eye.z + (((i >> 2) % 2) as f64 - 0.5) * bb_width * 0.8,
        )
    })
}

/// The portal block's name: the block at our feet is a portal (`Entity.handlePortal`).
pub fn is_portal_block(block: &Identifier) -> bool {
    block.path() == "nether_portal"
}

/// The camera overlay texture of a worn item: vanilla reads the equippable component's
/// `camera_overlay` (`Hud.extractCameraOverlays`), and `Items.CARVED_PUMPKIN` is the only
/// vanilla item that sets one (`Blocks.CARVED_PUMPKIN` -> `textures/misc/pumpkinblur.png`).
/// Ids we do not know draw nothing.
pub fn camera_overlay(item: &Identifier) -> Option<Identifier> {
    (item.path() == "carved_pumpkin")
        .then(|| Identifier::minecraft("textures/misc/pumpkinblur.png"))
}

/// The spyglass item's name (`Player.isScoping`: `getUseItem().is(Items.SPYGLASS)`).
pub fn is_spyglass(item: Option<&Identifier>) -> bool {
    item.is_some_and(|i| i.path() == "spyglass")
}

/// Vanilla's `causes_suffocation` block tag, which with a full collision shape is what makes a
/// block view-blocking (`BlockBehaviour`'s `isSuffocating` default).
fn causes_suffocation() -> Identifier {
    Identifier::minecraft("causes_suffocation")
}

/// The block's particle texture (`BlockStateModelSet.getParticleMaterial`), which is what the
/// in-block overlay draws. Every block's is `textures/block/<name>.png`.
fn particle_texture(block: &Identifier) -> Identifier {
    Identifier::minecraft(&format!("block/{}", block.path()))
}

/// Whether a state blocks the view: in the `causes_suffocation` tag and with a full collision
/// shape (`BlockBehaviour.BlockStateBase.isSuffocating`).
fn is_view_blocking(
    state: haste_core::BlockStateId,
    blocks: &haste_data::BlockStates,
    shapes: &dyn haste_core::view::CollisionShapes,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> bool {
    let block = blocks.block_of(state);
    let Some(tags) = tags else { return false };
    let Some(index) = blocks.block_index(block) else {
        return false;
    };
    if !tags.contains(&causes_suffocation(), index.0) {
        return false;
    }
    // `isCollisionShapeFullBlock`: the collision shape covers the whole unit cube.
    let boxes = shapes.collision_boxes(state);
    boxes.len() == 1
        && boxes.iter().all(|b| {
            b.min.abs_diff_eq(glam::DVec3::ZERO, 1e-9)
                && b.max.abs_diff_eq(glam::DVec3::splat(1.0), 1e-9)
        })
}

/// The world the screen effects' per-tick probe reads.
pub struct Probe<'a> {
    pub world: &'a dyn haste_core::view::BlockView,
    pub shapes: &'a dyn haste_core::view::CollisionShapes,
    pub blocks: &'a haste_data::BlockStates,
    /// The block tags the server synced (`update_tags`), for `causes_suffocation`.
    pub tags: Option<&'a haste_physics::mining::BlockTags>,
    /// The interpolated eye position and the feet, for the eye's block and the portal check.
    pub eye: glam::DVec3,
    pub feet: glam::DVec3,
    /// The player's box width, height and scale: the width and scale for the view-blocking
    /// sample points, all three for the fire block overlap.
    pub bb_width: f64,
    pub bb_height: f64,
    pub scale: f64,
}

impl Probe<'_> {
    /// Vanilla's `LevelExtractor.getViewBlockingState` (the block the camera is inside),
    /// `Level.getMaxLocalRawBrightness` at the eye (the water overlay's light) and
    /// `NetherPortalBlock.entityInside` (standing in a portal), in one world read.
    pub fn sample(&self) -> WorldProbe {
        let air = haste_core::BlockStateId::AIR;
        let mut block_overlay = None;
        for p in view_probe_points(self.eye, self.bb_width, self.scale) {
            let pos = haste_core::BlockPos::new(
                p.x.floor() as i32,
                p.y.floor() as i32,
                p.z.floor() as i32,
            );
            let state = self.world.block_state(pos);
            if state == air {
                continue;
            }
            if is_view_blocking(state, self.blocks, self.shapes, self.tags) {
                block_overlay = Some(particle_texture(self.blocks.block_of(state)));
                break;
            }
        }
        // The block at the feet is a nether portal (`Entity.setAsInsidePortal` fires from the
        // portal block's `entityInside`).
        let feet_pos = haste_core::BlockPos::new(
            self.feet.x.floor() as i32,
            self.feet.y.floor() as i32,
            self.feet.z.floor() as i32,
        );
        let feet_state = self.world.block_state(feet_pos);
        let in_portal = is_portal_block(self.blocks.block_of(feet_state));
        // Whether any fire block overlaps our bounding box: the local player's client-side
        // `BaseFireBlock.entityInside` -> `FIRE_IGNITE` is what sets its own fire ticks (the
        // server does not send the owner their own `set_entity_data`).
        let mut in_fire = false;
        let mut in_powder_snow = false;
        let half = self.bb_width * 0.5;
        let min = self.feet - glam::DVec3::new(half, 0.0, half);
        let max = self.feet + glam::DVec3::new(half, self.bb_height, half);
        let (x0, x1) = (min.x.floor() as i32, (max.x - 1e-7).floor() as i32);
        let (y0, y1) = (min.y.floor() as i32, (max.y - 1e-7).floor() as i32);
        let (z0, z1) = (min.z.floor() as i32, (max.z - 1e-7).floor() as i32);
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let pos = haste_core::BlockPos::new(x, y, z);
                    let block = self.blocks.block_of(self.world.block_state(pos));
                    in_fire |= is_fire_block(block, self.blocks, self.tags);
                    in_powder_snow |= is_powder_snow_block(block, self.blocks, self.tags);
                }
            }
        }
        // The eye position's brightest light channel (`getMaxLocalRawBrightness`: the
        // overworld's sky light is not darkened by the time of day here, so the raw values).
        let eye_pos = haste_core::BlockPos::new(
            self.eye.x.floor() as i32,
            self.eye.y.floor() as i32,
            self.eye.z.floor() as i32,
        );
        let eye_light = self
            .world
            .sky_light(eye_pos)
            .max(self.world.block_light(eye_pos));
        WorldProbe {
            block_overlay,
            in_portal,
            in_fire,
            in_powder_snow,
            eye_light,
        }
    }
}

/// Whether `block` is powder snow (`PowderSnowBlock` applies the `FREEZE` inside-block effect).
fn is_powder_snow_block(
    block: &Identifier,
    blocks: &haste_data::BlockStates,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> bool {
    if block.namespace() == "minecraft" && block.path() == "powder_snow" {
        return true;
    }
    // `BlockTags.POWDER_SNOW` (`minecraft:powder_snow`) covers it and any modded snow.
    match (tags, blocks.block_index(block)) {
        (Some(tags), Some(index)) => tags.contains(&Identifier::minecraft("powder_snow"), index.0),
        _ => false,
    }
}

/// One tick of the local player's freeze ticks, as the client predicts them: powder snow's
/// `FREEZE` adds one (`InsideBlockEffectType.FREEZE`, capped at `getTicksRequiredToFreeze`),
/// fire's `CLEAR_FREEZE` zeroes them, and outside powder snow the server's rule eases them out
/// by two per tick (`LivingEntity.aiStep`). The server never sends the owner their own
/// `set_entity_data` (`ChunkMap.TrackedEntity.updatePlayer` skips self), so Haste predicts it.
pub fn tick_frozen(current: i32, in_powder_snow: bool, in_fire: bool) -> i32 {
    if in_fire {
        0
    } else if in_powder_snow {
        (current + 1).min(FREEZE_TICKS)
    } else {
        (current - 2).max(0)
    }
}

/// Whether `block` sets the player on fire when inside it: a `BaseFireBlock` (fire, soul fire)
/// or lava. Both apply the fire inside-block effect (`FIRE_IGNITE` / `LAVA_IGNITE`), which is
/// what makes the local player's `isOnFire` true on the client.
fn is_fire_block(
    block: &Identifier,
    blocks: &haste_data::BlockStates,
    tags: Option<&haste_physics::mining::BlockTags>,
) -> bool {
    if block.namespace() == "minecraft" && matches!(block.path(), "fire" | "soul_fire" | "lava") {
        return true;
    }
    match (tags, blocks.block_index(block)) {
        (Some(tags), Some(index)) => {
            tags.contains(&Identifier::minecraft("fire"), index.0)
                || tags.contains(&Identifier::minecraft("lava"), index.0)
        }
        _ => false,
    }
}

/// Vanilla's `Lightmap.getBrightness`: the light curve `v / (4 - 3v)` lifted towards 1 by the
/// dimension's ambient light (the overworld's is 0, so the curve itself).
pub fn light_brightness(level: u8, ambient_light: f32) -> f32 {
    let v = f32::from(level) / 15.0;
    lerp(ambient_light, v / (4.0 - 3.0 * v), 1.0)
}

/// Vanilla's `Entity.getPercentFrozen`.
pub fn percent_frozen(ticks_frozen: i32) -> f32 {
    (ticks_frozen.min(FREEZE_TICKS) as f32) / FREEZE_TICKS as f32
}

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

    /// `Hud.extractPortalOverlay`: the portal's particle sprite over the whole screen, with
    /// the alpha squared twice and lifted off zero (`alpha < 1` curve).
    fn portal_overlay(&self, alpha: f32) -> GuiCommand {
        let mut alpha = alpha;
        if alpha < 1.0 {
            alpha *= alpha;
            alpha *= alpha;
            alpha = alpha * 0.8 + 0.2;
        }
        full_screen_texture(
            portal(),
            self.width,
            self.height,
            white(alpha),
            GuiBlend::Normal,
        )
    }

    /// `Hud.extractConfusionOverlay`: the nausea texture zoomed about the screen centre by
    /// `lerp(strength, 2, 1)` and added to the screen in a green tint.
    fn confusion_overlay(&self, strength: f32) -> GuiCommand {
        let size = lerp(strength, 2.0, 1.0);
        let tint = argb(1.0, 0.2 * strength, 0.4 * strength, 0.2 * strength);
        // The pose scales about the centre by `size` and blits the whole texture over the
        // scaled quad, so the screen sees the middle `1 / size` of the texture: the uv window
        // is `1 / size` wide, centred (from `(1 - 1 / size) / 2`).
        let span = 1.0 / size;
        let uv = (1.0 - span) / 2.0;
        GuiCommand::Texture {
            texture: confusion(),
            x: 0,
            y: 0,
            w: self.width as i32,
            h: self.height as i32,
            u: uv,
            v: uv,
            uw: span,
            vh: span,
            tex_w: 1,
            tex_h: 1,
            tint,
            blend: GuiBlend::Additive,
        }
    }

    /// `Hud.extractSpyglassOverlay`: the scope texture over a centred square, black around it.
    fn spyglass_overlay(&self) -> [GuiCommand; 5] {
        let (w, h) = (self.width as f32, self.height as f32);
        let src = w.min(h);
        // `min(w / src, h / src)` is 1 on any window (src is the smaller side), so the ratio is
        // the scope scale itself.
        let ratio = (w / src).min(h / src) * self.scope_scale;
        let size = (src * ratio).floor() as i32;
        let left = (self.width as i32 - size) / 2;
        let top = (self.height as i32 - size) / 2;
        let scope = GuiCommand::Texture {
            texture: spyglass_scope(),
            x: left,
            y: top,
            w: size,
            h: size,
            u: 0.0,
            v: 0.0,
            uw: 1.0,
            vh: 1.0,
            tex_w: 1,
            tex_h: 1,
            tint: 0xFFFF_FFFF,
            blend: GuiBlend::Normal,
        };
        let (w, h) = (self.width as i32, self.height as i32);
        let (right, bottom) = (left + size, top + size);
        [
            scope,
            fill(0, bottom, w, h, BLACK),
            fill(0, 0, w, top, BLACK),
            fill(0, top, left, bottom, BLACK),
            fill(right, top, w, bottom, BLACK),
        ]
    }

    /// `ScreenEffectRenderer.submitFire`: two 1x1 quads at z = -0.5, translated to
    /// (±0.24, -0.3) and rotated ±10° about Y, both sampling the whole fire sprite.
    ///
    /// The level pass projects them with the HUD projection, which maps the quad at z = -0.5
    /// to the screen by a uniform `height / tan(fov / 2)` pixels per unit in x and y (a
    /// rotation about Y keeps z constant, so it only shifts and squeezes the quad in x).
    fn fire_overlay(&self) -> [GuiCommand; 2] {
        let (w, h) = (self.width as f32, self.height as f32);
        let s = h / (hud_fov(self.effects.eye_in_water).to_radians() * 0.5).tan();
        let (cx, cy) = (w / 2.0, h / 2.0);
        let turn = (10.0f32).to_radians();
        // A corner (x, y, -0.5) lands at x * cos - 0.5 * sin shifted by ±0.24 in view units,
        // its y unchanged; screen y grows downwards, so view y inverts.
        let half = 0.5;
        let z = -0.5;
        let quad = |shift: f32, turn: f32| -> GuiCommand {
            let (cos, sin) = (turn.cos(), turn.sin());
            let left = shift + half * cos + z * sin;
            let right = shift - half * cos + z * sin;
            let x0 = (cx + left.min(right) * s).round() as i32;
            let x1 = (cx + left.max(right) * s).round() as i32;
            // `buildSpriteQuad` passes (u1, v1) at the quad's bottom-left corner and (u0, v0) at
            // the top-right, so u runs high-to-low left-to-right and v low-to-high top-to-bottom
            // (the sprite is mirrored in x only, not flipped in y).
            GuiCommand::Texture {
                texture: fire_sprite(),
                x: x0,
                y: (cy - 0.2 * s).round() as i32,
                w: x1 - x0,
                h: (1.0 * s).round() as i32,
                u: 1.0,
                v: 0.0,
                uw: -1.0,
                vh: 1.0,
                tex_w: 1,
                tex_h: 1,
                tint: FIRE_TINT,
                blend: GuiBlend::Normal,
            }
        };
        // `submitFire`: the quad shifted right turns left (-10°) and the one shifted left
        // turns right (+10°), so the pair is mirrored about the screen centre.
        [quad(0.24, -turn), quad(-0.24, turn)]
    }

    /// `ScreenEffectRenderer.submitWater` over `LevelExtractor.extractPlayerState`'s overlay:
    /// the underwater texture over the whole screen, alpha 0.1 and the lightmap brightness at
    /// the eye as the colour, its uv `(u0+4, v0+4)..(u0, v0)` scrolled by the look
    /// (`uOffset = -yRot / 64`, `vOffset = xRot / 64`) and repeated (the vanilla sampler is
    /// `REPEAT`). Haste's GUI atlas is texel-fetched (no sampler), so the same REPEAT is drawn
    /// as one quad per uv period: each tile samples the whole texture once, and the tiles
    /// together cover exactly the uv window vanilla's single quad would.
    fn water_overlay(&self, out: &mut Vec<GuiCommand>) {
        let (w, h) = (self.width as f32, self.height as f32);
        // Pixels per view unit at z = -0.5: the HUD projection maps view x to `x * h / tan(fov/2)`.
        let s = h / (hud_fov(self.effects.eye_in_water).to_radians() * 0.5).tan();
        // One uv period spans half the quad's 2 view units, i.e. `s / 2` pixels.
        let tile = s * 0.5;
        let u_off = -self.look.0 / 64.0;
        let v_off = self.look.1 / 64.0;
        let brightness = light_brightness(self.effects.eye_light, 0.0);
        let tint = argb(0.1, brightness, brightness, brightness);
        // Screen pixel where the uv `base + slope * (pixel - centre) / tile` crosses an integer.
        let edges = |size: f32, centre: f32, base: f32, slope: f32, out: &mut [f32; 24]| -> usize {
            let mut n = 1;
            out[0] = 0.0;
            let near = base + slope * (0.0 - centre) / tile;
            let far = base + slope * (size - centre) / tile;
            let (lo, hi) = (near.min(far), near.max(far));
            let mut k = lo.ceil();
            while k <= hi && n < out.len() - 1 {
                let p = centre + (k - base) * tile / slope;
                if p > 0.0 && p < size {
                    out[n] = p;
                    n += 1;
                }
                k += 1.0;
            }
            out[n] = size;
            n += 1;
            out[..n].sort_by(|a, b| a.partial_cmp(b).unwrap());
            n
        };
        let mut xs = [0.0f32; 24];
        let mut ys = [0.0f32; 24];
        // u decreases as x grows (vanilla samples u1 at the left), v increases downward.
        let nx = edges(w, w * 0.5, u_off + 2.0, -1.0, &mut xs);
        let ny = edges(h, h * 0.5, v_off + 2.0, 1.0, &mut ys);
        // The texture coordinate of a boundary: `frac_hi` keeps an exact integer at the end of
        // a period (1.0) so a full-period tile runs 1..0, `frac_lo` puts the start at 0.
        let frac_lo = |v: f32| v - v.floor();
        let frac_hi = |v: f32| v - (v.ceil() - 1.0);
        let u_at = |px: f32| u_off + 2.0 - (px - w * 0.5) / tile;
        let v_at = |py: f32| v_off + 2.0 + (py - h * 0.5) / tile;
        // All tiles are the same texture and blend, so they batch into one draw call. They are
        // prepended back to front, but never overlap, so their order among themselves is free.
        for yi in (0..ny - 1).rev() {
            for xi in (0..nx - 1).rev() {
                let (x0, x1) = (xs[xi], xs[xi + 1]);
                let (y0, y1) = (ys[yi], ys[yi + 1]);
                let u_left = frac_hi(u_at(x0));
                let u_right = frac_lo(u_at(x1));
                let v_top = frac_lo(v_at(y0));
                let v_bottom = frac_hi(v_at(y1));
                out.insert(
                    0,
                    GuiCommand::Texture {
                        texture: underwater(),
                        x: x0.round() as i32,
                        y: y0.round() as i32,
                        w: (x1 - x0).round() as i32,
                        h: (y1 - y0).round() as i32,
                        u: u_left,
                        v: v_top,
                        uw: u_right - u_left,
                        vh: v_bottom - v_top,
                        tex_w: 1,
                        tex_h: 1,
                        tint,
                        blend: GuiBlend::Normal,
                    },
                );
            }
        }
    }
}

/// A rectangle in GUI pixels, filled with `tint` (vanilla's `ARGB`).
fn fill(x1: i32, y1: i32, x2: i32, y2: i32, tint: u32) -> GuiCommand {
    GuiCommand::Fill {
        x1,
        y1,
        x2,
        y2,
        color: tint,
    }
}

/// A texture over the whole screen, stretched, with `tint` (`Hud.extractTextureOverlay`).
fn full_screen_texture(
    texture: Identifier,
    width: u32,
    height: u32,
    tint: u32,
    blend: GuiBlend,
) -> GuiCommand {
    GuiCommand::Texture {
        texture,
        x: 0,
        y: 0,
        w: width as i32,
        h: height as i32,
        u: 0.0,
        v: 0.0,
        uw: 1.0,
        vh: 1.0,
        tex_w: 1,
        tex_h: 1,
        tint,
        blend,
    }
}

/// Vanilla's spyglass black (`fill(..., -16777216)`).
const BLACK: u32 = 0xFF00_0000;

impl Frame<'_> {
    /// A texture over the whole screen, stretched, with `tint` (`Hud.extractTextureOverlay`).
    fn full_screen(&self, texture: Identifier, tint: u32, blend: GuiBlend) -> GuiCommand {
        full_screen_texture(texture, self.width, self.height, tint, blend)
    }
}

/// Vanilla's `ARGB.colorFromFloat` (each channel `floor(value * 255)`), packed like vanilla.
fn argb(alpha: f32, red: f32, green: f32, blue: f32) -> u32 {
    let c = |v: f32| ((v * 255.0).floor() as u32).min(255);
    c(alpha) << 24 | c(red) << 16 | c(green) << 8 | c(blue)
}

/// Vanilla's `ARGB.white(alpha)`.
fn white(alpha: f32) -> u32 {
    argb(alpha, 1.0, 1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GUI frame size to draw into (GUI pixels at scale 4 of 1920x1080).
    const W: u32 = 480;
    const H: u32 = 270;

    fn view(e: ViewEffects) -> ViewEffects {
        e
    }

    fn frame(partial_tick: f32, effects: &ViewEffects) -> Frame<'_> {
        Frame {
            width: W,
            height: H,
            first_person: true,
            effects,
            partial_tick,
            look: (0.0, 0.0),
            scope_scale: 1.125,
        }
    }

    /// A nausea instance as the server sends it (`effect give nausea 30`: amplifier 0,
    /// duration 600 ticks, the blend flag clear, which means `skipBlending`).
    fn nausea(blend: bool, duration: i32) -> Effect {
        Effect::new(
            0,
            duration,
            blend,
            NAUSEA_BLEND_IN,
            NAUSEA_BLEND_OUT,
            NAUSEA_BLEND_OUT_ADVANCE,
        )
    }

    // ---- the nausea blend, per tick ---------------------------------------------

    #[test]
    fn nausea_skips_blending_when_the_packet_says_so() {
        // `ClientPacketListener.handleUpdateMobEffect` calls `skipBlending` when the packet
        // has no blend flag: the factor starts at its target.
        let e = nausea(false, 600);
        assert_eq!(e.blend_pair(), (1.0, 1.0));
    }

    #[test]
    fn nausea_blends_in_one_150th_per_tick() {
        let mut e = nausea(true, 600);
        e.tick();
        assert!(
            (e.blend_pair().1 - 1.0 / 150.0).abs() < 1e-6,
            "{}",
            e.blend_pair().1
        );
        // The previous tick's factor is the frame's start (`BlendState.factorPreviousFrame`).
        assert_eq!(e.blend_pair().0, 0.0);
        assert!((e.blend_factor(0.5) - 1.0 / 300.0).abs() < 1e-6);
        // 150 ticks of blending in reaches the full intensity.
        for _ in 0..149 {
            e.tick();
        }
        assert!(
            (e.blend_pair().1 - 1.0).abs() < 1e-6,
            "{}",
            e.blend_pair().1
        );
    }

    #[test]
    fn nausea_blends_out_at_one_20th_once_it_is_ending() {
        // The blend-in runs at 1/150 per tick while the effect is more than the 60-tick advance
        // from ending. From 210 ticks, 149 ticks blend the factor to 149/150 with the duration
        // still at 61 (`endsWithin(60)` false).
        let mut e = nausea(true, 60 + 150);
        for _ in 0..149 {
            e.tick();
        }
        assert!(
            (e.blend_pair().1 - 149.0 / 150.0).abs() < 1e-6,
            "{}",
            e.blend_pair().1
        );
        // The 150th tick brings the duration to 60, so the effect starts ending and the blend
        // state eases out by 1/20 per tick, not the blend-in's 1/150.
        e.tick();
        assert!(e.ends_within(60), "duration {}", e.duration);
        let before = e.blend_pair().1;
        e.tick();
        assert!(
            (e.blend_pair().1 - (before - 1.0 / 20.0)).abs() < 1e-6,
            "{} -> {}",
            before,
            e.blend_pair().1
        );
    }

    #[test]
    fn an_infinite_duration_never_blends_out() {
        let mut e = nausea(true, -1);
        for _ in 0..400 {
            e.tick();
        }
        assert!(!e.ends_within(60));
        assert!((e.blend_pair().1 - 1.0).abs() < 1e-6);
    }

    // ---- the portal and spinning effects ----------------------------------------

    #[test]
    fn standing_in_a_portal_rises_the_intensity_by_1_80_per_tick() {
        let mut p = PortalEffects::default();
        for tick in 1..=80 {
            // Standing in the portal tick after tick, as the game thread's world probe says.
            p.set_inside_portal(true);
            p.tick(true, 0.0);
            assert!(
                (p.portal() - 0.0125 * tick as f32).abs() < 1e-6,
                "tick {tick}: {}",
                p.portal()
            );
        }
        // 80 ticks of 1/80 is 1.0: the full portal intensity.
        assert!((p.portal() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn leaving_a_portal_falls_the_intensity_by_1_20_per_tick() {
        let mut p = PortalEffects::default();
        for _ in 0..80 {
            p.set_inside_portal(true);
            p.tick(true, 0.0);
        }
        p.set_inside_portal(false);
        p.tick(true, 0.0);
        assert!((p.portal() - (1.0 - 0.05)).abs() < 1e-6, "{}", p.portal());
        // The fall never goes below zero, and holds there without spinning on.
        for _ in 0..20 {
            p.tick(true, 0.0);
        }
        assert_eq!(p.portal(), 0.0);
        assert_eq!(p.spinning_speed(), 0.0);
    }

    #[test]
    fn the_spinning_speed_is_the_weighted_mean_of_the_two_intensities() {
        // Vanilla's `tickSpinningEffect`: the speed is
        // (portal * 20 + nausea * 7) / (portal + nausea), and the angle advances by it.
        let at = |portal: f32, nausea: f32| -> (f32, f32) {
            let mut p = PortalEffects {
                portal,
                prev_portal: portal,
                inside_this_tick: false,
                spinning_time: 0.0,
                spinning_speed: 0.0,
            };
            p.tick(true, nausea);
            (p.spinning_speed(), p.spinning_time())
        };
        // Portal only spins at 20, nausea only at 7, and their mean is the weighted average of
        // the intensities *after* this tick's portal step (vanilla `tickSpinningEffect` runs
        // after `handlePortalTransitionEffect`, so 0.5 falls to 0.45 first).
        assert!((at(0.5, 0.0).0 - 20.0).abs() < 1e-6);
        assert!((at(0.0, 1.0).0 - 7.0).abs() < 1e-6);
        let expected = (0.45 * 20.0 + 0.5 * 7.0) / (0.45 + 0.5);
        let (speed, time) = at(0.5, 0.5);
        assert!((speed - expected).abs() < 1e-6, "{speed}");
        assert!(
            (time - expected).abs() < 1e-6,
            "the angle advances by the speed"
        );
        // No portal and no nausea: no spin at all.
        assert_eq!(at(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn no_portal_and_no_nausea_spins_not_at_all() {
        let mut p = PortalEffects::default();
        p.tick(true, 0.0);
        assert_eq!(p.spinning_speed(), 0.0);
        assert_eq!(p.portal(), 0.0);
        assert_eq!(p.spinning_time(), 0.0);
    }

    // ---- the screen warp ---------------------------------------------------------

    #[test]
    fn the_warp_squeezes_x_by_vanillas_skew() {
        // `GameRenderer.renderLevel`: skew = (5 / (I² + 5) - 0.04 I)².
        let w = SpinWarp {
            angle_deg: 0.0,
            intensity: 1.0,
        };
        let skew = 5.0 / 6.0 - 0.04;
        assert!((w.skew() - skew * skew).abs() < 1e-6);
        // At full intensity the screen is pulled wide in x: 1/skew ≈ 1.59.
        assert!((1.0 / w.skew() - 1.59).abs() < 0.01);
        // A weak intensity barely squeezes (the 5/5 dominates).
        let weak = SpinWarp {
            angle_deg: 0.0,
            intensity: 0.01,
        };
        assert!(weak.skew() > 0.99);
    }

    #[test]
    fn the_warp_needs_portal_or_nausea() {
        let calm = view(ViewEffects::default());
        assert!(calm.spin_warp(0.5).is_none());
        let spinning = view(ViewEffects {
            nausea: (1.0, 1.0),
            spinning_time: 12.0,
            spinning_speed: 7.0,
            ..Default::default()
        });
        let w = spinning.spin_warp(0.5).expect("nausea spins");
        assert!((w.intensity - 1.0).abs() < 1e-6);
        // Half a tick of spinning at 7 per tick: the angle has advanced by 3.5.
        assert!((w.angle_deg - 15.5).abs() < 1e-6);
        // The nausea intensity itself is the pair lerped by the partial tick.
        let half = view(ViewEffects {
            nausea: (0.0, 1.0),
            ..Default::default()
        });
        assert!((half.spin_warp(0.5).unwrap().intensity - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_portal_wins_over_nausea_in_the_warp() {
        let both = view(ViewEffects {
            portal: (0.0, 0.25),
            nausea: (0.0, 1.0),
            spinning_time: 0.0,
            spinning_speed: 20.0,
            ..Default::default()
        });
        // At the end of the tick `max(portal, nausea)` is the nausea's 1.0 here.
        let w = both.spin_warp(1.0).expect("spinning");
        assert!((w.intensity - 1.0).abs() < 1e-6);
    }

    // ---- the freeze overlay ------------------------------------------------------

    #[test]
    fn freeze_ticks_are_a_fraction_of_140() {
        assert_eq!(percent_frozen(0), 0.0);
        assert!((percent_frozen(70) - 0.5).abs() < 1e-6);
        // Fully frozen (140) and past it both read 1.
        assert_eq!(percent_frozen(140), 1.0);
        assert_eq!(percent_frozen(999), 1.0);
    }

    #[test]
    fn the_freeze_ticks_rise_in_powder_snow_and_fall_outside_it() {
        // Powder snow's `FREEZE` adds one per tick, capped at `getTicksRequiredToFreeze`.
        let mut ticks = 0;
        for _ in 0..FREEZE_TICKS {
            ticks = tick_frozen(ticks, true, false);
        }
        assert_eq!(ticks, FREEZE_TICKS);
        // It never goes above the cap.
        assert_eq!(tick_frozen(FREEZE_TICKS, true, false), FREEZE_TICKS);
        // Outside, the server's rule eases them out by two per tick, never below zero.
        assert_eq!(tick_frozen(10, false, false), 8);
        assert_eq!(tick_frozen(1, false, false), 0);
        assert_eq!(tick_frozen(0, false, false), 0);
        // Fire's `CLEAR_FREEZE` zeroes them at once.
        assert_eq!(tick_frozen(120, false, true), 0);
    }

    // ---- the water overlay -------------------------------------------------------

    /// The water overlay's tile quads: `(x, y, w, h, u, v, uw, vh)`.
    type WaterTile = (i32, i32, i32, i32, f32, f32, f32, f32);

    fn water_tiles(f: &Frame) -> Vec<WaterTile> {
        let mut out = Vec::new();
        f.append_to(&mut out);
        out.iter()
            .filter_map(|c| match c {
                GuiCommand::Texture {
                    texture,
                    x,
                    y,
                    w,
                    h,
                    u,
                    v,
                    uw,
                    vh,
                    ..
                } if texture.path() == "textures/misc/underwater.png" => {
                    Some((*x, *y, *w, *h, *u, *v, *uw, *vh))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_water_overlay_is_alpha_tenth_with_the_eyes_light() {
        // `ARGB.colorFromFloat(0.1, brightness, brightness, brightness)` with the eye at
        // light 7: brightness = (7/15) / (4 - 3 * 7/15).
        let e = view(ViewEffects {
            eye_in_water: true,
            eye_light: 7,
            ..Default::default()
        });
        let brightness = light_brightness(7, 0.0);
        assert!((brightness - 0.17948).abs() < 1e-4, "{brightness}");
        let mut out = vec![GuiCommand::Text {
            x: 0,
            y: 0,
            text: "hud".into(),
            color: 0,
            shadow: false,
        }];
        frame(0.5, &e).append_to(&mut out);
        // Every tile carries the alpha-0.1 tint of the eye's light.
        let channel = (0.17948 * 255.0) as u32;
        let tint = 25 << 24 | channel << 16 | channel << 8 | channel;
        let mut seen = 0;
        for c in &out {
            if let GuiCommand::Texture {
                texture, tint: t, ..
            } = c
            {
                if texture.path() == "textures/misc/underwater.png" {
                    assert_eq!(*t, tint);
                    seen += 1;
                }
            }
        }
        assert!(seen >= 1, "the water tiles");
    }

    #[test]
    fn the_water_overlay_tiles_the_texture_across_the_screen() {
        // Vanilla draws one quad over the whole screen with a uv span of 4 and a REPEAT
        // sampler; the tiles must cover the screen with each tile a period of the texture.
        let e = view(ViewEffects {
            eye_in_water: true,
            eye_light: 15,
            ..Default::default()
        });
        let tiles = water_tiles(&frame(0.5, &e));
        assert!(tiles.len() > 1, "the texture tiles, not one stretched quad");
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (i32::MAX, 0, i32::MAX, 0);
        for &(x, y, w, h, _u, _v, uw, vh) in &tiles {
            assert!(w > 0 && h > 0, "a tile has area");
            assert!(
                (-1.0 - 1e-6..=1e-6).contains(&uw),
                "u spans a period or less"
            );
            assert!(
                (-1e-6..=1.0 + 1e-6).contains(&vh),
                "v spans a period or less"
            );
            min_x = min_x.min(x);
            max_x = max_x.max(x + w);
            min_y = min_y.min(y);
            max_y = max_y.max(y + h);
        }
        assert_eq!((min_x, min_y), (0, 0));
        assert_eq!((max_x, max_y), (W as i32, H as i32));
    }

    #[test]
    fn the_water_overlay_scrolls_by_the_look() {
        // The uv at a screen pixel is vanilla's `u = uOffset + 2 - (px - w/2) / tile` (in
        // periods), with `uOffset = -yRot / 64`; the leftmost tile samples it at `px = 0`.
        let e = view(ViewEffects {
            eye_in_water: true,
            eye_light: 15,
            ..Default::default()
        });
        let f = Frame {
            width: W,
            height: H,
            first_person: true,
            effects: &e,
            partial_tick: 0.5,
            look: (128.0, -64.0),
            scope_scale: 1.125,
        };
        let tiles = water_tiles(&f);
        let tile = H as f32 / (hud_fov(true).to_radians() * 0.5).tan() * 0.5;
        let raw = -128.0 / 64.0 + 2.0 - (0.0 - W as f32 * 0.5) / tile;
        let expected = raw - (raw.ceil() - 1.0);
        let left = tiles
            .iter()
            .find(|t| t.0 == 0)
            .expect("a tile at the left edge");
        assert!((left.4 - expected).abs() < 1e-4, "{} vs {expected}", left.4);
        // A different yaw scrolls the texture: the left edge samples a different uv.
        let mut turned = f;
        turned.look = (96.0, -64.0);
        let turned_left = water_tiles(&turned);
        let t_left = turned_left.iter().find(|t| t.0 == 0).unwrap();
        assert!(
            (t_left.4 - left.4).abs() > 1e-3,
            "the look scrolls the water"
        );
    }

    #[test]
    fn the_water_overlay_is_first_person_only() {
        let e = view(ViewEffects {
            eye_in_water: true,
            ..Default::default()
        });
        let f = Frame {
            first_person: false,
            ..frame(0.5, &e)
        };
        let mut out = Vec::new();
        f.append_to(&mut out);
        assert!(out.is_empty(), "third person draws no water overlay");
    }

    // ---- the fire overlay --------------------------------------------------------

    #[test]
    fn the_fire_overlay_is_two_quads_over_the_bottom_of_the_screen() {
        let e = view(ViewEffects {
            on_fire: true,
            ..Default::default()
        });
        let mut out = Vec::new();
        frame(0.5, &e).append_to(&mut out);
        assert_eq!(out.len(), 2, "the fire overlay is two quads");
        let scale = (H as f32) / (hud_fov(false).to_radians() * 0.5).tan();
        // The quads sit at the bottom: their top edge is 0.2 view units above the centre and
        // one view unit tall (`ScreenEffectRenderer.submitFire`'s quad is 1x1 at z = -0.5,
        // translated to y = -0.3).
        let GuiCommand::Texture {
            texture,
            x: x0,
            y: y0,
            w: w0,
            h: h0,
            u,
            v,
            uw,
            vh,
            ..
        } = &out[0]
        else {
            panic!("expected a fire quad")
        };
        // One animation frame, mirrored in x, upright in y (`buildSpriteQuad`).
        assert_eq!(texture.path(), "block/fire_1");
        assert_eq!((*u, *v, *uw, *vh), (1.0, 0.0, -1.0, 1.0));
        assert!(((H as f32 / 2.0 - 0.2 * scale) - *y0 as f32).abs() <= 1.0);
        assert!((*h0 as f32 - scale).abs() <= 1.0);
        // 1 view unit is the height / tan(fov/2): a 480x270 window at 70 degrees has its quad
        // top at 135 - 0.2 * 386 = 58 pixels, 386 tall.
        assert_eq!((*y0, *h0), (58, 386));
        // Both quads are 1 view unit wide before the rotation's squeeze, and the pair is
        // symmetric about the centre: the left quad's right edge is as far from the centre
        // as the right quad's left edge.
        let GuiCommand::Texture { x: x1, w: w1, .. } = &out[1] else {
            panic!("expected the second fire quad")
        };
        assert_eq!(*w0, *w1);
        // Symmetric about the centre (W / 2): the left quad's right edge and the right quad's
        // left edge are equidistant from it, so their sum is the width.
        assert_eq!(
            (*x0 + *w0) + *x1,
            W as i32,
            "the pair is symmetric about the centre: {x0}..{} and {x1}..{}",
            x0 + w0,
            x1 + w1
        );
    }

    #[test]
    fn the_fire_overlay_uses_the_underwater_hud_fov() {
        // Underwater the HUD field of view drops to 70 * 0.857, which makes the fire quads
        // larger on screen.
        assert!((hud_fov(true) - 60.0).abs() < 1e-3);
        assert!((hud_fov(false) - 70.0).abs() < 1e-3);
        let dry = quad_height(&view(ViewEffects {
            on_fire: true,
            ..Default::default()
        }));
        let wet = quad_height(&view(ViewEffects {
            on_fire: true,
            eye_in_water: true,
            ..Default::default()
        }));
        assert!(wet > dry);
    }

    /// The height (GUI pixels) of the first fire quad in a frame.
    fn quad_height(e: &ViewEffects) -> i32 {
        let mut out = Vec::new();
        frame(0.5, e).append_to(&mut out);
        out.iter()
            .find_map(|c| match c {
                GuiCommand::Texture { texture, h, .. } if texture.path() == "block/fire_1" => {
                    Some(*h)
                }
                _ => None,
            })
            .expect("a fire quad")
    }

    // ---- the block the camera is inside ------------------------------------------

    #[test]
    fn the_block_overlay_draws_the_blocks_particle_texture() {
        let e = view(ViewEffects {
            block_overlay: Some(Identifier::minecraft("block/dirt")),
            ..Default::default()
        });
        let mut out = Vec::new();
        frame(0.5, &e).append_to(&mut out);
        let GuiCommand::Texture { texture, tint, .. } = &out[0] else {
            panic!("expected the block overlay")
        };
        assert_eq!(texture.path(), "block/dirt");
        // `ScreenEffectRenderer.submitBlockSprite`'s `-15132391`: a 10% white.
        assert_eq!(*tint, 0xFF19_1919);
    }

    // ---- the worn item's overlay -------------------------------------------------

    #[test]
    fn only_the_carved_pumpkin_hides_the_screen() {
        assert_eq!(
            camera_overlay(&Identifier::minecraft("carved_pumpkin")),
            Some(Identifier::minecraft("textures/misc/pumpkinblur.png"))
        );
        // Every other head item (and every unknown id) draws nothing.
        assert_eq!(camera_overlay(&Identifier::minecraft("player_head")), None);
        assert_eq!(
            camera_overlay(&Identifier::minecraft("turtle_helmet")),
            None
        );
    }

    #[test]
    fn the_pumpkin_overlay_covers_the_screen_untinted() {
        let e = view(ViewEffects {
            camera_overlay: camera_overlay(&Identifier::minecraft("carved_pumpkin")),
            ..Default::default()
        });
        let mut out = Vec::new();
        frame(0.5, &e).append_to(&mut out);
        let GuiCommand::Texture { texture, tint, .. } = &out[0] else {
            panic!("expected the pumpkin overlay")
        };
        assert_eq!(texture.path(), "textures/misc/pumpkinblur.png");
        assert_eq!(*tint, 0xFFFF_FFFF);
    }

    // ---- the spyglass scope -------------------------------------------------------

    #[test]
    fn the_spyglass_scope_is_a_centred_square_with_black_around_it() {
        let e = view(ViewEffects {
            scoping: true,
            ..Default::default()
        });
        let mut out = Vec::new();
        frame(0.5, &e).append_to(&mut out);
        assert_eq!(out.len(), 5, "the scope texture plus four black fills");
        // `min(w, h) * 1.125`, centred.
        let GuiCommand::Texture { x, y, w, h, .. } = &out[0] else {
            panic!("expected the scope texture")
        };
        let size = ((W.min(H)) as f32 * 1.125).floor() as i32;
        assert_eq!((*w, *h), (size, size));
        assert_eq!((*x, *y), ((W as i32 - size) / 2, (H as i32 - size) / 2));
        for (i, fill) in out[1..].iter().enumerate() {
            let GuiCommand::Fill { color, .. } = fill else {
                panic!("expected a black fill {i}")
            };
            assert_eq!(*color, 0xFF00_0000);
        }
    }

    #[test]
    fn the_scope_grows_over_frames_while_scoping() {
        // Vanilla eases the scale towards 1.125 by half the frame's ticks and resets it to
        // 0.5 when we are not scoping.
        let mut scale = 0.5f32;
        let step = lerp(0.5 * 0.139, scale, 1.125);
        assert!(step > scale);
        assert!(step < 1.125);
        scale = step;
        assert!((scale - (0.5 + 0.625 * 0.0695)).abs() < 1e-3);
        // Many frames of scoping reach the maximum.
        for _ in 0..1000 {
            scale = lerp(0.5 * 0.139, scale, 1.125);
        }
        assert!((scale - 1.125).abs() < 1e-3);
    }

    // ---- the portal and confusion overlays ---------------------------------------

    #[test]
    fn the_portal_overlay_alpha_is_squared_twice_and_lifted() {
        let at = |intensity: f32| {
            let e = view(ViewEffects {
                portal: (0.0, intensity),
                ..Default::default()
            });
            let mut out = Vec::new();
            // Partial 1.0 reads the pair's second component, the intensity this tick.
            frame(1.0, &e).append_to(&mut out);
            let GuiCommand::Texture { tint, .. } = &out[0] else {
                panic!("expected the portal overlay")
            };
            *tint >> 24
        };
        // alpha < 1: alpha *= alpha; alpha *= alpha; alpha * 0.8 + 0.2.
        // At 0.5: 0.25 -> 0.0625 -> 0.25.
        assert_eq!(at(0.5), (0.25 * 255.0) as u32);
        // At 1.0 the curve is skipped entirely.
        assert_eq!(at(1.0), 255);
        // Even a sliver of portal draws at alpha 0.2.
        assert_eq!(at(0.01), (0.2 * 255.0) as u32);
    }

    #[test]
    fn the_confusion_overlay_zooms_and_tints_at_the_default_scale_never() {
        // Screen-effect scale 1.0 (vanilla's default) draws no confusion overlay at all; the
        // nausea shows as the projection warp instead.
        let e = view(ViewEffects {
            nausea: (1.0, 1.0),
            ..Default::default()
        });
        let mut out = Vec::new();
        frame(0.5, &e).append_to(&mut out);
        assert!(out.is_empty());

        // The overlay itself (below 100%): zoomed about the centre by `lerp(strength, 2, 1)`
        // and added to the screen in a green tint.
        let strength = 0.5;
        let size = lerp(strength, 2.0, 1.0);
        assert!((size - 1.5).abs() < 1e-6);
        let tint = argb(1.0, 0.2 * strength, 0.4 * strength, 0.2 * strength);
        assert_eq!(tint, argb(1.0, 0.1, 0.2, 0.1));
        // The zoom samples the texture's middle `1 / size` part: at strength 0 the quad is
        // twice the screen, so the uv window is half the texture, from 0.25 to 0.75.
        let size = lerp(0.0, 2.0, 1.0);
        let span = 1.0 / size;
        assert!((span - 0.5).abs() < 1e-6);
        assert!(((1.0 - span) / 2.0 - 0.25).abs() < 1e-6);
    }

    // ---- the frame's order -------------------------------------------------------

    #[test]
    fn every_overlay_draws_under_the_huds_own_commands() {
        let e = view(ViewEffects {
            block_overlay: Some(Identifier::minecraft("block/sand")),
            eye_in_water: true,
            on_fire: true,
            camera_overlay: camera_overlay(&Identifier::minecraft("carved_pumpkin")),
            frozen_ticks: 70,
            scoping: true,
            portal: (1.0, 1.0),
            ..Default::default()
        });
        let hud = GuiCommand::Text {
            x: 1,
            y: 2,
            text: "hotbar".into(),
            color: 0,
            shadow: false,
        };
        let mut out = vec![hud];
        frame(0.5, &e).append_to(&mut out);
        // Vanilla's order, first to last: the level pass's block, water and fire, then the
        // HUD's camera overlays: the spyglass scope and its black fills, the freeze outline,
        // then the portal overlay.
        let names: Vec<&str> = out
            .iter()
            .map(|c| match c {
                GuiCommand::Texture { texture, .. } => texture.path(),
                GuiCommand::Fill { .. } => "black fill",
                GuiCommand::Text { .. } => "hud",
                other => other_variant(other),
            })
            .collect();
        // The water is drawn as several consecutive tile quads.
        let water = names
            .iter()
            .filter(|n| **n == "textures/misc/underwater.png")
            .count();
        let mut expected = vec!["block/sand"];
        expected.extend(std::iter::repeat_n("textures/misc/underwater.png", water));
        expected.extend([
            "block/fire_1",
            "block/fire_1",
            "textures/misc/spyglass_scope.png",
            "black fill",
            "black fill",
            "black fill",
            "black fill",
            "textures/misc/powder_snow_outline.png",
            "block/nether_portal",
            "hud",
        ]);
        assert_eq!(names, expected);
    }

    /// Scoping hides the worn item's overlay, as vanilla's `extractCameraOverlays` picks the
    /// spyglass branch instead of the item loop.
    #[test]
    fn scoping_draws_the_scope_instead_of_the_head_item() {
        let e = view(ViewEffects {
            scoping: true,
            camera_overlay: camera_overlay(&Identifier::minecraft("carved_pumpkin")),
            ..Default::default()
        });
        let mut out = Vec::new();
        frame(0.5, &e).append_to(&mut out);
        assert!(
            !out.iter().any(|c| matches!(
                c,
                GuiCommand::Texture { texture, .. } if texture.path().contains("pumpkinblur")
            )),
            "scoping must not draw the pumpkin overlay"
        );
        assert!(out.iter().any(|c| matches!(
            c,
            GuiCommand::Texture { texture, .. } if texture.path().contains("spyglass_scope")
        )));
    }

    fn other_variant(c: &GuiCommand) -> &'static str {
        match c {
            GuiCommand::Text { .. } => "hud",
            _ => "other",
        }
    }

    #[test]
    fn a_calm_frame_appends_nothing() {
        let e = view(ViewEffects::default());
        let mut out = vec![GuiCommand::Text {
            x: 0,
            y: 0,
            text: "crosshair".into(),
            color: 0,
            shadow: false,
        }];
        frame(0.5, &e).append_to(&mut out);
        assert_eq!(out.len(), 1);
    }
}
