//! Interaction (player workstream): hotbar, crosshair target, breaking, placing/using, attacking,
//! picking, and the client-side block predictions that go with them.
//!
//! Per tick, before movement (vanilla handles key bindings before the player's own tick):
//! 1. `set_carried_item` when the hotbar slot changes;
//! 2. the crosshair target: blocks against outline shapes within `block_interaction_range`,
//!    entities within `entity_interaction_range`, the nearer one wins;
//! 3. attack: on the press tick an entity target gets `attack` + `punch`, a block target starts
//!    breaking, nothing gets a lone `punch` (a miss). While held, breaking continues (with a
//!    `punch` every tick, as vanilla swings every tick it mines); releasing it or looking away
//!    aborts. Survival: `player_action` start (sequence), progress per tick from
//!    `haste_physics::mining`, finish (sequence) when it reaches 1, then a 5-tick delay; a
//!    block that breaks in one tick is finished by the start alone. Creative: start only, the
//!    block breaks at once, 5 ticks between blocks; tools that cannot break blocks in creative
//!    (swords) send nothing;
//! 4. use: on press and every 4 ticks while held, `use_item_on` (block target, cursor inside the
//!    block, sequence) or `interact` (entity) or `use_item` (nothing targeted, held item not empty);
//! 5. pick block: `pick_item_from_block` / `pick_item_from_entity`.
//!
//! Predictions: a block we break is set to air locally at once (`take_world_edits`). Until the
//! server acknowledges our sequence number (`block_changed_ack`) its block updates for that
//! position are only recorded; on the ack the last state the server told us (or, if it said
//! nothing, the state from before our prediction) is put back. Observed on the vanilla server:
//! a break it rejects gets an ack and no block update, so without the revert our world would keep
//! a block the server still has.
use std::collections::HashMap;
use std::sync::Arc;

use glam::DVec3;
use haste_core::entity::ItemStackData;
use haste_core::input::InteractInput;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, Direction, EntityId, Identifier, RegistryIndex};
use haste_physics::mining::{BlockTags, MiningParams, ToolTable};
use haste_physics::raycast::{self, BlockHit};
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::{ClientboundPlay, ServerboundPlay};

use super::LocalPlayer;
use haste_core::game::GameMode;

/// Ticks between two broken blocks while attack stays held.
pub const DESTROY_DELAY: u8 = 5;
/// Ticks between repeated uses while use stays held.
pub const USE_DELAY: u8 = 4;

/// `player_action` statuses. 26.3 inserted `CHANGE_DESTROY_DIRECTION` at 1, moving abort to 2
/// and finish to 3 (enum order read from the server jar's `ServerboundPlayerActionPacket$Action`
/// and confirmed on the vanilla server: status 2 is handled as an abort). The wiki's older order
/// (0 start, 1 abort, 2 finish) makes every survival break an abort.
pub mod action {
    pub const START_DESTROY: i32 = 0;
    pub const CHANGE_DESTROY_DIRECTION: i32 = 1;
    pub const ABORT_DESTROY: i32 = 2;
    pub const STOP_DESTROY: i32 = 3;
}

/// Interaction-related attributes (server values; vanilla defaults until it sends them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractAttributes {
    pub block_break_speed: f64,
    pub mining_efficiency: f64,
    pub submerged_mining_speed: f64,
    pub block_reach: f64,
    pub entity_reach: f64,
}

impl Default for InteractAttributes {
    fn default() -> Self {
        // wiki "Attribute" defaults.
        Self {
            block_break_speed: 1.0,
            mining_efficiency: 0.0,
            submerged_mining_speed: 0.2,
            block_reach: 4.5,
            entity_reach: 3.0,
        }
    }
}

/// What the interaction step reads besides the world: game data and nearby entity boxes.
pub struct InteractEnv<'a> {
    pub data: &'a haste_data::GameData,
    /// Hit boxes of the entities that can be targeted (not ourselves).
    pub entities: &'a [(EntityId, Aabb)],
}

/// What the crosshair is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Block(BlockHit),
    Entity { id: EntityId, location: DVec3 },
}

/// Interaction state of the local player.
#[derive(Default)]
pub struct Interaction {
    pub input: InteractInput,
    last: InteractInput,
    pub target: Option<Target>,
    pub attrs: InteractAttributes,
    /// Block being broken in survival, with its progress 0..1.
    destroying: Option<(BlockPos, Direction)>,
    progress: f32,
    destroy_delay: u8,
    use_delay: u8,
    /// Last sequence number used for a prediction.
    sequence: i32,
    /// Predicted blocks: (sequence, pos, the server's state: before the prediction, then the
    /// latest block update held back).
    predictions: Vec<(i32, BlockPos, BlockStateId)>,
    world_edits: Vec<(BlockPos, BlockStateId)>,
    /// Hotbar 0..=8 and off hand.
    pub hotbar: [ItemStackData; 9],
    pub offhand: ItemStackData,
    /// Active effects on us: `minecraft:mob_effect` index -> amplifier.
    pub effects: HashMap<i32, i32>,
    tools: Option<Arc<ToolTable>>,
    tags: Option<BlockTags>,
    effect_ids: Option<EffectIds>,
    /// Last blocks we sent actions for (debug logging of their server updates).
    recent: Vec<BlockPos>,
}

#[derive(Clone, Copy, Debug)]
struct EffectIds {
    haste: i32,
    conduit_power: i32,
    mining_fatigue: i32,
}

impl Interaction {
    /// Loads the tool table (once; reads the data generator's item component reports) and the
    /// effect ids.
    pub fn prepare(&mut self, data: &haste_data::GameData) {
        if self.tools.is_none() {
            let t = ToolTable::load_default().unwrap_or_else(|e| {
                tracing::warn!("no tool components (mining speed with bare hands only): {e:#}");
                ToolTable::default()
            });
            self.tools = Some(Arc::new(t));
        }
        if self.effect_ids.is_none() {
            let id = |n: &str| {
                data.registries
                    .index("minecraft:mob_effect", &Identifier::minecraft(n))
                    .map(|i| i.0 as i32)
            };
            if let (Some(haste), Some(conduit_power), Some(mining_fatigue)) =
                (id("haste"), id("conduit_power"), id("mining_fatigue"))
            {
                self.effect_ids = Some(EffectIds {
                    haste,
                    conduit_power,
                    mining_fatigue,
                });
            }
        }
    }

    pub fn has_block_tags(&self) -> bool {
        self.tags.is_some()
    }

    pub fn set_block_tags(&mut self, tags: BlockTags) {
        self.tags = Some(tags);
    }

    pub fn set_tools(&mut self, tools: Arc<ToolTable>) {
        self.tools = Some(tools);
    }

    pub(super) fn respawned(&mut self) {
        self.destroying = None;
        self.progress = 0.0;
        self.target = None;
        self.effects.clear();
    }

    fn next_sequence(&mut self) -> i32 {
        self.sequence += 1;
        self.sequence
    }

    /// Predicts `pos` becomes `state` (`None`: left to the server); `old` is our current state.
    fn predict(&mut self, seq: i32, pos: BlockPos, old: BlockStateId, state: Option<BlockStateId>) {
        // An earlier prediction for the same block already holds the server's state.
        let server = self
            .predictions
            .iter()
            .rev()
            .find(|p| p.1 == pos)
            .map_or(old, |p| p.2);
        self.predictions.push((seq, pos, server));
        if let Some(s) = state {
            self.world_edits.push((pos, s));
        }
    }
}

impl LocalPlayer {
    /// This tick's mouse buttons and hotbar selection (applied inside the next tick).
    pub fn set_interact(&mut self, input: InteractInput) {
        self.int.input = input;
    }

    /// Block under the crosshair and the face hit.
    pub fn target(&self) -> Option<(BlockPos, Direction)> {
        match self.int.target {
            Some(Target::Block(h)) => Some((h.pos, h.face)),
            _ => None,
        }
    }

    /// Entity under the crosshair.
    pub fn target_entity(&self) -> Option<EntityId> {
        match self.int.target {
            Some(Target::Entity { id, .. }) => Some(id),
            _ => None,
        }
    }

    /// Our own breaking: block and crack stage 0..=9.
    pub fn break_progress(&self) -> Option<(BlockPos, u8)> {
        let (pos, _) = self.int.destroying?;
        if self.int.progress <= 0.0 {
            return None;
        }
        Some((pos, ((self.int.progress * 10.0) as u8).min(9)))
    }

    /// The held main-hand stack.
    pub fn main_hand(&self) -> &ItemStackData {
        &self.int.hotbar[self.held_slot.clamp(0, 8) as usize]
    }

    /// Predicted block changes to apply to the world now (breaking), and server states released
    /// by `block_changed_ack`.
    pub fn take_world_edits(&mut self) -> std::vec::Drain<'_, (BlockPos, BlockStateId)> {
        self.int.world_edits.drain(..)
    }

    /// A server block update: `true` if it was held back because we predicted that block and the
    /// server has not acknowledged the prediction yet (it is applied on the ack).
    pub fn defer_block_update(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        if self.int.recent.contains(&pos) {
            tracing::debug!(
                "block update {pos:?} -> {} (predicted: {})",
                state.0,
                self.int.predictions.iter().any(|p| p.1 == pos)
            );
        }
        let mut held = false;
        for p in self.int.predictions.iter_mut().filter(|p| p.1 == pos) {
            p.2 = state;
            held = true;
        }
        held
    }

    pub(super) fn handle_interact_packet(&mut self, p: &ClientboundPlay) -> bool {
        use ClientboundPlay as P;
        let me = self.entity_id.0;
        match p {
            P::BlockChangedAck(a) => {
                tracing::debug!("block_changed_ack {}", a.sequence);
                let (done, keep): (Vec<_>, Vec<_>) = self
                    .int
                    .predictions
                    .drain(..)
                    .partition(|p| p.0 <= a.sequence);
                self.int.predictions = keep;
                for (_, pos, server) in done {
                    // The server's state wins, unless a newer prediction still covers the block.
                    if !self.int.predictions.iter().any(|p| p.1 == pos) {
                        self.int.world_edits.push((pos, server));
                    }
                }
            }
            P::ContainerSetContent(c) if c.window_id == 0 => {
                for (i, s) in c.slots.iter().enumerate() {
                    self.set_inventory_menu_slot(i as i32, s);
                }
            }
            P::ContainerSetSlot(c) if c.window_id == 0 => {
                self.set_inventory_menu_slot(c.slot as i32, &c.item)
            }
            P::SetPlayerInventory(s) => match s.slot {
                0..=8 => self.int.hotbar[s.slot as usize] = s.item.clone(),
                40 => self.int.offhand = s.item.clone(),
                _ => {}
            },
            P::UpdateMobEffect(e) if e.entity_id == me => {
                self.int.effects.insert(e.effect, e.amplifier);
            }
            P::RemoveMobEffect(e) if e.entity_id == me => {
                self.int.effects.remove(&e.effect);
            }
            _ => return false,
        }
        true
    }

    /// Slot of the player inventory menu (window 0): 36..=44 hotbar, 45 off hand
    /// (wiki "Inventory" slot numbering).
    fn set_inventory_menu_slot(&mut self, slot: i32, item: &ItemStackData) {
        match slot {
            36..=44 => self.int.hotbar[(slot - 36) as usize] = item.clone(),
            45 => self.int.offhand = item.clone(),
            _ => {}
        }
    }

    fn eye(&self) -> DVec3 {
        self.state.pos + DVec3::new(0.0, self.attrs.eye_height_for(self.state.pose), 0.0)
    }

    /// The crosshair target for a look direction.
    pub(super) fn pick(
        &self,
        look: (f32, f32),
        world: &dyn BlockView,
        shapes: &dyn CollisionShapes,
        entities: &[(EntityId, Aabb)],
    ) -> Option<Target> {
        let eye = self.eye();
        let dir = raycast::look_vector(look.0, look.1);
        let a = self.int.attrs;
        let block = raycast::raycast_blocks(eye, dir, a.block_reach, world, shapes);
        let block_dist = block.map_or(f64::INFINITY, |b| b.distance);
        let entity = raycast::raycast_entities(
            eye,
            dir,
            a.entity_reach.min(block_dist),
            entities.iter().map(|e| e.1),
        );
        match (entity, block) {
            (Some((i, d)), _) => Some(Target::Entity {
                id: entities[i].0,
                location: eye + dir * d,
            }),
            (None, Some(b)) => Some(Target::Block(b)),
            (None, None) => None,
        }
    }

    pub(super) fn interact_step(
        &mut self,
        look: (f32, f32),
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let input = self.int.input;
        let last = std::mem::replace(&mut self.int.last, input);
        if self.int.tags.is_none() {
            self.int.tags = Some(BlockTags::default());
        }

        // 1. Hotbar.
        if let Some(slot) = input.hotbar {
            if slot <= 8 && slot as i32 != self.held_slot {
                self.held_slot = slot as i32;
                out.push(ServerboundPlay::SetCarriedItem(sb::SetCarriedItem {
                    slot: slot as i16,
                }));
                // A new tool restarts breaking (vanilla compares the held item).
                if self.int.destroying.is_some() {
                    self.abort_destroy(out);
                }
            }
        }

        // 2. Target.
        self.int.target = if self.game_mode == GameMode::Spectator {
            None
        } else {
            self.pick(look, world, &env.data.shapes, env.entities)
        };
        if self.game_mode == GameMode::Spectator {
            self.int.input = InteractInput::default();
            return;
        }

        // 3. Attack / break.
        let pressed = input.attack && !last.attack;
        if pressed {
            match self.int.target {
                Some(Target::Entity { id, .. }) => {
                    out.push(ServerboundPlay::Attack(sb::Attack { entity_id: id.0 }));
                    out.push(ServerboundPlay::Punch(sb::Punch {}));
                }
                Some(Target::Block(h)) => {
                    self.start_destroy(h, world, env, out);
                    out.push(ServerboundPlay::Punch(sb::Punch {}));
                }
                None => out.push(ServerboundPlay::Punch(sb::Punch {})),
            }
        }
        if input.attack {
            match self.int.target {
                Some(Target::Block(h)) => {
                    if self.continue_destroy(h, world, env, out) {
                        out.push(ServerboundPlay::Punch(sb::Punch {}));
                    }
                }
                _ => self.abort_destroy(out),
            }
        } else {
            self.abort_destroy(out);
        }

        // 4. Use.
        if self.int.use_delay > 0 {
            self.int.use_delay -= 1;
        }
        let use_now = input.use_item
            && (!last.use_item || self.int.use_delay == 0)
            && self.int.destroying.is_none();
        if use_now {
            self.int.use_delay = USE_DELAY;
            self.use_item(look, out);
        }

        // 5. Pick block.
        if input.pick_block && !last.pick_block {
            let include_data = false;
            match self.int.target {
                Some(Target::Block(h)) => {
                    out.push(ServerboundPlay::PickItemFromBlock(sb::PickItemFromBlock {
                        pos: h.pos,
                        include_data,
                    }))
                }
                Some(Target::Entity { id, .. }) => out.push(ServerboundPlay::PickItemFromEntity(
                    sb::PickItemFromEntity {
                        entity_id: id.0,
                        include_data,
                    },
                )),
                None => {}
            }
        }
        // The press-only fields are consumed; held buttons stay until the caller changes them.
        self.int.input.pick_block = false;
        self.int.input.hotbar = None;
    }

    fn use_item(&mut self, look: (f32, f32), out: &mut Vec<ServerboundPlay>) {
        let holding = !self.main_hand().is_empty();
        match self.int.target {
            Some(Target::Block(h)) => {
                let seq = self.int.next_sequence();
                let c = h.location - h.pos.as_dvec3();
                out.push(ServerboundPlay::UseItemOn(sb::UseItemOn {
                    hand: 0,
                    pos: h.pos,
                    face: h.face as i32,
                    cursor_x: c.x as f32,
                    cursor_y: c.y as f32,
                    cursor_z: c.z as f32,
                    inside_block: h.inside,
                    world_border_hit: false,
                    sequence: seq,
                }));
                if holding {
                    out.push(ServerboundPlay::Punch(sb::Punch {}));
                }
            }
            Some(Target::Entity { id, location }) => {
                out.push(ServerboundPlay::Interact(sb::Interact {
                    entity_id: id.0,
                    hand: 0,
                    target_offset: location - self.eye(),
                    sneaking: self.state.sneaking,
                }));
            }
            None => {
                if holding {
                    let seq = self.int.next_sequence();
                    out.push(ServerboundPlay::UseItem(sb::UseItem {
                        hand: 0,
                        sequence: seq,
                        yaw: look.0,
                        pitch: look.1,
                    }));
                }
            }
        }
    }

    /// Mining parameters for a block with the held item.
    pub fn mining_params(&self, state: BlockStateId, env: &InteractEnv<'_>) -> MiningParams {
        let data = env.data;
        let block = data.blocks.block_of(state);
        let index = data.blocks.block_index(block).map_or(u32::MAX, |i| i.0);
        let tags = self.int.tags.as_ref();
        let (tool_speed, tool_correct) = self
            .held_tool(data)
            .map(|t| {
                t.speed_for(|set| {
                    tags.is_some_and(|tg| tg.matches(set, block, index))
                        || matches!(set, haste_physics::mining::BlockSet::Block(b) if b == block)
                        || matches!(set, haste_physics::mining::BlockSet::List(l) if l.contains(block))
                })
            })
            .unwrap_or((1.0, None));
        let level = |id: Option<i32>| {
            id.and_then(|i| self.int.effects.get(&i))
                .map_or(0, |a| (*a + 1).max(0) as u32)
        };
        let e = self.int.effect_ids;
        let a = self.int.attrs;
        MiningParams {
            hardness: data.destroy_time(state),
            requires_correct_tool: data.requires_correct_tool(state),
            tool_speed,
            tool_correct,
            mining_efficiency: a.mining_efficiency,
            haste_level: level(e.map(|e| e.haste)).max(level(e.map(|e| e.conduit_power))),
            fatigue_level: level(e.map(|e| e.mining_fatigue)),
            block_break_speed: a.block_break_speed,
            submerged_mining_speed: a.submerged_mining_speed,
            eyes_in_water: self.state.eyes_in_water,
            on_ground: self.state.on_ground,
        }
    }

    fn held_tool<'t>(
        &'t self,
        data: &haste_data::GameData,
    ) -> Option<&'t haste_physics::mining::Tool> {
        let item = self.main_hand();
        if item.is_empty() {
            return None;
        }
        let name = data
            .registries
            .name("minecraft:item", item.item.unwrap_or(RegistryIndex(0)))?;
        self.int.tools.as_ref()?.get(name)
    }

    fn start_destroy(
        &mut self,
        h: BlockHit,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let state = world.block_state(h.pos);
        // A block we broke this tick (or whose break the server has not acknowledged yet) is
        // gone for us even if the world has not applied the edit yet.
        let predicted_gone = self.int.predictions.iter().any(|p| p.1 == h.pos);
        if env.data.is_air(state) || predicted_gone {
            return;
        }
        if self.game_mode == GameMode::Adventure {
            // Breaking in adventure depends on the held item's can_break predicate (not decoded).
            return;
        }
        if self.game_mode == GameMode::Creative {
            if self.int.destroy_delay > 0 {
                return;
            }
            if self
                .held_tool(env.data)
                .is_some_and(|t| !t.can_destroy_blocks_in_creative)
            {
                return;
            }
            self.int.destroy_delay = DESTROY_DELAY;
            self.send_destroy(action::START_DESTROY, h, state, true, env, out);
            return;
        }
        if self.int.destroying.is_some_and(|d| d.0 == h.pos) {
            return;
        }
        if self.int.destroying.is_some() {
            self.abort_destroy(out);
        }
        let params = self.mining_params(state, env);
        let d = params.progress_per_tick();
        tracing::debug!("start breaking {:?}: {params:?} -> {d} per tick", h.pos);
        let instant = d >= 1.0;
        self.send_destroy(action::START_DESTROY, h, state, instant, env, out);
        if !instant {
            self.int.destroying = Some((h.pos, h.face));
            self.int.progress = 0.0;
        }
    }

    /// One tick of held attack on a block; true if the arm swings this tick.
    fn continue_destroy(
        &mut self,
        h: BlockHit,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) -> bool {
        if self.int.destroy_delay > 0 {
            self.int.destroy_delay -= 1;
            return true;
        }
        if self.game_mode == GameMode::Creative {
            self.start_destroy(h, world, env, out);
            return true;
        }
        if self.game_mode == GameMode::Adventure {
            return false;
        }
        match self.int.destroying {
            Some((pos, face)) if pos == h.pos => {
                if face != h.face {
                    // Same block, another face under the crosshair (new in 26.3).
                    out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                        status: action::CHANGE_DESTROY_DIRECTION,
                        pos,
                        face: h.face as u8,
                        sequence: 0,
                    }));
                    self.int.destroying = Some((pos, h.face));
                }
                let state = world.block_state(pos);
                if env.data.is_air(state) {
                    self.int.destroying = None;
                    return false;
                }
                self.int.progress += self.mining_params(state, env).progress_per_tick();
                if self.int.progress >= 1.0 {
                    self.send_destroy(action::STOP_DESTROY, h, state, true, env, out);
                    self.int.destroying = None;
                    self.int.progress = 0.0;
                    self.int.destroy_delay = DESTROY_DELAY;
                }
                true
            }
            _ => {
                self.start_destroy(h, world, env, out);
                true
            }
        }
    }

    fn abort_destroy(&mut self, out: &mut Vec<ServerboundPlay>) {
        if let Some((pos, face)) = self.int.destroying.take() {
            // Abort carries no prediction (sequence 0).
            out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                status: action::ABORT_DESTROY,
                pos,
                face: face as u8,
                sequence: 0,
            }));
            self.int.progress = 0.0;
        }
    }

    fn send_destroy(
        &mut self,
        status: i32,
        h: BlockHit,
        state: BlockStateId,
        breaks: bool,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let seq = self.int.next_sequence();
        if !self.int.recent.contains(&h.pos) {
            if self.int.recent.len() >= 8 {
                self.int.recent.remove(0);
            }
            self.int.recent.push(h.pos);
        }
        tracing::debug!(
            "player_action {status} {:?} seq {seq} on_ground {} progress {}",
            h.pos,
            self.state.on_ground,
            self.int.progress
        );
        out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
            status,
            pos: h.pos,
            face: h.face as u8,
            sequence: seq,
        }));
        if breaks {
            // Predict the block gone; a waterlogged block leaves water, which we leave to the
            // server's update.
            let to = env.data.fluid(state).is_none().then_some(BlockStateId::AIR);
            self.int.predict(seq, h.pos, state, to);
        }
    }
}

#[cfg(test)]
mod tests;
