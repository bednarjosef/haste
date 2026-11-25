//! Placing blocks (player workstream): the client half of `BlockItem.place` —
//! which held items place, where the block goes (`BlockPlaceContext.getClickedPos`
//! with the slab replaceable rule), the `canPlace` obstruction check
//! (`Level.isUnobstructed` with the entity boxes), the state placed (each covered
//! vanilla class's `getStateForPlacement`, chosen by the `block_classes.json` dump)
//! and the place sound. Split out of `use_item.rs` (vanilla keeps them apart too:
//! `BlockItem` against `Minecraft.startUseItem`).

use glam::DVec3;
use haste_core::entity::ItemStackData;
use haste_core::view::{BlockPhysics, BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, BlockStateId, Direction, Identifier};
use haste_data::BlockStates;
use haste_physics::mining::BlockSet;
use haste_physics::raycast::BlockHit;

use super::super::LocalPlayer;
use super::InteractEnv;

/// `BlockItem.place`'s client half: where the block goes and what we predict there.
pub(super) struct Placement {
    pub(super) pos: BlockPos,
    /// The state we set in the client world at once; `None` when the block's class is
    /// not covered (nothing predicted, the server's `block_update` shows the block).
    pub(super) state: Option<BlockStateId>,
}

/// What the covered `getStateForPlacement`s read, vanilla's `BlockPlaceContext`: the
/// world, the data, the placement position, the hit that caused the place and the
/// player's look yaw.
pub(super) struct BlockPlaceContext<'a> {
    world: &'a dyn BlockView,
    env: &'a InteractEnv<'a>,
    /// The placement position (`getClickedPos`).
    pos: BlockPos,
    hit: BlockHit,
    /// The player's yaw (`getHorizontalDirection`'s input).
    look_yaw: f32,
}

impl LocalPlayer {
    /// The held item's block, if it is a block item (item id = block id in vanilla,
    /// `BlockItemId.create(name)`; the 14 crop pairs with differing names are missing
    /// here — placing crops is rare and the server confirms anyway).
    fn held_block<'d>(
        &self,
        item: &ItemStackData,
        data: &'d haste_data::GameData,
    ) -> Option<&'d Identifier> {
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

    /// `BlockItem.place`'s client half: the placement position, the state that goes
    /// there (`getStateForPlacement`) and the `canPlace` obstruction check
    /// (`BlockPlaceContext.canPlace` / `getClickedPos`, then `Level.isUnobstructed`).
    /// `None` when the place fails client-side — the FAIL that stops the use with no
    /// swing and no sound. The packet still went out either way (vanilla sends it
    /// inside `startPrediction`, before `place` runs).
    ///
    /// The placement position is the clicked block's own cell when it is replaceable
    /// (`BlockState.canBeReplaced`: the `minecraft:replaceable` tag as the server
    /// synced it — short grass, water, snow layers — plus `SlabBlock`'s
    /// context-driven rule for a same-kind slab, which is what makes a double slab;
    /// without a tag sync yet nothing counts, which only misplaces this check, never
    /// a packet), else the neighbour in the clicked face's direction, which then
    /// has to be replaceable itself (`BlockPlaceContext.canPlace`'s second half).
    ///
    /// The obstruction check runs against the placement state's collision shape (or
    /// the default state's for a class we do not cover) with the entity set the
    /// environment carries (the pickable entities — vanilla's `blocksBuilding` set:
    /// item entities and experience orbs never block a placement either way) plus
    /// the placing player's own box, which vanilla passes with source null: a block
    /// cannot be placed into the player standing in it.
    ///
    /// `Placement::state` is `Some` for the block classes whose
    /// `getStateForPlacement` we port (see [`Self::placement_state`]) — the caller
    /// predicts that state into the client world. `None` means the class is not
    /// covered: the place itself behaves like before (swing, sound, no prediction)
    /// and the server's `block_update` shows the block a round trip later.
    /// `canSurvive` is the server's to decide either way; a state it rejects comes
    /// back with the ack, which restores what the server has.
    pub(super) fn placement(
        &self,
        h: BlockHit,
        item: &ItemStackData,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        look_yaw: f32,
    ) -> Option<Placement> {
        let block = self.held_block(item, env.data)?.clone();
        let default_state = env.data.blocks.default_state(&block)?;
        let clicked = world.block_state(h.pos);
        let pos = if self.is_replaceable_for_placement(clicked, &block, h, true, env) {
            h.pos
        } else {
            let pos = h.pos.offset(h.face);
            let neighbour = world.block_state(pos);
            if !self.is_replaceable_for_placement(neighbour, &block, h, false, env) {
                // `BlockPlaceContext.canPlace`: the clicked block is not replaceable,
                // so the neighbour has to be.
                return None;
            }
            pos
        };
        let state = self.placement_state(
            &block,
            default_state,
            &BlockPlaceContext {
                world,
                env,
                pos,
                hit: h,
                look_yaw,
            },
        );
        let shape = state.unwrap_or(default_state);
        // `isUnobstructed` with an empty collision shape is always true.
        let boxes = env.data.shapes.collision_boxes(shape);
        if !boxes.is_empty() {
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
        }
        Some(Placement { pos, state })
    }

    /// `BlockState.canBeReplaced(context)` for the cell a placement would go into:
    /// the `minecraft:replaceable` tag (as the server synced it), or `SlabBlock`'s
    /// context rule when the cell holds a same-kind slab. `clicked` is the cell the
    /// crosshair hit; for any other cell (the neighbour a non-replaceable clicked
    /// block redirects to) the slab rule's `replacingClickedOnBlock()` is false and
    /// a same-kind slab is always replaceable.
    fn is_replaceable_for_placement(
        &self,
        cell: BlockStateId,
        block: &Identifier,
        h: BlockHit,
        clicked: bool,
        env: &InteractEnv<'_>,
    ) -> bool {
        if env.data.is_air(cell) {
            return true;
        }
        let tag = self.int.tags.as_ref().is_some_and(|t| {
            t.matches(
                &BlockSet::Tag(Identifier::minecraft("replaceable")),
                env.data.blocks.block_of(cell),
                env.data
                    .blocks
                    .block_index(env.data.blocks.block_of(cell))
                    .map_or(u32::MAX, |i| i.0),
            )
        });
        if tag {
            return true;
        }
        // `SlabBlock.canBeReplaced`: only a same-kind, non-double slab, and (in the
        // clicked cell) only into its other half.
        if env.data.blocks.block_of(cell) != block {
            return false;
        }
        let kind = env
            .data
            .blocks
            .property_value(cell, "type")
            .unwrap_or("double");
        if kind == "double" {
            return false;
        }
        if !clicked {
            return true;
        }
        let above = h.location.y - h.pos.y as f64 > 0.5;
        if kind == "bottom" {
            h.face == Direction::Up || (above && h.face.is_horizontal())
        } else {
            h.face == Direction::Down || (!above && h.face.is_horizontal())
        }
    }

    /// The state a placement puts in the client world: the covered vanilla classes'
    /// `getStateForPlacement`, chosen by the block's Java class from the
    /// `block_classes.json` dump (rule 7: the rules come from the dump, not a block
    /// list). `None` for a class we do not cover — nothing is predicted then, the
    /// server's `block_update` shows the block — and the class is logged once at
    /// debug. `canSurvive` is not modelled (the server decides); a state it rejects
    /// is restored by the ack.
    fn placement_state(
        &self,
        block: &Identifier,
        default_state: BlockStateId,
        ctx: &BlockPlaceContext<'_>,
    ) -> Option<BlockStateId> {
        use super::block_classes::PlacementClass;
        let name = block.to_string();
        let Some(class) = super::block_classes::placement_class(&name) else {
            super::block_classes::note_uncovered_class(&name);
            return None;
        };
        let blocks = &ctx.env.data.blocks;
        match class {
            PlacementClass::Block => block_state_for_placement(default_state),
            PlacementClass::RotatedPillarBlock => {
                rotated_pillar_block_state_for_placement(blocks, default_state, ctx.hit)
            }
            PlacementClass::SlabBlock => slab_block_state_for_placement(blocks, default_state, ctx),
            PlacementClass::StairBlock => {
                stair_block_state_for_placement(blocks, default_state, ctx)
            }
        }
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

/// `Block.getStateForPlacement`: the block's default state.
fn block_state_for_placement(default: BlockStateId) -> Option<BlockStateId> {
    Some(default)
}

/// `RotatedPillarBlock.getStateForPlacement`: the default state with the axis of the
/// clicked face (`context.getClickedFace().getAxis()`).
fn rotated_pillar_block_state_for_placement(
    blocks: &BlockStates,
    default: BlockStateId,
    h: BlockHit,
) -> Option<BlockStateId> {
    let axis = match h.face {
        Direction::Down | Direction::Up => "y",
        Direction::North | Direction::South => "z",
        Direction::West | Direction::East => "x",
    };
    blocks.with_property(default, "axis", axis)
}

/// `SlabBlock.getStateForPlacement`: clicking into a same-kind slab doubles it (its
/// `waterlogged` goes, two slabs displace the fluid); otherwise the default state,
/// bottom or top by the clicked face and the click height, waterlogged from a water
/// source at the position.
fn slab_block_state_for_placement(
    blocks: &BlockStates,
    default: BlockStateId,
    ctx: &BlockPlaceContext<'_>,
) -> Option<BlockStateId> {
    let replaced = ctx.world.block_state(ctx.pos);
    if blocks.block_of(replaced) == blocks.block_of(default) {
        return blocks
            .with_property(replaced, "type", "double")
            .and_then(|s| blocks.with_property(s, "waterlogged", "false"));
    }
    let half = if half_is_top(ctx.hit, ctx.pos) {
        "top"
    } else {
        "bottom"
    };
    let state = blocks.with_property(default, "type", half)?;
    waterlogged_from_fluid(blocks, state, ctx)
}

/// `StairBlock.getStateForPlacement`: the default state with `facing` from the
/// player's horizontal direction (`context.getHorizontalDirection()`), `half` by
/// the clicked face and click height, `waterlogged` from a water source at the
/// position. `shape` is left at its default — it depends on the neighbours
/// (`getStairsShape`), and the server's `block_update` corrects it with the ack.
fn stair_block_state_for_placement(
    blocks: &BlockStates,
    default: BlockStateId,
    ctx: &BlockPlaceContext<'_>,
) -> Option<BlockStateId> {
    // `Direction.fromYRot`: floor(yaw / 90 + 0.5) & 3 -> 2D data value (0 south,
    // 1 west, 2 north, 3 east), which is also the `facing` property's value.
    let facing = match (ctx.look_yaw as f64 / 90.0 + 0.5).floor() as i32 & 3 {
        0 => "south",
        1 => "west",
        2 => "north",
        _ => "east",
    };
    let half = if half_is_top(ctx.hit, ctx.pos) {
        "top"
    } else {
        "bottom"
    };
    let state = blocks.with_property(default, "facing", facing)?;
    let state = blocks.with_property(state, "half", half)?;
    waterlogged_from_fluid(blocks, state, ctx)
}

/// The shared half rule of `SlabBlock`/`StairBlock.getStateForPlacement`: top when
/// the clicked face is DOWN (placing under something) or the click is in the upper
/// half of the cell with a horizontal face.
fn half_is_top(h: BlockHit, pos: BlockPos) -> bool {
    h.face == Direction::Down || (h.face != Direction::Up && h.location.y - pos.y as f64 > 0.5)
}

/// `setValue(WATERLOGGED, fluidState.is(Fluids.WATER))`: a water source at the
/// placement position leaves the block waterlogged (only the source fluid counts —
/// flowing water is `FLOWING_WATER`). Blocks without the property keep their state.
fn waterlogged_from_fluid(
    blocks: &BlockStates,
    state: BlockStateId,
    ctx: &BlockPlaceContext<'_>,
) -> Option<BlockStateId> {
    if blocks.property_value(state, "waterlogged").is_none() {
        return Some(state);
    }
    let source = ctx
        .env
        .data
        .fluid(ctx.world.block_state(ctx.pos))
        .is_some_and(|f| {
            f.kind == haste_core::view::FluidKind::Water && (f.height - 8.0 / 9.0).abs() < 1.0e-6
        });
    blocks.with_property(state, "waterlogged", if source { "true" } else { "false" })
}
