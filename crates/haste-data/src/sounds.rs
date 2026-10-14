//! Block sound types from the oracle dump `block_sounds.json`
//! (tools/recorder-mod, `dump_block_sounds` scenario): the five sound events
//! (break/step/place/hit/fall) plus volume and pitch per block, with per-state
//! overrides where a state differs from its block's default.

use std::collections::HashMap;

use haste_core::BlockStateId;

use crate::blocks::BlockStates;
use crate::GameData;

/// One sound type: the five sound event ids and vanilla's volume/pitch
/// multipliers for them (identical `SoundType`s are deduped in the dump and
/// share one index).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockSoundType {
    /// Sound event ids, namespaced (`minecraft:block.stone.break`); vanilla's
    /// "no sound" event is `minecraft:intentionally_empty`.
    pub break_sound: String,
    pub step_sound: String,
    pub place_sound: String,
    pub hit_sound: String,
    pub fall_sound: String,
    pub volume: f32,
    pub pitch: f32,
}

/// Block sound types loaded from `block_sounds.json`, indexed for
/// per-state lookup like `BlockProperties`.
#[derive(Default)]
pub struct BlockSoundTypes {
    /// One index per block (parallels `BlockStates`' block table); `u32::MAX`
    /// = not in the dump.
    block_type: Vec<u32>,
    /// State id -> type index, for the states that differ from their block's
    /// default.
    state_overrides: HashMap<u32, u32>,
    types: Vec<BlockSoundType>,
}

#[derive(serde::Deserialize)]
struct SoundsFile {
    sound_types: Vec<RawSoundType>,
    blocks: HashMap<String, RawBlock>,
}

#[derive(serde::Deserialize)]
struct RawSoundType {
    #[serde(rename = "break")]
    break_sound: String,
    step: String,
    place: String,
    hit: String,
    fall: String,
    volume: f32,
    pitch: f32,
}

#[derive(serde::Deserialize)]
struct RawBlock {
    sound: u32,
    #[serde(default)]
    states: Vec<RawState>,
}

#[derive(serde::Deserialize)]
struct RawState {
    sound: u32,
    properties: HashMap<String, serde_json::Value>,
}

impl BlockSoundTypes {
    /// Parses `block_sounds.json` (FORMATS.md). `blocks` provides the block
    /// order to index by; unknown blocks in the dump are skipped (a newer
    /// dump against older reports must not fail the load).
    pub(crate) fn parse(json: &str, blocks: &BlockStates) -> anyhow::Result<Self> {
        let file: SoundsFile =
            serde_json::from_str(json).map_err(|e| anyhow::anyhow!("block_sounds.json: {e}"))?;
        let types: Vec<BlockSoundType> = file
            .sound_types
            .iter()
            .map(|t| BlockSoundType {
                break_sound: t.break_sound.clone(),
                step_sound: t.step.clone(),
                place_sound: t.place.clone(),
                hit_sound: t.hit.clone(),
                fall_sound: t.fall.clone(),
                volume: t.volume,
                pitch: t.pitch,
            })
            .collect();
        let mut block_type = vec![u32::MAX; blocks.block_count()];
        let mut state_overrides = HashMap::new();
        for (name, raw) in &file.blocks {
            let Some(idx) = blocks.block_index_by_name(name) else {
                continue;
            };
            block_type[idx as usize] = raw.sound;
            for st in &raw.states {
                // The dump keys a state by its properties; resolve to a state
                // id through BlockStates' full-property lookup.
                let props: Vec<(&str, &str)> = st
                    .properties
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), s)))
                    .collect();
                if let Some(id) = blocks.state(&parse_ident(name), &props) {
                    state_overrides.insert(id.0, st.sound);
                }
            }
        }
        Ok(Self {
            block_type,
            state_overrides,
            types,
        })
    }

    /// Empty table (used when the oracle file is absent): every lookup
    /// returns `None`.
    pub(crate) fn empty(blocks: &BlockStates) -> Self {
        Self {
            block_type: vec![u32::MAX; blocks.block_count()],
            state_overrides: HashMap::new(),
            types: Vec::new(),
        }
    }

    /// Whether the dump was loaded.
    pub fn is_loaded(&self) -> bool {
        !self.types.is_empty()
    }
}

/// The dump's block keys are plain `minecraft:stone` strings; `BlockStates::state`
/// wants an `Identifier`.
fn parse_ident(name: &str) -> haste_core::Identifier {
    haste_core::Identifier::parse(name).unwrap_or_else(|_| haste_core::Identifier::minecraft("air"))
}

impl GameData {
    /// The sound type of `state`: per-state override first, then the block's
    /// default. `None` when the dump is absent or the block is not in it
    /// (mods play nothing, per the brief).
    pub fn block_sound_type(&self, state: BlockStateId) -> Option<&BlockSoundType> {
        let index = match self.sounds.state_overrides.get(&state.0) {
            Some(&i) => i,
            None => {
                let block = self.blocks.block_index_of_state(state);
                *self.sounds.block_type.get(block as usize)?
            }
        };
        self.sounds.types.get(index as usize)
    }
}
