//! Tests for the HUD warm-up. The JVM ones need the cache and share the process's JVM and
//! game state with `ui/tests.rs`'s frame tests (the `frame_tests` lock serialises them).

use super::*;
use crate::ui::tests::{frame_tests, host};
use haste_protocol::haste_core::game::GameMode;
use haste_protocol::haste_core::gui::GuiCommand;

/// The warm-up finishes (quiet, or at its hard cap) and leaves the UI rendering the HUD:
/// its frames are the real ones, so going live afterwards draws the same HUD.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn warmup_finishes_and_the_hud_still_renders() {
    let _serial = frame_tests().lock().unwrap();
    let ui = Ui::new(host());
    ui.gui().with_state(|s| {
        s.in_world = true;
        s.game_mode = GameMode::Creative;
    });
    let mut warmup = HudWarmup::new(ui);
    // Stepped like the game thread's ticks (12 ms budget each); it must go warm well
    // before the step count could reach the hard cap's frames.
    let mut steps = 0;
    while !warmup.step(Duration::from_millis(12)) {
        steps += 1;
        assert!(steps < 500, "warm-up did not finish in {steps} ticks");
    }
    let mut ui = warmup.into_ui();
    let frame = ui.frame(320, 240, 2, (0.0, 0.0), 0.0);
    let sprites: Vec<String> = frame
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                Some(sprite.to_string())
            }
            _ => None,
        })
        .collect();
    assert!(
        sprites.iter().any(|s| s.starts_with("minecraft:hud/")),
        "no HUD sprites after the warm-up: {sprites:?}"
    );
}
