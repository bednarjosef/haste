//! `BlockBehaviour.skipRendering`: the per-block face-skip rules of vanilla's
//! `Block.shouldRenderFace` (its second step, between the full-opaque occluder
//! check and the shape comparison; the shape comparison is `culling.rs`).
//!
//! Vanilla asks the block's Java class before comparing shapes: glass against
//! glass, ice against ice, powder snow against powder snow hide the shared
//! face (`HalfTransparentBlock`/`PowderSnowBlock`), connected bars and panes
//! hide theirs (`IronBarsBlock`), and so on. Haste's mesher had only the shape
//! comparison, so a glass wall drew every interior face — visible as inner
//! edges through the glass.
//!
//! Which rule a block uses depends on its Java class, so the rules come from
//! the recorder mod's `block_classes.json` dump (see `tools/recorder-mod/
//! BlockClassDump.java` and haste-testkit's FORMATS.md), not a hand-written
//! block list — mods add blocks, and a block the dump does not know uses the
//! default (no skip). The dump is resolved once per state at load into flat
//! per-state arrays; the mesh-time lookup is a byte read plus rule-specific
//! comparisons, with no allocation.

use std::collections::HashMap;

use haste_assets::Direction;
use haste_core::view::BlockStateTable;
use haste_core::BlockStateId;

#[cfg(test)]
mod tests;

/// The rule one block's states follow, stored as one byte per state.
/// Each variant names the vanilla class its logic follows; see `skips`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum SkipRule {
    /// `BlockBehaviour.skipRendering`: never skip.
    None = 0,
    /// `HalfTransparentBlock` and `PowderSnowBlock`: skip when the neighbour
    /// is the same block.
    SameBlock,
    /// `IronBarsBlock`: skip against the same block or another bars block.
    Bars,
    /// `LeavesBlock`: skip against leaves when leaves are not cutout ("fast"
    /// graphics).
    Leaves,
    /// `MangroveRootsBlock`: skip against `minecraft:mangrove_roots` on the
    /// vertical axis.
    MangroveRoots,
}

/// The `block_classes.json` dump: per block id its Java class chain (simple
/// names, the concrete class first, `Block` last).
pub struct BlockClassDump {
    blocks: HashMap<String, Vec<String>>,
}

/// The dump's rule for one class chain: the vanilla override the chain's
/// classes carry (single inheritance, so at most one matches). `LiquidBlock`'s
/// override is deliberately not mapped: its states are `RenderShape.INVISIBLE`
/// (never model-meshed), so in vanilla the rule cannot run — equal fluids are
/// culled by the fluid mesher (`fluid.rs`, `isNeighborSameFluid`).
fn rule_of_chain(chain: &[String]) -> SkipRule {
    for name in chain {
        match name.as_str() {
            "HalfTransparentBlock" | "PowderSnowBlock" => return SkipRule::SameBlock,
            "IronBarsBlock" => return SkipRule::Bars,
            "LeavesBlock" => return SkipRule::Leaves,
            "MangroveRootsBlock" => return SkipRule::MangroveRoots,
            _ => {}
        }
    }
    SkipRule::None
}

impl BlockClassDump {
    /// Parses the dump's JSON (version 1; unknown versions are an error).
    pub fn parse(json: &str) -> anyhow::Result<BlockClassDump> {
        #[derive(serde::Deserialize)]
        struct File {
            version: u32,
            #[serde(default)]
            #[allow(dead_code)] // written for traceability; not needed once loaded
            minecraft: Option<String>,
            blocks: HashMap<String, Vec<String>>,
        }
        let file: File = serde_json::from_str(json)?;
        if file.version != 1 {
            anyhow::bail!("block_classes.json: unsupported version {}", file.version);
        }
        Ok(BlockClassDump {
            blocks: file.blocks,
        })
    }

    /// The dump from the cache's oracle dir, read and parsed once per process
    /// (the oracle dir never changes while the client runs). `None` when the
    /// file is absent (the dump script has not run) or unreadable — warned
    /// once, never fatal: the mesher then draws every face like before.
    fn cached() -> Option<&'static BlockClassDump> {
        static CACHE: std::sync::OnceLock<Option<BlockClassDump>> = std::sync::OnceLock::new();
        CACHE
            .get_or_init(|| {
                let path = haste_core::paths::oracle_dir().join("block_classes.json");
                match std::fs::read_to_string(&path) {
                    Ok(json) => match BlockClassDump::parse(&json) {
                        Ok(dump) => Some(dump),
                        Err(e) => {
                            tracing::warn!(
                                "block_classes.json unreadable ({e:#}): no face-skip rules"
                            );
                            None
                        }
                    },
                    Err(_) => None,
                }
            })
            .as_ref()
    }
}

/// Per-state face-skip data (vanilla `BlockBehaviour.skipRendering`), resolved
/// once from the class dump: one rule byte and one block-identity key per
/// state, plus the horizontal connection bits `IronBarsBlock`'s rule reads.
/// The same state never changes rule (the dump describes blocks, not states),
/// so the mesh path is a few array reads with no allocation.
#[derive(Clone)]
pub struct SkipTable {
    /// Rule per state (`SkipRule` as a byte).
    rule: Vec<SkipRule>,
    /// Per state, a dense key of the owning block (the `is(this)` identity of
    /// vanilla's same-block checks; equal keys = same block).
    block: Vec<u32>,
    /// Per state, the north/east/south/west connection property bits
    /// (1/2/4/8), for `IronBarsBlock`'s horizontal rule.
    conn: Vec<u8>,
    /// Block key of `minecraft:mangrove_roots` (vanilla's
    /// `MangroveRootsBlock` names that one block by id, so we do too).
    mangrove_roots: u32,
    /// Vanilla `Options.cutoutLeaves`: leaves skip only when leaves are NOT
    /// cutout. Haste draws cutout (fancy) leaves like vanilla's default.
    cutout_leaves: bool,
}

impl SkipTable {
    /// No rules: every face renders (no dump, or the data is not loaded yet).
    pub fn absent() -> SkipTable {
        SkipTable {
            rule: Vec::new(),
            block: Vec::new(),
            conn: Vec::new(),
            mangrove_roots: u32::MAX,
            cutout_leaves: true,
        }
    }

    /// The cache's dump resolved per state; a dump missing a block leaves
    /// that block's states at the default (no skip), and out-of-range state
    /// ids (modded servers) read as no rule in `skips`.
    pub fn from_dump(states: &dyn BlockStateTable, dump: &BlockClassDump) -> SkipTable {
        let n = states.len();
        let mut rule = vec![SkipRule::None; n];
        let mut block = vec![u32::MAX; n];
        let mut conn = vec![0u8; n];
        let mut keys: HashMap<String, u32> = HashMap::new();
        let mut mangrove_roots = u32::MAX;
        for id in 0..n as u32 {
            let state = BlockStateId(id);
            let name = states.block_of(state).to_string();
            let key = match keys.get(&name) {
                Some(&k) => k,
                None => {
                    let k = keys.len() as u32;
                    keys.insert(name.clone(), k);
                    if name == "minecraft:mangrove_roots" {
                        mangrove_roots = k;
                    }
                    k
                }
            };
            block[id as usize] = key;
            rule[id as usize] = dump
                .blocks
                .get(&name)
                .map(|chain| rule_of_chain(chain))
                .unwrap_or(SkipRule::None);
            for (bit, prop) in [(1, "north"), (2, "east"), (4, "south"), (8, "west")] {
                if states.property_value(state, prop) == Some("true") {
                    conn[id as usize] |= bit;
                }
            }
        }
        SkipTable {
            rule,
            block,
            conn,
            mangrove_roots,
            cutout_leaves: true,
        }
    }

    /// The cache's dump (`BlockClassDump::cached`) resolved per state, or
    /// [`SkipTable::absent`] when there is no dump.
    pub fn load(states: &dyn BlockStateTable) -> SkipTable {
        match BlockClassDump::cached() {
            Some(dump) => SkipTable::from_dump(states, dump),
            None => SkipTable::absent(),
        }
    }

    /// Whether `state`'s face towards `dir` (its cullface) is hidden by
    /// `neighbor` through the state's `skipRendering` rule — vanilla's
    /// `state.skipRendering(neighborState, direction)` in
    /// `Block.shouldRenderFace`. Unknown state ids never skip.
    #[inline]
    pub fn skips(&self, state: BlockStateId, neighbor: BlockStateId, dir: Direction) -> bool {
        let s = state.0 as usize;
        if s >= self.rule.len() {
            return false;
        }
        let n = neighbor.0 as usize;
        let neighbor_rule = self.rule.get(n).copied().unwrap_or(SkipRule::None);
        match self.rule[s] {
            // HalfTransparentBlock / PowderSnowBlock: `neighborState.is(this)`.
            SkipRule::SameBlock => self.block.get(n) == Some(&self.block[s]),
            // IronBarsBlock: the same block or another bars block (vanilla's
            // #minecraft:bars tag check, approximated by the class the dump
            // names — every vanilla bars block is an IronBarsBlock); the
            // vertical faces always skip, a horizontal face only when this
            // state connects towards the neighbour and the neighbour connects
            // back.
            SkipRule::Bars => {
                if self.block.get(n) != Some(&self.block[s]) && neighbor_rule != SkipRule::Bars {
                    return false;
                }
                if matches!(dir, Direction::Up | Direction::Down) {
                    return true;
                }
                conn_bit(self.conn.get(n).copied().unwrap_or(0), dir.opposite())
                    && conn_bit(self.conn[s], dir)
            }
            // LeavesBlock: `!cutoutLeaves && neighbour instanceof LeavesBlock`.
            SkipRule::Leaves => !self.cutout_leaves && neighbor_rule == SkipRule::Leaves,
            // MangroveRootsBlock: neighbour is minecraft:mangrove_roots, on
            // the vertical axis only.
            SkipRule::MangroveRoots => {
                matches!(dir, Direction::Up | Direction::Down)
                    && self.block.get(n) == Some(&self.mangrove_roots)
            }
            SkipRule::None => false,
        }
    }
}

/// The connection bit of `dir`'s property in `bits` (0 = not connected or a
/// vertical direction, which the bars rule never reads a property for).
fn conn_bit(bits: u8, dir: Direction) -> bool {
    let bit = match dir {
        Direction::North => 1,
        Direction::East => 2,
        Direction::South => 4,
        Direction::West => 8,
        _ => return false,
    };
    bits & bit != 0
}
