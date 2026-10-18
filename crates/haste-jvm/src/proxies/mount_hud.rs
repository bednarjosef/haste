//! The mount HUD state mirrored into the player proxy (moved out of `proxies.rs` to
//! keep that file within the size guard; a pure move, no behaviour change).

/// The mount HUD while riding (vanilla `Hud`'s vehicle hearts and jump bar). Set while the
/// player rides a living vehicle, cleared on foot: the hearts replace the food bar
/// (`Hud.extractVehicleHealth` over `player.getVehicle()`), and the jump bar replaces the
/// experience bar (`Hud.nextContextualInfoState` over `LocalPlayer.jumpableVehicle`) while
/// we steer a saddled horse-family vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MountHud {
    /// The vehicle's health (`LivingEntity.DATA_HEALTH_ID`, metadata index 9).
    pub health: f32,
    /// The vehicle's `Attributes.MAX_HEALTH` (the synced attribute, the family's base
    /// before a sync lands).
    pub max_health: f32,
    /// Vanilla `LocalPlayer.jumpableVehicle`: the vehicle we control can jump (the
    /// horse-family vehicle Rust steers; its jump bar shows [`Self::jump_scale`]).
    pub jumpable: bool,
    /// Vanilla `LocalPlayer.getJumpRidingScale`: the charged jump the jump bar shows.
    pub jump_scale: f32,
}
