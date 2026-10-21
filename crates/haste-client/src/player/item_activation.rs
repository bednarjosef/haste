//! The item the totem of undying (or any stack the game shows as "activated") holds up in front
//! of the local player (vanilla `net.minecraft.client.player.ItemActivation`): a copy of the
//! stack, a 40-tick countdown and two random offsets that place it on screen. It is set by
//! `LocalPlayer.displayItemActivation` and ticked by `GameRenderer.tick`, and the screen-effect
//! renderer reads it back (`ScreenEffectRenderer.renderItemActivationAnimation`).

use haste_audio::random::LegacyRandom;
use haste_core::entity::ItemStackData;

/// Vanilla `ItemActivation.ANIMATION_LENGTH`: ticks the item stays up.
pub const ANIMATION_LENGTH: i32 = 40;

/// Vanilla `ItemActivation`: the activated item and its countdown.
#[derive(Clone, Debug, Default)]
pub struct ItemActivation {
    /// The item shown (`ItemActivation.item`); cleared when the countdown ends.
    item: Option<ItemStackData>,
    /// Ticks left (`ItemActivation.ticks`).
    ticks: i32,
    /// Random offsets in -1..1 (`ItemActivation.offX`/`offY`), fixed for the animation.
    off_x: f32,
    off_y: f32,
}

impl ItemActivation {
    /// Vanilla `ItemActivation.tick`: counts down, and drops the item when the count reaches 0.
    pub fn tick(&mut self) {
        if self.ticks > 0 {
            self.ticks -= 1;
            if self.ticks == 0 {
                self.item = None;
            }
        }
    }

    /// Vanilla `ItemActivation.reset`: no item, no countdown.
    pub fn reset(&mut self) {
        self.item = None;
        self.ticks = 0;
    }

    /// Vanilla `ItemActivation.activate`: shows a copy of `stack` for `ANIMATION_LENGTH` ticks,
    /// with two random offsets drawn from `random` (x first, then y, as vanilla).
    pub fn activate(&mut self, stack: &ItemStackData, random: &mut LegacyRandom) {
        self.item = Some(stack.clone());
        self.ticks = ANIMATION_LENGTH;
        self.off_x = random.next_float() * 2.0 - 1.0;
        self.off_y = random.next_float() * 2.0 - 1.0;
    }

    /// Vanilla `ItemActivation.isActive`: an item is set and its countdown runs.
    pub fn is_active(&self) -> bool {
        self.item.is_some() && self.ticks > 0
    }

    /// The activated item (`ItemActivation.item`).
    pub fn item(&self) -> Option<&ItemStackData> {
        self.item.as_ref()
    }

    /// Ticks left (`ItemActivation.ticks`).
    pub fn ticks(&self) -> i32 {
        self.ticks
    }

    /// The random x offset (`ItemActivation.offX`).
    pub fn off_x(&self) -> f32 {
        self.off_x
    }

    /// The random y offset (`ItemActivation.offY`).
    pub fn off_y(&self) -> f32 {
        self.off_y
    }

    /// What the renderer draws this tick (vanilla `LevelExtractor` fills
    /// `PlayerRenderState.itemActivation` only while `isActive`); `None` when idle.
    pub fn render_state(&self) -> Option<haste_render::entity::ItemActivation> {
        let item = self.item.as_ref().filter(|_| self.is_active())?.item?;
        Some(haste_render::entity::ItemActivation {
            item,
            ticks: self.ticks,
            off_x: self.off_x,
            off_y: self.off_y,
        })
    }
}

#[cfg(test)]
mod tests;
