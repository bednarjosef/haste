//! The camera's 20 Hz tick (vanilla's `Camera.tick` with `LivingEntity.tick`'s hurt and death
//! counters): moved out of `net.rs` as is.

use haste_data::GameData;

use super::{fov_modifier_target, Session, UseItemKind};

impl Session {
    /// One 20 Hz tick of the camera effects (vanilla's `Camera.tick`, plus `LivingEntity.tick` /
    /// `tickDeath`'s hurt and death counters): the FOV modifier target and its smoothing, and
    /// the hurt and death counters. The walk distance and bob for the view bobbing are the
    /// player hand's own (vanilla's one `ClientAvatarState` serves the hand and the camera
    /// alike). Called by the game thread after `end_tick` (the headless path has no camera and
    /// skips it).
    pub fn tick_camera(&mut self, data: Option<&GameData>, first_person: bool) {
        let p = &self.player;
        // The drawn item: the held stack while the use button is down. A change of the held
        // stack restarts the draw (vanilla stops the use when the item in hand changes).
        let held = p.int.input.use_item.then(|| p.main_hand().item).flatten();
        if held != self.use_item {
            self.use_ticks = 0;
        }
        self.use_item = held;
        if held.is_some() {
            self.use_ticks += 1;
        }
        let using = if self.use_ticks > 0 {
            held.and_then(|h| UseItemKind::classify(data, h))
        } else {
            None
        };
        let sprinting = if p.state.sprinting {
            // Vanilla adds the hardcoded "sprinting" attribute modifier (x1.3,
            // ADD_MULTIPLIED_TOTAL) while the player sprints and `getFieldOfViewModifier`
            // reads the attribute with it; haste's physics applies sprinting itself and
            // strips that modifier from its own copy of the attribute.
            1.3
        } else {
            1.0
        };
        let movement_speed = p.attrs.movement_speed * sprinting;
        // `Camera.tickFov`: only a player camera entity has a modifier; any other eases to 1.
        // (A spectated remote player's own modifier is not modelled: it eases to 1 as well.)
        let target = if self.camera.snapshot().is_some() {
            1.0
        } else {
            fov_modifier_target(
                p.state.flying,
                movement_speed,
                p.abilities.fov_modifier,
                using,
                self.use_ticks,
                first_person,
                self.fov_effect_scale,
            )
        };
        self.old_fov_modifier = self.fov_modifier;
        self.fov_modifier =
            (self.fov_modifier + (target - self.fov_modifier) * 0.5).clamp(0.1, 1.5);

        // Hurt and death counters (LivingEntity.tick / tickDeath).
        self.hurt_ticks = self.hurt_ticks.saturating_sub(1);
        // The damage cooldown the HUD heart blink reads (`LivingEntity.tick`).
        self.damage_cooldown = self.damage_cooldown.saturating_sub(1);
        if p.health <= 0.0 {
            self.death_ticks = self.death_ticks.saturating_add(1);
        } else {
            self.death_ticks = 0;
        }
    }

    /// The camera's eye position and look at `partial` (the player's, or the spectated
    /// entity's: [`crate::game::camera_entity::camera_pose`]).
    pub fn camera_pose(&self, partial: f32) -> (glam::DVec3, f32, f32) {
        crate::game::camera_entity::camera_pose(&self.camera, &self.player, partial)
    }
}
