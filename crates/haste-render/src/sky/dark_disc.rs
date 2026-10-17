// The dark disc below the horizon (vanilla `SkyRenderer.renderDarkDisc`): a black fan under
// the camera, drawn after the celestial bodies while the player's eye is below the level's
// horizon height and not under water (`SkyRenderer.shouldRenderDarkDisc`).

/// `ClientLevel.ClientLevelData.getHorizonHeight`: the world's bottom on a superflat world,
/// 63 otherwise.
pub fn horizon_height(is_flat: bool, min_y: i32) -> f64 {
    if is_flat {
        min_y as f64
    } else {
        63.0
    }
}

/// `SkyRenderer.shouldRenderDarkDisc`: the eye (interpolated player eye position) is below
/// the horizon height and the player is not under water.
pub fn should_render_dark_disc(eye_y: f64, horizon_height: f64, under_water: bool) -> bool {
    eye_y - horizon_height < 0.0 && !under_water
}

/// The bottom sky fan (`SkyRenderer.buildSkyDisc(-16)`: centre (0, -16, 0), the ring at
/// x = -512 cos a, z = 512 sin a for a = -180..=180 step 45) lifted by the 12 blocks
/// `renderDarkDisc` translates it, as eight triangles in the camera's rotation frame.
pub(super) fn dark_disc_positions() -> [[f32; 3]; 24] {
    let y = -16.0 + 12.0;
    let ring = |i: i32| {
        let a = ((-180 + 45 * i) as f32).to_radians();
        [-512.0 * a.cos(), y, 512.0 * a.sin()]
    };
    let mut out = [[0.0; 3]; 24];
    for i in 0..8 {
        out[i as usize * 3] = [0.0, y, 0.0];
        out[i as usize * 3 + 1] = ring(i);
        out[i as usize * 3 + 2] = ring(i + 1);
    }
    out
}
