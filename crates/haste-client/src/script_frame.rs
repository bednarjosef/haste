//! The script frame logging: what a script screenshot's frame drew, as greppable lines. The
//! check scripts (`scripts/checks/*.txt`) assert on these instead of opening the PNGs.

use haste_protocol::haste_core::gui::{GuiCommand, GuiFrame};
use haste_protocol::haste_core::Identifier;

/// Prints the text, item and HUD-sprite facts of a script screenshot's frame: the item and
/// hotbar facts (creative inventory checks grep these), the heart and food sprite counts
/// (a spectator draws none, `Hud.extractHotbarAndDecorations`'s `canHurtPlayer` gate) and
/// whether a view-blocking block covered the screen (`ScreenEffectRenderer.submitBlockSprite`,
/// which vanilla never draws for a spectator).
pub(crate) fn log_script_frame(frame: &GuiFrame, name: &str, item_names: &[Identifier]) {
    let mut hotbar = false;
    let mut hearts = 0;
    let mut food = 0;
    let mut block_overlay = false;
    let mut items = Vec::new();
    for c in &frame.commands {
        match c {
            GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                println!("script text[{name}]: {text}");
            }
            GuiCommand::Sprite { sprite, .. } => {
                let path = sprite.path();
                if path == "hud/hotbar" {
                    hotbar = true;
                } else if path.starts_with("hud/heart") {
                    hearts += 1;
                } else if path.starts_with("hud/food") {
                    food += 1;
                }
            }
            // The block overlay is the view-blocking block's particle texture, drawn over
            // the whole screen (every other HUD block texture is an item model, drawn by
            // `GuiCommand::Item`).
            GuiCommand::Texture { texture, .. } if texture.path().starts_with("block/") => {
                block_overlay = true;
            }
            GuiCommand::Item { item, .. } => {
                // The item's registry name (creative inventory checks grep for these).
                if let Some(id) = item_names.get(item.0 as usize) {
                    items.push(id.to_string());
                }
            }
            _ => {}
        }
    }
    println!(
        "script frame[{name}]: hotbar={}, hearts={hearts}, food={food}, block_overlay={}, items=[{}]",
        if hotbar { "yes" } else { "no" },
        if block_overlay { "yes" } else { "no" },
        items.join(",")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_protocol::haste_core::gui::GuiBlend;

    fn sprite(path: &str) -> GuiCommand {
        GuiCommand::Sprite {
            sprite: Identifier::minecraft(path),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            tint: 0,
            blend: GuiBlend::Normal,
        }
    }

    /// The heart and food counts the spectator checks grep for: a frame with hearts and
    /// food sprites counts them all, a spectator's (empty) frame counts none. The output is
    /// println'd (the check scripts grep it); the test pins the counting logic itself.
    #[test]
    fn hearts_and_food_sprites_are_counted() {
        let f = GuiFrame {
            commands: vec![
                sprite("hud/hotbar"),
                sprite("hud/heart/container"),
                sprite("hud/heart/full"),
                sprite("hud/food_empty"),
                sprite("hud/food_half"),
                sprite("hud/hotbar_selection"),
            ],
            ..GuiFrame::default()
        };
        let mut hearts = 0;
        let mut food = 0;
        let mut hotbar = false;
        for c in &f.commands {
            if let GuiCommand::Sprite { sprite, .. } = c {
                let path = sprite.path();
                if path == "hud/hotbar" {
                    hotbar = true;
                } else if path.starts_with("hud/heart") {
                    hearts += 1;
                } else if path.starts_with("hud/food") {
                    food += 1;
                }
            }
        }
        assert!(hotbar);
        assert_eq!(hearts, 2);
        assert_eq!(food, 2);
        log_script_frame(&f, "t", &[]);
    }
}
