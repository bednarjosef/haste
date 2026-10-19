//! Vanilla `SpectatorGui`: the menu's open/closed state and fade timing, its input entry
//! points and its HUD drawing (the hotbar sprite with the menu's slots, the selection
//! highlight, the key labels, the item icons and the action line above the bar).

use std::borrow::Cow;
use std::time::{Duration, Instant};

use haste_core::gui::{Argb, GuiBlend, GuiCommand};
use haste_core::Identifier;
use uuid::Uuid;

use super::menu::{MenuEffects, MenuItem, SpectatorMenu};
use super::{DrawContext, TabList, Teams};

/// The bar stays fully visible this long after the last selection
/// (`SpectatorGui.FADE_OUT_DELAY`).
const FADE_OUT_DELAY: Duration = Duration::from_millis(5000);
/// ... and then fades over this long (`FADE_OUT_TIME`).
const FADE_OUT_TIME: Duration = Duration::from_millis(2000);

/// `SpectatorGui`: the menu (open while `Some`) and when it was last selected.
pub struct SpectatorGui {
    menu: Option<SpectatorMenu>,
    /// `lastSelectionTime`; `Instant` deltas are what the alpha reads.
    last_selection: Instant,
    /// Bumped on every menu change, so the action line's text is rebuilt only then.
    revision: u64,
}

impl Default for SpectatorGui {
    fn default() -> Self {
        SpectatorGui {
            menu: None,
            last_selection: Instant::now(),
            revision: 0,
        }
    }
}

impl SpectatorGui {
    pub fn new() -> Self {
        Self::default()
    }

    /// `isMenuActive`.
    pub fn menu_active(&self) -> bool {
        self.menu.is_some()
    }

    /// The menu's revision (bumped on every change; the action text caches against it).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The action line's text (see [`SpectatorMenu::action_text`]); `None` with no menu.
    pub fn action_text(&self) -> Option<Cow<'_, str>> {
        self.menu.as_ref().map(|m| m.action_text())
    }

    /// The open menu's selected slot, for the tests.
    pub(super) fn menu_selected_slot(&self) -> i32 {
        self.menu.as_ref().map_or(-1, |m| m.selected_slot())
    }

    /// Whether the open menu is on the root category, for the tests.
    pub(super) fn menu_is_root(&self) -> bool {
        self.menu.as_ref().is_some_and(|m| m.is_root())
    }

    /// `onHotbarSelected` (vanilla `Minecraft.handleKeybinds`' hotbar loop, spectator
    /// branch): refreshes the fade timer and selects a slot, opening the menu if closed.
    pub fn on_hotbar_selected(
        &mut self,
        slot: i32,
        tab: &TabList,
        teams: &Teams,
        out: &mut MenuEffects,
    ) {
        self.last_selection = Instant::now();
        match &mut self.menu {
            Some(menu) => menu.select_slot(slot, tab, out),
            None => self.menu = Some(SpectatorMenu::new(tab, teams)),
        }
        self.changed(out);
    }

    /// `onMouseScrolled` (the wheel while the menu is open): steps the selection over
    /// empty and disabled slots, one slot per notch.
    pub fn on_mouse_scrolled(&mut self, wheel: i32, tab: &TabList, out: &mut MenuEffects) {
        let Some(menu) = &mut self.menu else {
            return;
        };
        let mut new_slot = menu.selected_slot() + wheel;
        while (0..=8).contains(&new_slot) && !menu.item(new_slot).enabled(tab) {
            new_slot += wheel;
        }
        if (0..=8).contains(&new_slot) {
            menu.select_slot(new_slot, tab, out);
            self.last_selection = Instant::now();
        }
        self.changed(out);
    }

    /// `onHotbarActionKeyPressed` (the key bound to `keySpectatorHotbar`: middle click):
    /// opens the menu, or activates the selected slot of an open one.
    pub fn on_hotbar_action_key(&mut self, tab: &TabList, teams: &Teams, out: &mut MenuEffects) {
        self.last_selection = Instant::now();
        match &mut self.menu {
            Some(menu) => {
                let slot = menu.selected_slot();
                if slot != -1 {
                    menu.select_slot(slot, tab, out);
                }
            }
            None => self.menu = Some(SpectatorMenu::new(tab, teams)),
        }
        self.changed(out);
    }

    /// `onSpectatorMenuClosed` (`SpectatorMenu.exit`): the menu is gone and the bar's
    /// alpha is zero, so nothing draws.
    pub fn close(&mut self) {
        self.menu = None;
        self.last_selection = Instant::now() - FADE_OUT_DELAY - FADE_OUT_TIME;
        self.revision += 1;
    }

    /// Ages the last selection by `t` (the tests' clock).
    pub(super) fn age_selection(&mut self, t: Duration) {
        self.last_selection -= t;
    }

    /// Applies what an activation did: the close item ends the menu.
    fn changed(&mut self, out: &MenuEffects) {
        if out.closed {
            self.close();
        } else {
            self.revision += 1;
        }
    }

    /// `getHotbarAlpha`: one for `FADE_OUT_DELAY` after the last selection, then fading to
    /// zero over `FADE_OUT_TIME`.
    fn hotbar_alpha(&self) -> f32 {
        let since = self.last_selection.elapsed();
        let remaining = FADE_OUT_DELAY.saturating_sub(since);
        let alpha = remaining.as_secs_f32() / FADE_OUT_TIME.as_secs_f32();
        alpha.clamp(0.0, 1.0)
    }

    /// `extractHotbar` + `extractAction`: draws the menu into the frame the vanilla `Hud`
    /// filled (it draws the item hotbar only outside spectator mode, so the bar's place is
    /// free). The fade-out's end closes the menu, like vanilla's `menu.exit()` there.
    pub fn extract(
        &mut self,
        out: &mut Vec<GuiCommand>,
        width: i32,
        height: i32,
        ctx: &DrawContext<'_>,
    ) {
        if self.menu.is_none() {
            return;
        }
        let alpha = self.hotbar_alpha();
        if alpha <= 0.0 {
            self.close();
            return;
        }
        let menu = self.menu.as_ref().expect("checked above");
        // `extractPage`: the hotbar sprite rises from the bottom edge as it fades.
        let center = width / 2;
        let y = (height as f32 - 22.0 * alpha).floor() as i32;
        let color = white(alpha);
        out.push(GuiCommand::Sprite {
            sprite: Identifier::minecraft("hud/hotbar"),
            x: center - 91,
            y,
            w: 182,
            h: 22,
            tint: color,
            blend: GuiBlend::Normal,
        });
        if menu.selected_slot() >= 0 {
            out.push(GuiCommand::Sprite {
                sprite: Identifier::minecraft("hud/hotbar_selection"),
                x: center - 91 - 1 + menu.selected_slot() * 20,
                y: y - 1,
                w: 24,
                h: 23,
                tint: color,
                blend: GuiBlend::Normal,
            });
        }
        for slot in 0..9 {
            Self::extract_slot(
                out,
                slot,
                width / 2 - 90 + slot * 20 + 2,
                y + 3,
                alpha,
                menu.item(slot),
                ctx,
            );
        }
        // `extractAction`: the selected item's name (or the category's prompt) above the bar.
        if let Some(metrics) = ctx.metrics {
            let text_width = metrics.width(ctx.action);
            out.push(GuiCommand::Text {
                x: (width - text_width) / 2,
                y: height - 35,
                text: ctx.action.to_owned(),
                color: white(alpha),
                shadow: true,
            });
        }
    }

    /// `extractSlot`: the item's icon at the slot, dimmed when disabled, and the key label
    /// of the bound hotbar key on enabled items.
    fn extract_slot(
        out: &mut Vec<GuiCommand>,
        slot: i32,
        x: i32,
        y: i32,
        alpha: f32,
        item: MenuItem<'_>,
        ctx: &DrawContext<'_>,
    ) {
        if matches!(item, MenuItem::Empty) {
            return;
        }
        let enabled = item.enabled(ctx.tab);
        let brightness = if enabled { 1.0 } else { 0.25 };
        let icon = color_from_float(alpha, brightness, brightness, brightness);
        match item {
            MenuItem::Close => sprite(out, "spectator/close", x, y, icon),
            MenuItem::ScrollLeft => sprite(out, "spectator/scroll_left", x, y, icon),
            MenuItem::ScrollRight { .. } => sprite(out, "spectator/scroll_right", x, y, icon),
            MenuItem::RootPlayers(_) => sprite(out, "spectator/teleport_to_player", x, y, icon),
            MenuItem::RootTeams(_) => sprite(out, "spectator/teleport_to_team", x, y, icon),
            MenuItem::Player(p) => player_face(out, ctx, p.uuid, x, y, white(alpha)),
            MenuItem::Team(t) => {
                if let Some(rgb) = t.color {
                    out.push(GuiCommand::Fill {
                        x1: x + 1,
                        y1: y + 1,
                        x2: x + 15,
                        y2: y + 15,
                        color: scale_rgb(rgb, brightness),
                    });
                }
                player_face(out, ctx, t.icon, x, y, icon);
            }
            MenuItem::Empty => {}
        }
        if alpha > 0.0 && enabled {
            if let Some(metrics) = ctx.metrics {
                // The key labels are the digits (vanilla draws each slot's bound
                // `keyHotbarSlots` key, bottom-right of the slot).
                let key = KEY_LABELS[slot as usize];
                let width = metrics.width(key);
                out.push(GuiCommand::Text {
                    x: x + 19 - 2 - width,
                    y: y + 6 + 3,
                    text: key.to_owned(),
                    color: white(alpha),
                    shadow: true,
                });
            }
        }
    }
}

/// The slot keys' labels (vanilla `options.keyHotbarSlots[slot].getTranslatedKeyMessage`).
static KEY_LABELS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// One gui-atlas sprite, 16x16.
fn sprite(out: &mut Vec<GuiCommand>, path: &str, x: i32, y: i32, tint: Argb) {
    out.push(GuiCommand::Sprite {
        sprite: Identifier::minecraft(path),
        x,
        y,
        w: 16,
        h: 16,
        tint,
        blend: GuiBlend::Normal,
    });
}

/// `PlayerFaceExtractor.extractRenderState`: the head (8, 8, 8x8) and hat (40, 8, 8x8)
/// regions of the 64x64 skin, drawn 12x12 at the slot's (2, 2). The skin is the resolved
/// one from the entity store, else the profile's default.
fn player_face(
    out: &mut Vec<GuiCommand>,
    ctx: &DrawContext<'_>,
    uuid: Uuid,
    x: i32,
    y: i32,
    tint: Argb,
) {
    let body = match ctx.skins.get(&uuid) {
        Some(body) => body,
        None => &haste_entity::skin::default_skin(uuid).body,
    };
    for (u, region) in [(8.0, 8.0), (40.0, 8.0)] {
        out.push(GuiCommand::Texture {
            texture: body.clone(),
            x: x + 2,
            y: y + 2,
            w: 12,
            h: 12,
            u,
            v: region,
            uw: 8.0,
            vh: 8.0,
            tex_w: 64,
            tex_h: 64,
            tint,
            blend: GuiBlend::Normal,
        });
    }
}

/// `ARGB.white(alpha)`.
fn white(alpha: f32) -> Argb {
    (channel(alpha) << 24) | 0x00ff_ffff
}

/// `ARGB.colorFromFloat(alpha, r, g, b)`.
fn color_from_float(alpha: f32, r: f32, g: f32, b: f32) -> Argb {
    (channel(alpha) << 24) | (channel(r) << 16) | (channel(g) << 8) | channel(b)
}

/// `ARGB.as8BitChannel`: `Mth.floor(value * 255)`.
fn channel(v: f32) -> u32 {
    (v * 255.0).floor().clamp(0.0, 255.0) as u32
}

/// `ARGB.scaleRGB(color, scale)` (the alpha is kept).
fn scale_rgb(color: u32, scale: f32) -> Argb {
    let ch = |c: u32| ((c as f32 * scale).floor().clamp(0.0, 255.0)) as u32;
    (color & 0xff00_0000)
        | (ch(color >> 16 & 0xff) << 16)
        | (ch(color >> 8 & 0xff) << 8)
        | ch(color & 0xff)
}
