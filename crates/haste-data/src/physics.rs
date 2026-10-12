//! Block physics properties from the oracle dump `block_properties.json`
//! (friction/speed/jump per block) and the `BlockPhysics`/`BlockStateTable` impls for
//! `GameData`/`BlockStates`.

use haste_core::view::{BlockPhysics, BlockStateTable, FluidInfo};
use haste_core::BlockStateId;

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
}

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
}

impl BlockStateTable for BlockStates {
    fn len(&self) -> usize {
        BlockStates::len(self)
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
