//! Block physics properties from the oracle dump `block_properties.json`
//! (friction/speed/jump per block) and the `BlockPhysics`/`BlockStateTable` impls for
//! `GameData`/`BlockStates`.

use haste_core::view::{BlockPhysics, BlockStateTable, BlockView, FluidInfo};
use haste_core::{BlockPos, BlockStateId, DVec3, Direction};

use crate::blocks::BlockStates;
use crate::GameData;

/// Per-block friction/speed/jump from `block_properties.json`. Missing blocks use vanilla
/// defaults (0.6 / 1.0 / 1.0).
#[derive(Clone, Copy)]
pub struct BlockProps {
    pub friction: f32,
    pub speed_factor: f32,
    pub jump_factor: f32,
    /// Hardness (`destroy_time` in the dump; -1 = unbreakable). NaN when the dump predates it.
    pub destroy_time: f32,
    /// Drops only with the correct tool (`requires_correct_tool_for_drops`).
    pub requires_correct_tool: bool,
}

impl Default for BlockProps {
    fn default() -> Self {
        Self {
            friction: 0.6,
            speed_factor: 1.0,
            jump_factor: 1.0,
            destroy_time: f32::NAN,
            requires_correct_tool: false,
        }
    }
}

/// Block-name -> BlockProps table loaded from the oracle dump.
pub struct BlockProperties {
    /// Index parallels `BlockStates`' block descriptors are NOT name-ordered; we look up by
    /// block index into `BlockStates` via a side table built at load.
    props: Vec<BlockProps>,
    /// States whose destroy time / correct-tool flag differ from their block's default state.
    state_overrides: std::collections::HashMap<u32, (f32, bool)>,
}

#[derive(serde::Deserialize)]
struct PropsFile {
    blocks: std::collections::HashMap<String, RawProps>,
}

#[derive(serde::Deserialize, Clone)]
struct RawProps {
    #[serde(default = "d_friction")]
    friction: f32,
    #[serde(default = "one")]
    speed_factor: f32,
    #[serde(default = "one")]
    jump_factor: f32,
    #[serde(default = "nan")]
    destroy_time: f32,
    #[serde(default)]
    requires_correct_tool_for_drops: bool,
    /// State id -> differing breaking values.
    #[serde(default)]
    states: std::collections::HashMap<String, RawStateProps>,
}

#[derive(serde::Deserialize, Clone, Copy)]
struct RawStateProps {
    destroy_time: f32,
    requires_correct_tool_for_drops: bool,
}

fn nan() -> f32 {
    f32::NAN
}

fn d_friction() -> f32 {
    0.6
}
fn one() -> f32 {
    1.0
}

impl BlockProperties {
    /// Parses `block_properties.json`. `blocks` provides the block-name order to index by.
    pub(crate) fn parse(json: &str, blocks: &BlockStates) -> anyhow::Result<Self> {
        let file: PropsFile = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("block_properties.json: {e}"))?;
        // One props slot per block, indexed like BlockStates' internal block table: we ask
        // BlockStates to place them (it owns the name -> index map).
        let mut props = vec![BlockProps::default(); blocks.block_count()];
        let mut state_overrides = std::collections::HashMap::new();
        for (name, raw) in &file.blocks {
            let Some(idx) = blocks.block_index_by_name(name) else {
                // Unknown block in the dump (newer game?): keep the default, stay silent.
                continue;
            };
            props[idx as usize] = BlockProps {
                friction: raw.friction,
                speed_factor: raw.speed_factor,
                jump_factor: raw.jump_factor,
                destroy_time: raw.destroy_time,
                requires_correct_tool: raw.requires_correct_tool_for_drops,
            };
            for (id, st) in &raw.states {
                if let Ok(id) = id.parse::<u32>() {
                    state_overrides
                        .insert(id, (st.destroy_time, st.requires_correct_tool_for_drops));
                }
            }
        }
        Ok(Self {
            props,
            state_overrides,
        })
    }

    /// All-defaults table (used when the oracle file is absent).
    pub(crate) fn defaults(blocks: &BlockStates) -> Self {
        Self {
            props: vec![BlockProps::default(); blocks.block_count()],
            state_overrides: Default::default(),
        }
    }

    pub(crate) fn get_by_block(&self, block: u32) -> BlockProps {
        self.props.get(block as usize).copied().unwrap_or_default()
    }
}

impl GameData {
    /// Hardness of `state` (vanilla "destroy time"; -1 = unbreakable, 0 = instant). NaN when the
    /// oracle dump has no breaking data (regenerate with tools/recorder-mod/dump-server-oracles.sh).
    pub fn destroy_time(&self, state: BlockStateId) -> f32 {
        match self.physics.state_overrides.get(&state.0) {
            Some(&(d, _)) => d,
            None => self.props_of(state).destroy_time,
        }
    }

    /// Whether `state` drops anything only when broken with the correct tool.
    pub fn requires_correct_tool(&self, state: BlockStateId) -> bool {
        match self.physics.state_overrides.get(&state.0) {
            Some(&(_, c)) => c,
            None => self.props_of(state).requires_correct_tool,
        }
    }

    /// Friction/speed/jump of the block owning `state`.
    fn props_of(&self, state: BlockStateId) -> BlockProps {
        let block = self.blocks.block_index_of_state(state);
        self.physics.get_by_block(block)
    }

    /// Whether `state` is the vanilla block `minecraft:<path>`. A mod block reusing the path in
    /// another namespace is not it.
    fn is_minecraft_block(&self, state: BlockStateId, path: &str) -> bool {
        let block = self.blocks.block_of(state);
        block.namespace() == "minecraft" && block.path() == path
    }

    /// The fluid state's `FALLING` flag. Water's block state `level` is `getLegacyLevel`
    /// (`FlowingFluid.getLegacyLevel`): a source is 0, flowing 1..=7, falling 8..=15, so
    /// `level >= 8` is falling. Waterlogged blocks carry no `level` and are full sources.
    fn is_falling_fluid(&self, state: BlockStateId) -> bool {
        self.blocks
            .property_value(state, "level")
            .and_then(|l| l.parse::<u32>().ok())
            .is_some_and(|l| l >= 8)
    }

    /// Approximation of `BlockTags.BLOCKS_FLUID_FLOW`: a solid full block stops fluid flowing
    /// past it in `FlowingFluid.getFlow`. The tag itself is not loaded yet (see NOTES.md), so a
    /// full opaque cube stands in for it.
    fn blocks_fluid_flow(&self, state: BlockStateId) -> bool {
        self.shapes.is_opaque_full(state)
    }

    /// Approximation of `BlockState.isFaceSturdy` for `FlowingFluid.isSolidFace`: a full opaque
    /// cube is a solid face. The exact collision-shape test needs the face direction, which the
    /// callers in `getFlow` only use to decide the FALLING push.
    fn is_solid_face(&self, world: &dyn BlockView, pos: BlockPos) -> bool {
        self.shapes.is_opaque_full(world.block_state(pos))
    }
}

/// Vanilla `Vec3.normalize`: a vector shorter than 1e-4 becomes zero (glam's `normalize` returns
/// NaN there).
fn mc_normalize(v: DVec3) -> DVec3 {
    let len = v.length();
    if len < 1.0e-4 {
        DVec3::ZERO
    } else {
        v / len
    }
}

/// The horizontal plane in `Direction.Plane.HORIZONTAL` order (north, south, west, east).
const HORIZONTAL: [Direction; 4] = [
    Direction::North,
    Direction::South,
    Direction::West,
    Direction::East,
];

impl BlockPhysics for GameData {
    fn friction(&self, state: BlockStateId) -> f32 {
        self.props_of(state).friction
    }
    fn speed_factor(&self, state: BlockStateId) -> f32 {
        self.props_of(state).speed_factor
    }
    fn jump_factor(&self, state: BlockStateId) -> f32 {
        self.props_of(state).jump_factor
    }
    fn fluid(&self, state: BlockStateId) -> Option<FluidInfo> {
        self.shapes.fluid(state)
    }
    fn light_emission(&self, state: BlockStateId) -> u8 {
        self.shapes.light_emission(state)
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        self.shapes.is_air(state)
    }
    fn is_opaque_full(&self, state: BlockStateId) -> bool {
        self.shapes.is_opaque_full(state)
    }
    fn is_cobweb(&self, state: BlockStateId) -> bool {
        self.is_minecraft_block(state, "cobweb")
    }
    fn is_powder_snow(&self, state: BlockStateId) -> bool {
        self.is_minecraft_block(state, "powder_snow")
    }
    fn is_sweet_berry_bush(&self, state: BlockStateId) -> bool {
        self.is_minecraft_block(state, "sweet_berry_bush")
    }
    fn is_slime(&self, state: BlockStateId) -> bool {
        self.is_minecraft_block(state, "slime_block")
    }
    fn is_honey(&self, state: BlockStateId) -> bool {
        self.is_minecraft_block(state, "honey_block")
    }
    /// `Block.getBounceRestitution`: 1.0 for `minecraft:slime_block`, 0.0 otherwise. Vanilla
    /// 26.3 also gives the coloured beds and `minecraft:shelf_mushroom` 0.75
    /// (`Blocks.BED`/`Blocks.SHELF_MUSHROOM` `bounceRestitution`); those blocks are out of this
    /// task's scope (see the report), so they keep 0.0.
    fn bounce_restitution(&self, state: BlockStateId) -> f32 {
        if self.is_slime(state) {
            1.0
        } else {
            0.0
        }
    }
    /// `FluidState.getFlow` -> `FlowingFluid.getFlow(level, pos, fluidState)`: the normalised
    /// horizontal flow at the cell, from the fluid-height difference to its four horizontal
    /// neighbours (and the cell below an empty one), with the downward FALLING component against
    /// a solid face.
    fn water_flow(
        &self,
        world: &dyn BlockView,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Option<DVec3> {
        let own = self.shapes.fluid(state)?;
        let own_height = own.height as f64;
        let mut flow_x = 0.0_f64;
        let mut flow_z = 0.0_f64;
        for dir in HORIZONTAL {
            let o = dir.offset();
            let nb_pos = BlockPos::new(pos.x + o.x, pos.y, pos.z + o.z);
            let nb_state = world.block_state(nb_pos);
            let nb_fluid = self.shapes.fluid(nb_state);
            // `affectsFlow`: an empty neighbour or one of the same fluid type.
            if nb_fluid.is_some_and(|f| f.kind != own.kind) {
                continue;
            }
            let nb_height = nb_fluid.map_or(0.0, |f| f.height as f64);
            let distance = if nb_height == 0.0 {
                // No fluid of its own: look at the cell below it, unless the neighbour block
                // stops fluid flow (`BlockTags.BLOCKS_FLUID_FLOW`).
                if self.blocks_fluid_flow(nb_state) {
                    0.0
                } else {
                    let below_pos = BlockPos::new(nb_pos.x, nb_pos.y - 1, nb_pos.z);
                    match self.shapes.fluid(world.block_state(below_pos)) {
                        Some(f) if f.kind == own.kind && f.height > 0.0 => {
                            own_height - (f.height as f64 - 0.8888889)
                        }
                        _ => 0.0,
                    }
                }
            } else {
                own_height - nb_height
            };
            if distance != 0.0 {
                flow_x += o.x as f64 * distance;
                flow_z += o.z as f64 * distance;
            }
        }
        let mut flow = DVec3::new(flow_x, 0.0, flow_z);
        if self.is_falling_fluid(state) {
            for dir in HORIZONTAL {
                let o = dir.offset();
                let nb_pos = BlockPos::new(pos.x + o.x, pos.y, pos.z + o.z);
                let above = BlockPos::new(nb_pos.x, nb_pos.y + 1, nb_pos.z);
                if self.is_solid_face(world, nb_pos) || self.is_solid_face(world, above) {
                    flow = mc_normalize(flow) + DVec3::new(0.0, -6.0, 0.0);
                    break;
                }
            }
        }
        Some(mc_normalize(flow))
    }
}

impl BlockStateTable for BlockStates {
    fn len(&self) -> usize {
        BlockStates::len(self)
    }
    fn placeholder_state(&self) -> BlockStateId {
        BlockStates::placeholder_state(self)
    }
    fn block_of(&self, state: BlockStateId) -> &haste_core::Identifier {
        BlockStates::block_of(self, state)
    }
    fn properties(&self, state: BlockStateId) -> Vec<(String, String)> {
        BlockStates::properties(self, state)
    }
    fn property_value(&self, state: BlockStateId, name: &str) -> Option<&str> {
        BlockStates::property_value(self, state, name)
    }
}
