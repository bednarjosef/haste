//! Dev screenshot helper for the resource-pack branch: takes one offscreen
//! shot of the snowy plain with a generated pack that turns snow red, proving
//! `Assets::apply_packs` layers pack textures into the world atlas.
//!
//! Uses the shared test server on 127.0.0.1:25691 (never starts one) and
//! `target/debug/haste`. Ignored unless run by name (needs that server and the
//! client jar cache).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate inside the repo")
        .to_path_buf()
}

/// Builds `target/redsnow-packs/redsnow`: a pack overriding
/// `minecraft:block/snow` with a solid red tile (pack format 97, 26.3's
/// resource pack format), in a folder named like the pack id expects.
fn write_red_snow_pack(root: &std::path::Path) {
    let dir = root.join("target/redsnow-packs/redsnow/assets/minecraft/textures/block");
    std::fs::create_dir_all(&dir).expect("create pack dirs");
    std::fs::write(
        root.join("target/redsnow-packs/redsnow/pack.mcmeta"),
        r#"{"pack":{"pack_format":97,"description":"red snow test pack"}}"#,
    )
    .expect("write pack.mcmeta");
    let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([220, 40, 40, 255]));
    img.save(dir.join("snow.png")).expect("write snow.png");
}

#[test]
#[ignore = "needs the local test server on 25691 (dev screenshot helper)"]
fn screenshot_with_red_snow_pack() {
    let root = repo_root();
    let shot = root.join("target/redsnow-shot.png");
    write_red_snow_pack(&root);

    // A no-pack baseline of the same camera: time of day differs between
    // runs, so the pack's effect is measured against this, not a fixed hue.
    take_shot(&root, root.join("target/plain-shot.png"), false);

    take_shot(&root, shot.clone(), true);

    // The pack's snow is solid (220, 40, 40): the snowy ground in the lower
    // half must be far redder with the pack than the same shot without it.
    let pack_red = red_pixels(&shot);
    let plain_red = red_pixels(&root.join("target/plain-shot.png"));
    println!("snow check: {pack_red} red lower-half pixels with the pack vs {plain_red} without");
    assert!(
        pack_red > plain_red * 3 + 10_000,
        "the red snow pack should dominate the ground ({pack_red} vs {plain_red})"
    );
}

/// Takes one offscreen screenshot at the snowy-plain camera, with the red snow
/// pack applied (`with_pack`) or plain vanilla.
fn take_shot(root: &std::path::Path, shot: std::path::PathBuf, with_pack: bool) {
    let mut cmd = Command::new(root.join("target/debug/haste"));
    cmd.args([
        "--server",
        "127.0.0.1:25691",
        "--username",
        "worker-redsnow",
        "--screenshot",
        shot.to_str().unwrap(),
        "--hud",
        "off",
        "--pos",
        "9.5",
        "71",
        "6.5",
        "--yaw",
        "0",
        "--pitch",
        "10",
        "--width",
        "960",
        "--height",
        "540",
        "--render-distance",
        "8",
        "--seconds",
        "40",
    ])
    .current_dir(root)
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit());
    if with_pack {
        cmd.env("HASTE_RESOURCE_PACKS", "file/redsnow").env(
            "HASTE_RESOURCE_PACKS_DIR",
            root.join("target/redsnow-packs"),
        );
    }
    let mut client = cmd.spawn().expect("start the screenshot client");
    let (summary_tx, summary) = mpsc::channel();
    if let Some(out) = client.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if line.contains("screenshot ") {
                    let _ = summary_tx.send(line.clone());
                }
            }
        });
    }
    let status = client.wait().expect("client exit");
    println!("client exit ({with_pack}): {status:?}");
    let mut saved = false;
    while let Ok(line) = summary.try_recv() {
        println!("SUMMARY: {line}");
        saved |= true;
    }
    assert!(
        saved,
        "client should log the saved screenshot ({with_pack})"
    );
    assert!(shot.is_file(), "screenshot file should exist ({with_pack})");
}

/// Count of strongly red-dominant pixels in the lower half of an image.
fn red_pixels(path: &std::path::Path) -> usize {
    let img = image::open(path).expect("open the screenshot");
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut red = 0usize;
    for y in (h / 2)..h {
        for x in 0..w {
            let p = rgba.get_pixel(x, y);
            if p[0] as i32 - p[1] as i32 > 60 && p[0] as i32 - p[2] as i32 > 60 {
                red += 1;
            }
        }
    }
    red
}
