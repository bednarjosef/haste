//! The nether portal's screen effect: `LocalPlayer`'s portal intensity and spinning
//! effect, and `Hud.extractPortalOverlay`.
use super::draw::{full_screen_texture, white};
use super::Frame;
use haste_protocol::haste_core::gui::{GuiBlend, GuiCommand};
use haste_protocol::haste_core::Identifier;
/// The nether portal's particle sprite, `Hud.extractPortalOverlay`'s
/// `getParticleMaterial(Blocks.NETHER_PORTAL)`.
fn portal() -> Identifier {
    Identifier::minecraft("block/nether_portal")
}

/// Vanilla's `LocalPlayer.portalEffectIntensity` / `oPortalEffectIntensity`, the spinning
/// effect time and speed, and `PortalProcessor.insidePortalThisTick`, ticked 20 Hz.
#[derive(Clone, Debug)]
pub struct PortalEffects {
    pub(super) portal: f32,
    pub(super) prev_portal: f32,
    pub(super) inside_this_tick: bool,
    pub(super) spinning_time: f32,
    pub(super) spinning_speed: f32,
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

/// The portal block's name: the block at our feet is a portal (`Entity.handlePortal`).
pub fn is_portal_block(block: &Identifier) -> bool {
    block.path() == "nether_portal"
}

impl Frame<'_> {
    /// `Hud.extractPortalOverlay`: the portal's particle sprite over the whole screen, with
    /// the alpha squared twice and lifted off zero (`alpha < 1` curve).
    pub(super) fn portal_overlay(&self, alpha: f32) -> GuiCommand {
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
}
