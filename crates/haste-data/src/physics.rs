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
}

impl Default for BlockProps {
    fn default() -> Self {
        Self {
            friction: 0.6,
            speed_factor: 1.0,
            jump_factor: 1.0,
        }
    }
}

/// Block-name -> BlockProps table loaded from the oracle dump.
pub struct BlockProperties {
    /// Index parallels `BlockStates`' block descriptors are NOT name-ordered; we look up by
    /// block index into `BlockStates` via a side table built at load.
    props: Vec<BlockProps>,
}

#[derive(serde::Deserialize)]
struct PropsFile {
    blocks: std::collections::HashMap<String, RawProps>,
}

#[derive(serde::Deserialize, Clone, Copy)]
struct RawProps {
    #[serde(default = "d_friction")]
    friction: f32,
    #[serde(default = "one")]
    speed_factor: f32,
    #[serde(default = "one")]
    jump_factor: f32,
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
        for (name, raw) in &file.blocks {
            let Some(idx) = blocks.block_index_by_name(name) else {
                // Unknown block in the dump (newer game?): keep the default, stay silent.
                continue;
            };
            props[idx as usize] = BlockProps {
                friction: raw.friction,
                speed_factor: raw.speed_factor,
                jump_factor: raw.jump_factor,
            };
        }
        Ok(Self { props })
    }

    /// All-defaults table (used when the oracle file is absent).
    pub(crate) fn defaults(blocks: &BlockStates) -> Self {
        Self {
            props: vec![BlockProps::default(); blocks.block_count()],
        }
    }

    pub(crate) fn get_by_block(&self, block: u32) -> BlockProps {
        self.props.get(block as usize).copied().unwrap_or_default()
    }
}

impl GameData {
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
