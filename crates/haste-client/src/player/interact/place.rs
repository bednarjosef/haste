//! Placing blocks (player workstream): the client half of `BlockItem.place` —
//! which held items place, where the block goes (`BlockPlaceContext
//! .getClickedPos`), the `canPlace` obstruction check (`Level.isUnobstructed`
//! with the entity boxes) and the place sound. Split out of `use_item.rs`
//! (vanilla keeps them apart too: `BlockItem` against `Minecraft.startUseItem`).

use glam::DVec3;
use haste_core::entity::ItemStackData;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, Identifier};
use haste_physics::mining::BlockSet;
use haste_physics::raycast::BlockHit;

use super::super::LocalPlayer;
use super::InteractEnv;

impl LocalPlayer {
    /// The held item's block, if it is a block item (item id = block id in vanilla,
    /// `BlockItemId.create(name)`; the 14 crop pairs with differing names are missing
    /// here — placing crops is rare and the server confirms anyway).
    fn held_block<'d>(
        &self,
        item: &ItemStackData,
        data: &'d haste_data::GameData,
    ) -> Option<&'d haste_core::Identifier> {
        if item.is_empty() {
            return None;
        }
        let index = item.item?;
        data.registries
            .name("minecraft:item", index)
            .and_then(|id| data.blocks.default_state(id).map(|_| id))
    }

    /// Whether `item` is a block item (`BlockItem`): its `use_item_on` places a block, a Success
    /// on the client.
    pub(super) fn is_block_item(&self, item: &ItemStackData, data: &haste_data::GameData) -> bool {
        self.held_block(item, data).is_some()
    }

    /// `BlockItem.place`'s placement position and its `canPlace` obstruction check
    /// (`BlockPlaceContext.getClickedPos`, then `Level.isUnobstructed`): `Some(pos)`
    /// when the block can go there, `None` when an entity's box intersects the new
    /// block's collision shape at the placement position — the FAIL that stops the
    /// use with no swing and no sound.
    ///
    /// The placement position is the clicked block's own cell when it is replaceable
    /// (`BlockState.canBeReplaced`: the `minecraft:replaceable` tag as the server
    /// synced it — short grass, water, snow layers; without a tag sync yet nothing
    /// counts, which only misplaces this check, never a packet), else the neighbour
    /// in the clicked face's direction. The entity set is the one the environment
    /// carries (the pickable entities — vanilla's `blocksBuilding` set: item
    /// entities and experience orbs never block a placement either way) plus the
    /// placing player's own box, which vanilla passes with source null: a block
    /// cannot be placed into the player standing in it. The state placed is the
    /// block's default; `getStateForPlacement`'s context-driven property choices
    /// and `canSurvive` are not modelled (the server decides those), so this only
    /// answers the obstruction half of `canPlace`.
    pub(super) fn placement_pos(
        &self,
        h: BlockHit,
        item: &ItemStackData,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
    ) -> Option<BlockPos> {
        let block = self.held_block(item, env.data)?;
        let state = env.data.blocks.default_state(block)?;
        let clicked = world.block_state(h.pos);
        let replaceable = env.data.is_air(clicked)
            || self.int.tags.as_ref().is_some_and(|t| {
                t.matches(
                    &BlockSet::Tag(Identifier::minecraft("replaceable")),
                    env.data.blocks.block_of(clicked),
                    env.data
                        .blocks
                        .block_index(env.data.blocks.block_of(clicked))
                        .map_or(u32::MAX, |i| i.0),
                )
            });
        let pos = if replaceable {
            h.pos
        } else {
            h.pos.offset(h.face)
        };
        // `isUnobstructed` with an empty collision shape is always true.
        let boxes = env.data.shapes.collision_boxes(state);
        if boxes.is_empty() {
            return Some(pos);
        }
        let blocked = |b: &Aabb| {
            boxes.iter().any(|s| {
                b.intersects(&Aabb {
                    min: s.min + pos.as_dvec3(),
                    max: s.max + pos.as_dvec3(),
                })
            })
        };
        if env.entities.iter().any(|(_, b)| blocked(b)) {
            return None;
        }
        if blocked(&Aabb::entity(
            self.state.pos,
            self.attrs.box_width(),
            self.attrs.height_for(self.state.pose),
        )) {
            return None;
        }
        Some(pos)
    }

    /// Whether the block's own `useWithoutItem`/`useItemOn` is known to succeed on the client
    /// (containers, doors, redstone switches, workstations, beds, ...), so `startUseItem`
    /// stops at it instead of falling through to `useItem`. Mod blocks and anything not listed
    /// pass through; the server still runs the real use.
    pub(super) fn block_has_use(&self, state: BlockStateId, data: &haste_data::GameData) -> bool {
        const USES: &[&str] = &[
            "chest",
            "barrel",
            "shulker_box",
            "furnace",
            "smoker",
            "crafting_table",
            "door",
            "trapdoor",
            "fence_gate",
            "button",
            "lever",
            "bed",
            "anvil",
            "enchanting_table",
            "brewing_stand",
            "hopper",
            "dispenser",
            "dropper",
            "note_block",
            "repeater",
            "comparator",
            "daylight_detector",
            "bell",
            "lectern",
            "cartography_table",
            "smithing_table",
            "grindstone",
            "stonecutter",
            "loom",
            "beacon",
            "command_block",
            "jukebox",
            "crafter",
            "respawn_anchor",
        ];
        let path = data.blocks.block_of(state).path();
        USES.iter().any(|u| {
            path.strip_suffix(*u)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('_'))
        })
    }

    /// The place sound for a `use_item_on` with a block item held
    /// (`BlockItem.place` plays it on the client too — the server's copy excludes
    /// us), at the placement position. The held block item's block decides;
    /// unknown items play nothing.
    pub(super) fn own_place_sound(
        &mut self,
        pos: BlockPos,
        item: &ItemStackData,
        env: &InteractEnv<'_>,
    ) {
        let Some(block) = self.held_block(item, env.data) else {
            return;
        };
        let state = env.data.blocks.default_state(block);
        if let Some(state) = state {
            if let Some(t) = env.data.block_sound_type(state) {
                self.own_sounds.block_placed(
                    pos.as_dvec3() + DVec3::new(0.5, 0.5, 0.5),
                    t,
                    &mut self.sound_out,
                );
            }
        }
    }
}
