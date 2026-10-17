//! Menu mode's frame (no world joined): the open screen's GUI over the background the
//! renderer draws without a world.
use glam::DVec3;
use haste_protocol::haste_core::gui::GuiFrame;
use haste_render::{Camera, Renderer, SkyState};
use haste_setup::options::Options;

use super::{gui_scaled_size, resolve_gui_scale, InputRouter};

/// The GUI frame of menu mode: the open screen over the plain background the renderer draws
/// without a world. `mouse` is the cursor in GUI pixels; the camera looks straight ahead.
pub(super) fn menu_frame(
    ui: &mut crate::ui::Ui,
    renderer: &mut Renderer,
    input: &InputRouter,
    options: &Options,
) -> (GuiFrame, Camera, SkyState, f32) {
    let (w, h) = renderer.size();
    let scale = resolve_gui_scale(u32::from(options.gui_scale), w, h);
    let (gw, gh) = gui_scaled_size(w, h, scale);
    let gui = ui.frame(gw, gh, scale, input.mouse, 0.0).clone();
    (gui, menu_camera(), menu_sky(), 0.0)
}

/// Menu mode's camera: at the origin looking north (the world is not drawn).
pub(super) fn menu_camera() -> Camera {
    Camera {
        pos: DVec3::ZERO,
        yaw: 180.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    }
}

/// Menu mode's sky: noon, no fog (a plain bright background behind the screen).
pub(super) fn menu_sky() -> SkyState {
    SkyState {
        time_of_day: 6000,
        rain: 0.0,
        dimension_has_sky: true,
        fog_start: 1000.0,
        fog_end: 1000.0,
        ..Default::default()
    }
}
