//! Renders a Markdown summary table for bench recordings in a directory.
//!
//! Usage: `cargo run -p haste-testkit --example bench-report -- <dir>` (default
//! `../cache/oracle/26.3/bench`). Reads every `*.jsonl` bench file and prints one
//! Markdown table (name, frames, fps, mean/p99 frame ms, max RSS, draw calls, sections).

use std::path::PathBuf;

use haste_testkit::bench::Bench;
use haste_testkit::report::bench_table;

fn main() -> anyhow::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../cache/oracle/26.3/bench"));
    if !dir.is_dir() {
        anyhow::bail!(
            "no bench dir at {} (record one with /haste bench, see docs/RECORDING.md)",
            dir.display()
        );
    }
    let mut paths: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        anyhow::bail!("no *.jsonl bench files in {}", dir.display());
    }
    let benches: Vec<Bench> = paths
        .iter()
        .map(|p| Bench::load(p))
        .collect::<Result<_, _>>()?;
    let refs: Vec<&Bench> = benches.iter().collect();
    println!("{}", bench_table(&refs));
    Ok(())
}
