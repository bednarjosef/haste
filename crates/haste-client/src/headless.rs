//! M1: headless join. Owner: WS1 (uses haste-world and haste-entity through their public APIs).
use crate::Args;

pub fn run(_args: &Args) -> anyhow::Result<()> {
    anyhow::bail!("headless join is WS1's first deliverable (TASKS.md M1)")
}
