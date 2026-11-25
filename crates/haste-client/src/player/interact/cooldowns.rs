//! Item cooldowns (vanilla `ItemCooldowns`, `Player.cooldowns`): the cooldown groups the
//! server's `cooldown` packets start (`ClientPacketListener.handleItemCooldown`), ticked
//! with the player (`Player.tick`), the percent the hotbar overlay reads
//! (`GuiGraphicsExtractor.itemCooldown`) and the use gating of
//! `MultiPlayerGameMode.performUseItemOn`/`useItem`.

use std::collections::HashMap;

use haste_core::entity::ItemStackData;
use haste_core::Identifier;

use super::Interaction;

/// One running cooldown (vanilla `ItemCooldowns.CooldownInstance`): the player-tick counter's
/// value when it started and when it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CooldownInstance {
    pub start_tick: i32,
    pub end_tick: i32,
}

/// Vanilla `ItemCooldowns`: cooldowns by group id (an item's `minecraft:use_cooldown`
/// `cooldown_group`, else its `minecraft:item` registry id), on the player's own tick counter.
///
/// Like vanilla, the times are the local tick counter, not the server's: a `cooldown` packet
/// applies at the tick it arrived in (packet drain runs before the tick, like vanilla's
/// main-thread packet processing), so a cooldown started mid-game lines up with the server's
/// within the connection's latency.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemCooldowns {
    cooldowns: HashMap<Identifier, CooldownInstance>,
    tick_count: i32,
}

impl ItemCooldowns {
    /// `ItemCooldowns.tick` (`Player.tick`): one player tick past; a cooldown whose end
    /// reached the counter is dropped.
    pub fn tick(&mut self) {
        self.tick_count += 1;
        self.cooldowns.retain(|_, c| c.end_tick > self.tick_count);
    }

    /// `ItemCooldowns.addCooldown(group, time)`: the group's cooldown restarts from now.
    pub fn add(&mut self, group: Identifier, ticks: i32) {
        let now = self.tick_count;
        self.cooldowns.insert(
            group,
            CooldownInstance {
                start_tick: now,
                end_tick: now + ticks,
            },
        );
    }

    /// `ItemCooldowns.removeCooldown(group)` (the packet's zero duration).
    pub fn remove(&mut self, group: &Identifier) {
        self.cooldowns.remove(group);
    }

    /// `ItemCooldowns.getCooldownPercent(itemStack, partial)` for a group already resolved.
    /// `Mth.clamp(remaining / duration, 0.0, 1.0)`, 0 without a cooldown.
    pub fn percent(&self, group: &Identifier, partial: f32) -> f32 {
        let Some(c) = self.cooldowns.get(group) else {
            return 0.0;
        };
        let duration = (c.end_tick - c.start_tick) as f32;
        let remaining = c.end_tick as f32 - (self.tick_count as f32 + partial);
        (remaining / duration).clamp(0.0, 1.0)
    }

    /// The entries and the tick counter, for the HUD's mirror (the overlay asks per item by
    /// group; `percent` is computed on the Java side's partial tick from these).
    pub fn snapshot(&self) -> (i32, Vec<(Identifier, CooldownInstance)>) {
        (
            self.tick_count,
            self.cooldowns
                .iter()
                .map(|(g, c)| (g.clone(), *c))
                .collect(),
        )
    }
}

impl Interaction {
    /// `ItemCooldowns.getCooldownGroup(itemStack)`: the stack's `minecraft:use_cooldown`
    /// group, else the item's registry id. `None` when the item cannot be resolved (no data,
    /// unknown item): nothing can be on cooldown for it.
    ///
    /// The stack's component patch can override the component, which this side does not
    /// decode (like `use_effects`); the default components from the report decide.
    pub fn cooldown_group(
        &self,
        stack: &ItemStackData,
        data: &haste_data::GameData,
    ) -> Option<Identifier> {
        let name = stack
            .item
            .and_then(|i| data.registries.name("minecraft:item", i))?;
        Some(
            self.item_components
                .as_ref()
                .and_then(|c| c.cooldown_group(name))
                .cloned()
                .unwrap_or_else(|| name.clone()),
        )
    }

    /// `ItemCooldowns.isOnCooldown(itemStack)`: the use gating of
    /// `MultiPlayerGameMode.performUseItemOn`/`useItem` (ender pearls, shields after an axe
    /// hit, wind charges).
    pub fn is_on_cooldown(&self, stack: &ItemStackData, data: &haste_data::GameData) -> bool {
        self.cooldown_group(stack, data)
            .is_some_and(|g| self.cooldowns.percent(&g, 0.0) > 0.0)
    }
}
