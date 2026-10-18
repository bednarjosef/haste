//! Spectator mode (`GameType.SPECTATOR`): the flight rules that differ from creative.
//! Vanilla `GameType.updatePlayerAbilities` forces flight on when the mode becomes spectator
//! and off for survival/adventure (`MultiPlayerGameMode.setLocalMode` runs it on every
//! game_event mode change and at respawn), and `LocalPlayer.aiStep` re-forces it every tick
//! while `mayfly` holds, so a server `player_abilities` cannot leave a spectator falling.

use super::LocalPlayer;
use haste_core::game::GameMode;

/// Vanilla `GameType.updatePlayerAbilities`' `flying` field: spectator forces flight on,
/// survival and adventure off, creative keeps whatever it was.
pub fn flying_after_mode_change(mode: GameMode, flying: bool) -> bool {
    match mode {
        GameMode::Spectator => true,
        GameMode::Creative => flying,
        _ => false,
    }
}

/// Vanilla `LocalPlayer.aiStep`'s spectator branch of the creative-flight block: with
/// `mayfly`, a spectator that is somehow not flying starts flying again this tick. The caller
/// treats the change as a creative-flight toggle (`justToggledCreativeFlight`: no glide start,
/// and the `player_abilities` packet leaves with the tick like every flight change).
pub fn ai_step_forces_flight(may_fly: bool, spectator: bool, flying: bool) -> bool {
    may_fly && spectator && !flying
}

/// The flying-speed step one wheel notch adds (`MouseHandler.onScroll`).
pub const FLYING_SPEED_SCROLL_STEP: f32 = 0.005;
/// The flying speed's clamp (`MouseHandler.onScroll`: `Mth.clamp(speed, 0.0F, 0.2F)`).
pub const FLYING_SPEED_MAX: f32 = 0.2;

impl LocalPlayer {
    /// Vanilla `MouseHandler.onScroll`'s spectator branch: with the spectator menu closed (it
    /// takes the wheel when open), the wheel steps the flying speed instead of the hotbar
    /// slot — `0.005` per notch, clamped to `0..0.2`. The speed is purely client-side: no
    /// packet tells the server.
    pub fn spectator_scroll_fly_speed(&mut self, wheel: f32) {
        self.attrs.flying_speed = (self.attrs.flying_speed + wheel * FLYING_SPEED_SCROLL_STEP)
            .clamp(0.0, FLYING_SPEED_MAX);
        self.abilities.flying_speed = self.attrs.flying_speed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::LocalPlayer;
    use glam::DVec3;
    use haste_core::input::InputState;
    use haste_core::EntityId;
    use haste_physics::testing::{states, TestShapes, TestWorld};
    use haste_protocol::packets::play::clientbound as cb;
    use haste_protocol::{ClientboundPlay, ServerboundPlay};

    const ME: EntityId = EntityId(7);

    fn ids() -> crate::player::AttributeIds {
        crate::player::AttributeIds {
            movement_speed: 26,
            jump_strength: 19,
            gravity: 18,
            step_height: 33,
            scale: 30,
            sneaking_speed: 31,
            block_break_speed: 7,
            mining_efficiency: 24,
            submerged_mining_speed: 34,
            block_interaction_range: 8,
            entity_interaction_range: 13,
            attack_speed: 3,
        }
    }

    /// A player in `mode` standing on the flat stone floor, after its spawn accept.
    fn spawned(mode: GameMode) -> LocalPlayer {
        let mut p = LocalPlayer::with_entity(ME, mode);
        p.set_attribute_ids(ids());
        let mut out = Vec::new();
        assert!(p.handle(
            &ClientboundPlay::PlayerPosition(cb::PlayerPosition {
                teleport_id: 1,
                pos: DVec3::new(0.5, 0.0, 0.5),
                velocity: DVec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
            }),
            &mut out
        ));
        let world = TestWorld::flat(0, states::STONE);
        for _ in 0..2 {
            p.tick_with(
                &InputState::default(),
                &world,
                &TestShapes::default(),
                &TestShapes::default(),
                None,
                &mut out,
            );
        }
        p
    }

    fn tick(p: &mut LocalPlayer, input: &InputState) -> Vec<ServerboundPlay> {
        let mut out = Vec::new();
        let world = TestWorld::flat(0, states::STONE);
        p.tick_with(
            input,
            &world,
            &TestShapes::default(),
            &TestShapes::default(),
            None,
            &mut out,
        );
        out
    }

    fn game_event(mode_id: f32) -> ClientboundPlay {
        ClientboundPlay::GameEvent(cb::GameEvent {
            event: 3,
            value: mode_id,
        })
    }

    fn jump() -> InputState {
        InputState {
            jump: true,
            ..InputState::default()
        }
    }

    #[test]
    fn mode_change_forces_flight_like_update_player_abilities() {
        // Spectator: flying on, whatever it was.
        assert!(flying_after_mode_change(GameMode::Spectator, false));
        assert!(flying_after_mode_change(GameMode::Spectator, true));
        // Creative: kept.
        assert!(!flying_after_mode_change(GameMode::Creative, false));
        assert!(flying_after_mode_change(GameMode::Creative, true));
        // Survival and adventure: off.
        assert!(!flying_after_mode_change(GameMode::Survival, true));
        assert!(!flying_after_mode_change(GameMode::Adventure, true));
    }

    #[test]
    fn ai_step_only_forces_flight_for_a_flying_capable_spectator() {
        assert!(ai_step_forces_flight(true, true, false));
        assert!(!ai_step_forces_flight(true, true, true));
        assert!(!ai_step_forces_flight(false, true, false));
        assert!(!ai_step_forces_flight(true, false, false));
    }

    #[test]
    fn game_event_to_spectator_starts_flight_and_back_to_survival_stops_it() {
        let mut p = spawned(GameMode::Survival);
        assert!(!p.state.flying);
        let mut out = Vec::new();
        p.handle(&game_event(3.0), &mut out);
        assert!(p.state.flying, "spectator forces flight on");
        // `MultiPlayerGameMode.setLocalMode` sends nothing (the server owns the change), so
        // the tick that follows does not either.
        let out = tick(&mut p, &InputState::default());
        assert!(
            !out.iter()
                .any(|o| matches!(o, ServerboundPlay::PlayerAbilities(_))),
            "no abilities packet for the mode change itself"
        );
        // Back to survival: flight goes off again.
        let mut out = Vec::new();
        p.handle(&game_event(0.0), &mut out);
        assert!(!p.state.flying);
    }

    #[test]
    fn a_double_tap_of_jump_in_spectator_does_not_toggle_flight_off() {
        let mut p = spawned(GameMode::Spectator);
        let mut out = Vec::new();
        p.handle(&game_event(3.0), &mut out);
        assert!(p.state.flying);
        // A double press of jump, the toggle that stops flight in creative
        // (`LocalPlayer.aiStep`'s spectator branch skips the toggle entirely).
        tick(&mut p, &jump());
        tick(&mut p, &InputState::default());
        let out = tick(&mut p, &jump());
        assert!(
            p.state.flying,
            "the jump key never ends flight in spectator"
        );
        assert!(
            !out.iter()
                .any(|o| matches!(o, ServerboundPlay::PlayerAbilities(_))),
            "no flight-change packet: flight was never toggled"
        );
    }

    #[test]
    fn the_scroll_wheel_steps_the_flying_speed_within_vanillas_clamp() {
        let mut p = spawned(GameMode::Spectator);
        assert_eq!(p.attrs.flying_speed, 0.05);
        p.spectator_scroll_fly_speed(1.0);
        assert_eq!(p.attrs.flying_speed, 0.055);
        p.spectator_scroll_fly_speed(-2.0);
        assert_eq!(p.attrs.flying_speed, 0.045);
        // Clamped at both ends, never negative and never past 0.2.
        for _ in 0..40 {
            p.spectator_scroll_fly_speed(1.0);
        }
        assert_eq!(p.attrs.flying_speed, 0.2);
        for _ in 0..40 {
            p.spectator_scroll_fly_speed(-1.0);
        }
        // f32 accumulation leaves a positive epsilon steps above zero (vanilla's too).
        assert!(p.attrs.flying_speed < 1e-6 && p.attrs.flying_speed >= 0.0);
        // The abilities mirror follows, so the FOV modifier path sees the same value.
        assert_eq!(p.abilities.flying_speed, p.attrs.flying_speed);
    }

    #[test]
    fn a_server_abilities_packet_that_stops_flight_is_overruled_next_tick() {
        let mut p = spawned(GameMode::Spectator);
        let mut out = Vec::new();
        p.handle(&game_event(3.0), &mut out);
        tick(&mut p, &InputState::default());
        // The server turns flight off in `player_abilities`.
        p.handle(
            &ClientboundPlay::PlayerAbilities(cb::PlayerAbilities {
                flags: crate::player::ability_flags::MAY_FLY,
                flying_speed: 0.05,
                fov_modifier: 0.1,
            }),
            &mut out,
        );
        assert!(!p.state.flying);
        let out = tick(&mut p, &InputState::default());
        assert!(p.state.flying, "aiStep re-forces flight for a spectator");
        assert!(out.iter().any(|o| matches!(
            o,
            ServerboundPlay::PlayerAbilities(a) if a.flags & crate::player::ability_flags::FLYING != 0
        )));
    }
}
