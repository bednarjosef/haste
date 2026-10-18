//! The block side of vanilla's block entity types, from the oracle dump `block_entity_types.json`
//! (tools/recorder-mod, `BlockEntityTypeDump`): which block carries which
//! `minecraft:block_entity_type`, as `EntityBlock.newBlockEntity` returned it for every state
//! (the call vanilla `LevelChunk.setBlockState` makes when a block update places the block).

use std::collections::HashMap;

use haste_core::{BlockStateId, Identifier};

/// Block -> block entity type pairs from the recorder mod's dump. One lookup, resolved by the
/// client into per-state ids at join; `empty` (no dump) means block updates create no block
/// entities (the server's `block_entity_data` still works).
#[derive(Default)]
pub struct BlockEntityTypes {
    /// Block id -> its block entity type id; the states of one block share the type.
    blocks: HashMap<Identifier, Identifier>,
    /// State id -> type id, for the blocks whose states disagree or return no entity for some
    /// of them (none in vanilla 26.3; kept so a mod whose `newBlockEntity` varies survives).
    states: HashMap<u32, Identifier>,
}

/// The raw dump: string-keyed because identifiers parse (and fail) with context.
#[derive(serde::Deserialize)]
struct TypesFile {
    version: u32,
    blocks: HashMap<String, String>,
    #[serde(default)]
    states: HashMap<String, String>,
}

impl BlockEntityTypes {
    /// Parses `block_entity_types.json` (FORMATS.md).
    pub(crate) fn parse(json: &str) -> anyhow::Result<Self> {
        let file: TypesFile = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("block_entity_types.json: {e}"))?;
        if file.version != 1 {
            anyhow::bail!(
                "block_entity_types.json: unsupported version {}",
                file.version
            );
        }
        let mut blocks = HashMap::with_capacity(file.blocks.len());
        for (block, ty) in file.blocks {
            blocks.insert(id(&block, "block")?, id(&ty, "type")?);
        }
        let mut states = HashMap::with_capacity(file.states.len());
        for (state, ty) in file.states {
            let state: u32 = state
                .parse()
                .map_err(|e| anyhow::anyhow!("block_entity_types.json: state id {state:?}: {e}"))?;
            states.insert(state, id(&ty, "type")?);
        }
        Ok(Self { blocks, states })
    }

    /// The empty table: no dump was found, so no block carries an entity.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether a dump backs this table.
    pub fn is_loaded(&self) -> bool {
        !self.blocks.is_empty() || !self.states.is_empty()
    }

    /// The block entity type a state carries: the state's own entry first, then its block's
    /// (the dump's authority; a block or state it does not know carries none).
    pub fn type_of(&self, block: &Identifier, state: BlockStateId) -> Option<&Identifier> {
        self.states.get(&state.0).or_else(|| self.blocks.get(block))
    }
}

/// Parses one identifier from the dump, naming what was wrong when it does not.
fn id(s: &str, what: &str) -> anyhow::Result<Identifier> {
    Identifier::parse(s).map_err(|e| anyhow::anyhow!("block_entity_types.json: {what}: {e}"))
}
