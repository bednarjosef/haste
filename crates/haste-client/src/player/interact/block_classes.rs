//! The `block_classes.json` dump for placement prediction (player workstream): per block
//! id its Java class chain (simple names, the concrete class first, `Block` last), as the
//! recorder mod dumps it (see `tools/recorder-mod/BlockClassDump.java` and haste-testkit's
//! FORMATS.md). Placement prediction needs it to know which vanilla
//! `getStateForPlacement` a block follows — the rules come from the dump, not a
//! hand-written block list, and a block the dump does not know is not predicted.
//!
//! The dump is read once per process from the cache's oracle dir
//! (`haste_core::paths::oracle_dir`); a missing or unreadable dump (the dump script has
//! not run) predicts nothing placement-wise, warned once, never fatal — breaking
//! prediction does not need it and keeps working.

use std::collections::HashMap;
use std::sync::OnceLock;

/// A block's Java class chain, resolved to the placement rule it follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlacementClass {
    /// Class `Block` itself: `getStateForPlacement` returns the default state.
    Block,
    /// `RotatedPillarBlock`: the axis comes from the clicked face.
    RotatedPillarBlock,
    /// `SlabBlock`: bottom/top by the click, double into a same-kind slab.
    SlabBlock,
    /// `StairBlock`: facing from the player, half by the click.
    StairBlock,
}

/// The `block_classes.json` dump: per block name its Java class chain. The same format
/// haste-render's mesher reads (`mesher/skip.rs`); parsed here so the interaction side
/// does not reach into the renderer's crate.
pub(crate) struct BlockClassDump {
    blocks: HashMap<String, Vec<String>>,
}

impl BlockClassDump {
    fn parse(json: &str) -> anyhow::Result<BlockClassDump> {
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

    /// The dump from the cache's oracle dir, read and parsed once per process. `None` when
    /// the file is absent (the dump script has not run) or unreadable — warned once, never
    /// fatal: nothing placement-side is predicted then, like before this dump existed.
    fn cached() -> Option<&'static BlockClassDump> {
        static CACHE: OnceLock<Option<BlockClassDump>> = OnceLock::new();
        CACHE
            .get_or_init(|| {
                let path = haste_core::paths::oracle_dir().join("block_classes.json");
                match std::fs::read_to_string(&path) {
                    Ok(json) => match BlockClassDump::parse(&json) {
                        Ok(dump) => Some(dump),
                        Err(e) => {
                            tracing::warn!(
                                "block_classes.json unreadable ({e:#}): no placement prediction"
                            );
                            None
                        }
                    },
                    Err(_) => None,
                }
            })
            .as_ref()
    }

    /// The placement rule a block follows, `None` when the dump does not know the block
    /// (a mod's block, or no dump): never predicted, the server's `block_update` shows it.
    pub(crate) fn placement_class(&self, block: &str) -> Option<PlacementClass> {
        // Single inheritance: at most one class of the chain decides.
        let chain = self.blocks.get(block)?;
        let mut has_block = false;
        for name in chain {
            match name.as_str() {
                "Block" => has_block = true,
                "RotatedPillarBlock" => return Some(PlacementClass::RotatedPillarBlock),
                "SlabBlock" => return Some(PlacementClass::SlabBlock),
                "StairBlock" => return Some(PlacementClass::StairBlock),
                // Any other class: its `getStateForPlacement` is its own (or inherits
                // `Block`'s, which we cannot tell apart from an override) — not covered.
                _ => return None,
            }
        }
        // A chain of exactly `Block` (most simple blocks: stone, planks, glass).
        has_block.then_some(PlacementClass::Block)
    }

    /// The concrete class of `block`'s chain (the first entry), for the once-per-class
    /// debug note when a block is not predicted because its class is not covered.
    pub(crate) fn concrete_class(&self, block: &str) -> Option<&str> {
        self.blocks
            .get(block)
            .and_then(|c| c.first())
            .map(String::as_str)
    }
}

/// The placement class of `block` from the cached dump (`None` without a dump or for an
/// unknown block). `Interaction` owns no dump state: the dump is per-process.
pub(crate) fn placement_class(block: &str) -> Option<PlacementClass> {
    BlockClassDump::cached()?.placement_class(block)
}

/// Logs the concrete class of a block whose placement state is not predicted, once per
/// class (a second block of the same class does not log again).
pub(crate) fn note_uncovered_class(block: &str) {
    static SEEN: OnceLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> =
        OnceLock::new();
    let Some(dump) = BlockClassDump::cached() else {
        return;
    };
    let Some(class) = dump.concrete_class(block) else {
        return;
    };
    let seen = SEEN.get_or_init(Default::default);
    let mut guard = seen.lock().unwrap_or_else(|e| e.into_inner());
    if guard.insert(class) {
        tracing::debug!("placement of blocks of class {class} is not predicted ({block})");
    }
}
