use super::textures::{animation_from_raw, AnimationMetaRaw, TextureMetaFile};
use super::*;
use haste_core::Identifier;

fn open_assets() -> Option<Assets> {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        return None;
    }
    let assets_dir = haste_core::paths::cache_dir().join("assets");
    Assets::open(&jar, &assets_dir, "26.3").ok()
}

fn jar_test(name: &str, f: impl FnOnce(&mut Assets)) {
    let mut assets = match open_assets() {
        Some(a) => a,
        None => {
            eprintln!("ignored: {name} needs the client jar in the cache");
            return;
        }
    };
    f(&mut assets);
}

#[test]
fn asset_keys() {
    let id = Identifier::parse("minecraft:block/stone").unwrap();
    assert_eq!(
        asset_key(&id, "blockstates", ".json"),
        "assets/minecraft/blockstates/block/stone.json"
    );
    let id = Identifier::parse("stone").unwrap();
    assert_eq!(
        asset_key(&id, "textures", ".png"),
        "assets/minecraft/textures/stone.png"
    );
}

/// A tiny generated pack (folder form) overriding `block/stone.png` with a
/// red tile plus a new texture, layered under a zip pack adding another.
fn pack_fixture(assets_dir: &std::path::Path) -> Vec<String> {
    use std::io::Write as _;
    let packs_dir = assets_dir.join("resourcepacks");
    let red = packs_dir.join("RedStone");
    std::fs::create_dir_all(red.join("assets/minecraft/textures/block")).unwrap();
    std::fs::write(
        red.join("pack.mcmeta"),
        r#"{"pack":{"min_format":97,"max_format":97,"description":"red stone"}}"#,
    )
    .unwrap();
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16,
        16,
        image::Rgba([200, 30, 30, 255]),
    ))
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();
    std::fs::write(red.join("assets/minecraft/textures/block/stone.png"), &png).unwrap();
    let zip_path = packs_dir.join("Extra.zip");
    {
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(
            std::fs::File::create(&zip_path).unwrap(),
        ));
        zip.start_file::<_, ()>("pack.mcmeta", Default::default())
            .unwrap();
        zip.write_all(br#"{"pack":{"min_format":97,"max_format":97,"description":"extra"}}"#)
            .unwrap();
        zip.start_file::<_, ()>(
            "assets/minecraft/textures/block/haste_extra.png",
            Default::default(),
        )
        .unwrap();
        zip.write_all(&png).unwrap();
        zip.finish().unwrap();
    }
    vec!["file/RedStone".to_string(), "file/Extra.zip".to_string()]
}

#[test]
#[ignore = "needs the client jar in the cache"]
fn pack_overrides_and_extends_jar() {
    let mut assets = match open_assets() {
        Some(a) => a,
        None => {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
    };
    let packs = pack_fixture(&haste_core::paths::cache_dir().join("assets"));
    let packs_dir = haste_core::paths::cache_dir()
        .join("assets")
        .join("resourcepacks");
    assets.apply_packs(&packs_dir, &packs);
    // The pack's red stone replaces the jar's.
    let stone = assets
        .texture(&Identifier::minecraft("block/stone"))
        .unwrap();
    assert_eq!(stone.get_pixel(0, 0), &image::Rgba([200, 30, 30, 255]));
    // A new texture from the zip pack is readable (and listable).
    let extra = assets
        .texture(&Identifier::minecraft("block/haste_extra"))
        .unwrap();
    assert_eq!(extra.dimensions(), (16, 16));
    let names = assets.block_texture_names();
    assert!(names.iter().any(|n| n.path() == "block/haste_extra"));
    // Untouched assets still come from the jar.
    let dirt = assets
        .texture(&Identifier::minecraft("block/dirt"))
        .unwrap();
    assert_eq!(dirt.dimensions(), (16, 16));
    // reopen re-applies the same pack stack.
    let mut reopened = assets.reopen().unwrap();
    assert_eq!(reopened.pack_ids(), packs.as_slice());
    assert_eq!(reopened.packs_dir(), packs_dir);
    let stone = reopened
        .texture(&Identifier::minecraft("block/stone"))
        .unwrap();
    assert_eq!(stone.get_pixel(0, 0), &image::Rgba([200, 30, 30, 255]));
}

#[test]
#[ignore = "needs the client jar in the cache"]
fn jar_texture_and_blockstate() {
    jar_test("texture", |a| {
        let img = a.texture(&Identifier::minecraft("block/stone")).unwrap();
        assert_eq!(img.dimensions(), (16, 16));
        let bs = a.blockstate(&Identifier::minecraft("stone")).unwrap();
        assert!(bs.variants.is_some());
    });
}

#[test]
#[ignore = "needs the client jar in the cache"]
fn jar_font_and_language() {
    jar_test("font", |a| {
        let font = a.font_default().unwrap();
        assert!(font.defined_glyph('A').is_some());
        assert_eq!(font.char_width(' '), Some(4));
        assert!(font.char_width('W').unwrap() > font.char_width('i').unwrap_or(0));
        let en = a.language("en_us").unwrap();
        assert!(en.contains_key("block.minecraft.stone"));
        use haste_core::gui::TextMetrics;
        // Vanilla widths: glyph columns + 1 spacing column.
        assert_eq!(font.width("i"), 2);
        assert_eq!(font.width("l"), 3);
        assert_eq!(font.width("A"), 6);
        assert_eq!(font.width("W"), 6);
        assert_eq!(font.width(" "), 4);
        assert_eq!(font.width("Hello World"), 55); // H6 e6 l3 l3 o6 _4 W6 o6 r6 l3 d6
        assert!(
            font.len() > 10_000,
            "unifont not loaded from the assets index"
        );
        // Accented page (9x12 cells, height 12): measured in its own cell size.
        assert_eq!(font.width("É"), 6);
        // Unifont from the assets index (CJK: 16 columns at half scale + 1).
        if font.len() > 10_000 {
            assert_eq!(font.width("中"), 9);
        }
    });
}

#[test]
#[ignore = "needs the client jar in the cache"]
fn jar_gui_sprite_nine_slice() {
    jar_test("gui_sprite", |a| {
        let (img, meta) = a.gui_sprite(&Identifier::minecraft("hud/hotbar")).unwrap();
        assert!(img.dimensions().0 > 0);
        assert_eq!(meta, SpriteMeta::Stretch);
        let (_, meta) = a
            .gui_sprite(&Identifier::minecraft("advancements/title_box"))
            .unwrap();
        assert_eq!(
            meta,
            SpriteMeta::NineSlice {
                width: 200,
                height: 26,
                border: [10; 4],
                stretch_inner: false
            }
        );
        let (_, meta) = a
            .gui_sprite(&Identifier::minecraft("tooltip/frame"))
            .unwrap();
        assert!(matches!(
            meta,
            SpriteMeta::NineSlice {
                stretch_inner: true,
                ..
            }
        ));
    });
}

/// Mob effect icons (vanilla `Hud.getMobEffectSprite` -> `mob_effect/<effect>`) live
/// under `textures/mob_effect/`, not `gui/sprites`: the gui atlas definition's
/// `mob_effect` directory source resolves them (before it, the HUD icon was the
/// missing texture).
#[test]
#[ignore = "needs the client jar in the cache"]
fn jar_gui_sprite_mob_effect() {
    jar_test("gui_sprite_mob_effect", |a| {
        let sprite = Identifier::minecraft("mob_effect/regeneration");
        let key = a.atlas_sprite_texture_key(&sprite, "gui").unwrap();
        assert_eq!(key, "assets/minecraft/textures/mob_effect/regeneration.png");
        let (img, meta) = a.gui_sprite(&sprite).unwrap();
        assert_eq!(img.dimensions(), (18, 18));
        assert_eq!(meta, SpriteMeta::Stretch);
        // The effect frame behind the icon is an ordinary gui sprite.
        let (frame, _) = a
            .gui_sprite(&Identifier::minecraft(
                "container/inventory/effect_background",
            ))
            .unwrap();
        assert_eq!(frame.dimensions(), (32, 32));
    });
}

/// A pack layer's atlas definition stacks on the jar's (vanilla
/// `SpriteSourceList.load` reads every layer's `atlases/gui.json`): its directory
/// sources add sprites, the jar's keep serving `gui/sprites`, and a sprite id's
/// namespace picks the namespace its texture is read from (mod effect icons).
#[test]
#[ignore = "needs the client jar in the cache"]
fn pack_atlas_sources_stack() {
    let mut assets = match open_assets() {
        Some(a) => a,
        None => {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
    };
    let packs_dir = haste_core::paths::cache_dir()
        .join("assets")
        .join("resourcepacks");
    let pack = packs_dir.join("AtlasSources");
    let dir = pack.join("assets/minecraft/textures/custom");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(pack.join("assets/minecraft/atlases")).unwrap();
    std::fs::create_dir_all(pack.join("assets/haste_test/textures/mob_effect")).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"min_format":97,"max_format":97,"description":"atlas sources"}}"#,
    )
    .unwrap();
    std::fs::write(
        pack.join("assets/minecraft/atlases/gui.json"),
        r#"{"sources":[{"type":"minecraft:directory","source":"custom","prefix":"custom/"}]}"#,
    )
    .unwrap();
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([10, 200, 90, 255]),
    ))
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();
    std::fs::write(dir.join("thing.png"), &png).unwrap();
    std::fs::write(
        pack.join("assets/haste_test/textures/mob_effect/thing.png"),
        &png,
    )
    .unwrap();
    assets.apply_packs(&packs_dir, &["file/AtlasSources".to_string()]);

    // The pack's directory source resolves its own sprites...
    let (img, _) = assets
        .gui_sprite(&Identifier::minecraft("custom/thing"))
        .unwrap();
    assert_eq!(img.get_pixel(0, 0), &image::Rgba([10, 200, 90, 255]));
    // ...the jar's sources still serve plain gui sprites...
    let (hotbar, _) = assets
        .gui_sprite(&Identifier::minecraft("hud/hotbar"))
        .unwrap();
    assert!(hotbar.dimensions().0 > 0);
    // ...and the jar's `mob_effect` source reads a pack texture in its own
    // namespace (how a mod's effect icon reaches the gui atlas).
    let (modded, _) = assets
        .gui_sprite(&Identifier::parse("haste_test:mob_effect/thing").unwrap())
        .unwrap();
    assert_eq!(modded.dimensions(), (8, 8));

    std::fs::remove_dir_all(&pack).unwrap();
}

#[test]
#[ignore = "needs the client jar in the cache"]
fn jar_model_parent_resolution() {
    jar_test("model", |a| {
        // stone -> block/cube_all -> block/cube -> block/block: one element,
        // textures all -> #all -> block/stone.
        let model = a.model(&Identifier::minecraft("block/stone")).unwrap();
        assert_eq!(model.elements.len(), 1);
        assert_eq!(
            model.textures.get("all").map(String::as_str),
            Some("minecraft:block/stone")
        );
        assert!(model.ambientocclusion);
    });
}

// ---- animation schedule and frame splitting (synthetic mcmeta + images) ----

/// A vertical 3-frame strip of 2x2 frames, colours distinguishable per frame.
fn three_frame_strip() -> image::RgbaImage {
    let mut sheet = image::RgbaImage::new(2, 6);
    for (frame, color) in [(60u8, 0, 0, 255), (120, 60, 0, 255), (180, 120, 0, 255)]
        .iter()
        .enumerate()
    {
        for y in 0..2 {
            for x in 0..2 {
                sheet.put_pixel(
                    x as u32,
                    (frame * 2 + y) as u32,
                    image::Rgba((*color).into()),
                );
            }
        }
    }
    sheet
}

fn mcmeta(json: &str) -> AnimationMetaRaw {
    let file: TextureMetaFile = serde_json::from_str(&format!("{{\"animation\":{json}}}"))
        .unwrap_or_else(|e| panic!("bad test mcmeta {json}: {e}"));
    file.animation
        .expect("test mcmeta should have an animation section")
}

fn animation_of(sheet: image::RgbaImage, json: &str) -> Animation {
    animation_from_raw(sheet, &mcmeta(json))
}

#[test]
fn default_schedule_steps_through_all_frames() {
    // No frames list, frametime 2: frames 0,1,2 each for 2 ticks, then wraps.
    let animation = animation_of(three_frame_strip(), r#"{"frametime":2}"#);
    assert_eq!(
        animation.schedule,
        AnimationSchedule::sequential(3, 2, false)
    );
    for (tick, frame) in [
        (0u64, 0u32),
        (1, 0),
        (2, 1),
        (3, 1),
        (4, 2),
        (5, 2),
        (6, 0),
        (7, 0),
    ] {
        assert_eq!(
            frame_at(&animation.schedule, tick),
            (frame, (frame + 1) % 3, 0.0),
            "tick {tick}"
        );
    }
}

#[test]
fn custom_frames_with_per_frame_times() {
    // Index 2 for 1 tick, index 0 for 5 ticks (time overrides frametime).
    let animation = animation_of(
        three_frame_strip(),
        r#"{"frametime":2,"frames":[{"index":2,"time":1},{"index":0,"time":5}]}"#,
    );
    assert_eq!(
        animation.schedule.frames,
        vec![
            AnimationFrame { index: 2, time: 1 },
            AnimationFrame { index: 0, time: 5 }
        ]
    );
    assert_eq!(frame_at(&animation.schedule, 0), (2, 0, 0.0));
    assert_eq!(frame_at(&animation.schedule, 1), (0, 2, 0.0));
    assert_eq!(frame_at(&animation.schedule, 5), (0, 2, 0.0));
    assert_eq!(frame_at(&animation.schedule, 6), (2, 0, 0.0)); // wrap at total 6
                                                               // An object without `time` uses frametime.
    let animation = animation_of(
        three_frame_strip(),
        r#"{"frametime":3,"frames":[1,{"index":2}]}"#,
    );
    assert_eq!(
        animation.schedule.frames,
        vec![
            AnimationFrame { index: 1, time: 3 },
            AnimationFrame { index: 2, time: 3 }
        ]
    );
}

#[test]
fn frames_given_as_plain_ints() {
    let animation = animation_of(three_frame_strip(), r#"{"frametime":4,"frames":[2,0,2]}"#);
    assert_eq!(
        animation.schedule.frames,
        vec![
            AnimationFrame { index: 2, time: 4 },
            AnimationFrame { index: 0, time: 4 },
            AnimationFrame { index: 2, time: 4 }
        ]
    );
    assert_eq!(frame_at(&animation.schedule, 4), (0, 2, 0.0));
    // Last tick of the cycle: the next entry wraps to the first (index 2 again).
    assert_eq!(frame_at(&animation.schedule, 11), (2, 2, 0.0));
    assert_eq!(frame_at(&animation.schedule, 12), (2, 0, 0.0)); // wrapped
}

#[test]
fn interpolate_blends_towards_next_frame() {
    let animation = animation_of(three_frame_strip(), r#"{"frametime":4,"interpolate":true}"#);
    assert!(animation.schedule.interpolate);
    // Blend is elapsed/time within each frame: 0 at the start, 0.5 halfway.
    assert_eq!(frame_at(&animation.schedule, 0), (0, 1, 0.0));
    assert_eq!(frame_at(&animation.schedule, 1), (0, 1, 0.25));
    assert_eq!(frame_at(&animation.schedule, 2), (0, 1, 0.5));
    assert_eq!(frame_at(&animation.schedule, 3), (0, 1, 0.75));
    // Next frame in playback order, wrapping at the end of the schedule.
    assert_eq!(frame_at(&animation.schedule, 10), (2, 0, 0.5));
    // Without interpolate the blend stays 0.
    let plain = animation_of(three_frame_strip(), r#"{"frametime":4}"#);
    assert_eq!(frame_at(&plain.schedule, 2), (0, 1, 0.0));
}

#[test]
fn grid_layout_splits_frames_in_index_order() {
    // 2x2 grid of 2x2 frames: index order is row-major (left-right, top-bottom).
    let mut sheet = image::RgbaImage::new(4, 4);
    let colors = [
        (10u8, 0, 0, 255),
        (20, 0, 0, 255),
        (30, 0, 0, 255),
        (40, 0, 0, 255),
    ];
    for (index, color) in colors.iter().enumerate() {
        let (fx, fy) = ((index % 2) * 2, (index / 2) * 2);
        for y in 0..2 {
            for x in 0..2 {
                sheet.put_pixel(
                    (fx + x) as u32,
                    (fy + y) as u32,
                    image::Rgba((*color).into()),
                );
            }
        }
    }
    let animation = animation_of(sheet, r#"{"width":2,"height":2,"frametime":1}"#);
    assert_eq!(animation.frames.len(), 4);
    for (index, color) in colors.iter().enumerate() {
        assert_eq!(
            animation.frames[index].get_pixel(0, 0),
            &image::Rgba((*color).into()),
            "frame {index}"
        );
    }
    assert_eq!(animation.schedule.frames.len(), 4);
}

#[test]
fn interpolate_flag_parses_and_initial_frame_is_first_listed() {
    let animation = animation_of(
        three_frame_strip(),
        r#"{"interpolate":true,"frames":[{"index":1,"time":2}]}"#,
    );
    assert!(animation.schedule.interpolate);
    assert_eq!(
        animation.initial_frame().get_pixel(0, 0),
        &image::Rgba([120, 60, 0, 255])
    );
    // The meta path carries the flag too.
    assert!(AnimationMeta::from(mcmeta(r#"{"interpolate":true}"#)).interpolate);
    assert!(!AnimationMeta::from(mcmeta(r#"{}"#)).interpolate);
}
