//! Prints a vanilla reference shot's JSON metadata (camera, size, scenario fields).
//! usage: shot-meta <shot.json>

use anyhow::{bail, Result};
use haste_testkit::screenshots::ScreenshotMeta;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 1 {
        bail!("usage: shot-meta <shot.json>");
    }
    let meta: ScreenshotMeta = serde_json::from_reader(
        std::fs::File::open(Path::new(&args[0])).expect("opening the json"),
    )
    .expect("parsing the json");
    println!("{meta:#?}");
    Ok(())
}
