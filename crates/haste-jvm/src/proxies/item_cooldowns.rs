//! The item cooldowns mirrored for the HUD (moved out of `proxies.rs` to keep that file
//! within the size guard). The client's `ItemCooldowns` (vanilla `Player.cooldowns`) lives in
//! haste-client; the vanilla Java `GuiGraphicsExtractor.itemCooldown` asks for a stack's
//! cooldown percent per frame through the `itemCooldown` native, which reads the snapshot the
//! client publishes here once per tick.

use std::collections::HashMap;

use haste_core::Identifier;
use parking_lot::Mutex;

/// One running cooldown (vanilla `ItemCooldowns.CooldownInstance`) on the player's tick
/// counter: the tick it started and the tick it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemCooldownHud {
    pub start_tick: i32,
    pub end_tick: i32,
}

/// The snapshot of the player's `ItemCooldowns` the HUD reads (vanilla
/// `ItemCooldowns.getCooldownPercent`): the tick counter and every group's cooldown on it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemCooldownsHud {
    /// The player's tick counter as of the last `ItemCooldowns.tick`.
    pub tick_count: i32,
    /// Cooldown group -> its running cooldown.
    pub cooldowns: HashMap<Identifier, ItemCooldownHud>,
}

impl ItemCooldownsHud {
    /// `ItemCooldowns.getCooldownPercent(group, partial)`: `Mth.clamp(remaining / duration,
    /// 0.0F, 1.0F)`, 0 when the group has no cooldown.
    pub fn percent(&self, group: &Identifier, partial: f32) -> f32 {
        let Some(c) = self.cooldowns.get(group) else {
            return 0.0;
        };
        let duration = (c.end_tick - c.start_tick) as f32;
        let remaining = c.end_tick as f32 - (self.tick_count as f32 + partial);
        (remaining / duration).clamp(0.0, 1.0)
    }
}

/// The live snapshot (set by the client once per tick, read by the `itemCooldown` native once
/// per decorated stack per frame). A static rather than a `GameState` field: the state is
/// write-only from the client's side and read-only from Java's.
static ITEM_COOLDOWNS: std::sync::LazyLock<Mutex<ItemCooldownsHud>> =
    std::sync::LazyLock::new(|| Mutex::new(ItemCooldownsHud::default()));

/// Publishes this tick's cooldowns (the client's `ItemCooldowns` snapshot).
pub fn set_item_cooldowns(cooldowns: ItemCooldownsHud) {
    *ITEM_COOLDOWNS.lock() = cooldowns;
}

/// The cooldown percent the HUD asks for (vanilla `GuiGraphicsExtractor.itemCooldown` reading
/// `player.getCooldowns()`), by group and the frame's partial tick. An unknown group id (a
/// mod name Java could not resolve) answers 0: no overlay.
pub(crate) fn item_cooldown_percent(group: &str, partial: f32) -> f32 {
    match Identifier::parse(group) {
        Ok(g) => ITEM_COOLDOWNS.lock().percent(&g, partial),
        Err(_) => 0.0,
    }
}
