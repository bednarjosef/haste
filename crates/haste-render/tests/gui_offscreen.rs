//! GUI drawing checked offscreen: a hand-built `GuiFrame` with every command kind, rendered at
//! GUI scale 2 and probed at known pixels. Saves `target/render-checks/gui.png` for a human look.
//! Needs the client jar, the datagen reports and a Vulkan driver (source scripts/env.sh).

use haste_core::gui::{GuiBlend, GuiCommand, GuiFrame, TextStyle};
use haste_core::{Identifier, RegistryIndex};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

const SCALE: u32 = 2;
const W: u32 = 400;
const H: u32 = 300;

fn text(x: i32, y: i32, s: &str, color: u32, shadow: bool) -> GuiCommand {
    GuiCommand::Text {
        x,
        y,
        text: s.into(),
        color,
        shadow,
    }
}

fn styled(x: i32, y: i32, s: &str, style: TextStyle) -> GuiCommand {
    GuiCommand::StyledText {
        x,
        y,
        text: s.into(),
        color: 0xFFFF_FFFF,
        shadow: true,
        style,
    }
}

fn sprite(name: &str, x: i32, y: i32, w: i32, h: i32) -> GuiCommand {
    GuiCommand::Sprite {
        sprite: Identifier::minecraft(name),
        x,
        y,
        w,
        h,
        tint: 0xFFFF_FFFF,
        blend: GuiBlend::Normal,
    }
}

fn frame(items: &[(RegistryIndex, &str)]) -> GuiFrame {
    let mut c = vec![
        // A dark panel behind everything so text and items have a known background.
        GuiCommand::Fill {
            x1: 0,
            y1: 0,
            x2: W as i32,
            y2: H as i32,
            color: 0xFF20_2030,
        },
        GuiCommand::Fill {
            x1: 10,
            y1: 10,
            x2: 30,
            y2: 30,
            color: 0xFFFF_0000,
        },
        GuiCommand::FillGradient {
            x1: 40,
            y1: 10,
            x2: 60,
            y2: 30,
            top: 0xFFFF_FFFF,
            bottom: 0xFF00_0000,
        },
        // Half-transparent black over the red square: vanilla blends sRGB values.
        GuiCommand::Fill {
            x1: 20,
            y1: 20,
            x2: 30,
            y2: 30,
            color: 0x8000_0000,
        },
        text(70, 10, "Hello World", 0xFFFF_FFFF, true),
        text(70, 22, "§cRed §aGreen §lBold§r plain", 0xFFFF_FFFF, true),
        text(70, 34, "Příliš žluťoučký kůň 中文", 0xFFFF_FF55, false),
        text(250, 10, "invisible: alpha 0", 0x00FF_FFFF, false),
        styled(
            250,
            22,
            "bold",
            TextStyle {
                bold: true,
                ..Default::default()
            },
        ),
        styled(
            290,
            22,
            "italic",
            TextStyle {
                italic: true,
                ..Default::default()
            },
        ),
        styled(
            250,
            34,
            "under",
            TextStyle {
                underline: true,
                ..Default::default()
            },
        ),
        styled(
            290,
            34,
            "strike",
            TextStyle {
                strikethrough: true,
                ..Default::default()
            },
        ),
        styled(
            340,
            34,
            "magic",
            TextStyle {
                obfuscated: true,
                ..Default::default()
            },
        ),
        // Stretch sprite (hotbar), nine-slice buttons at natural and stretched sizes.
        sprite("hud/hotbar", 10, 50, 182, 22),
        sprite("hud/hotbar_selection", 9, 49, 24, 23),
        sprite("widget/button", 200, 50, 200, 20),
        sprite("widget/button", 200, 74, 60, 30),
        sprite("widget/button_highlighted", 270, 74, 120, 20),
        sprite("tooltip/background", 10, 230, 120, 40),
        sprite("tooltip/frame", 10, 230, 120, 40),
        // A region of a full texture (the inventory background's top-left 176x83).
        GuiCommand::Texture {
            texture: Identifier::parse("minecraft:textures/gui/container/inventory.png").unwrap(),
            x: 200,
            y: 110,
            w: 176,
            h: 83,
            u: 0.0,
            v: 0.0,
            uw: 176.0,
            vh: 83.0,
            tex_w: 256,
            tex_h: 256,
            tint: 0xFFFF_FFFF,
            blend: GuiBlend::Normal,
        },
        GuiCommand::SpriteRegion {
            sprite: Identifier::minecraft("hud/experience_bar_progress"),
            sprite_w: 182,
            sprite_h: 5,
            u: 0,
            v: 0,
            x: 10,
            y: 76,
            w: 91,
            h: 5,
            tint: 0xFFFF_FFFF,
            blend: GuiBlend::Normal,
        },
        // Scissor: a blue fill larger than the clip.
        GuiCommand::ScissorPush {
            x1: 200,
            y1: 200,
            x2: 220,
            y2: 220,
        },
        GuiCommand::Fill {
            x1: 190,
            y1: 190,
            x2: 240,
            y2: 240,
            color: 0xFF00_00FF,
        },
        GuiCommand::ScissorPop,
        // Translate: a green 10x10 square moved by (5, 5).
        GuiCommand::TranslatePush {
            dx: 5.0,
            dy: 5.0,
            dz: 0.0,
        },
        GuiCommand::Fill {
            x1: 250,
            y1: 200,
            x2: 260,
            y2: 210,
            color: 0xFF00_FF00,
        },
        GuiCommand::TranslatePop,
    ];
    // Items in the hotbar slots (slot n at x = 13 + 20 n, y = 53).
    for (n, (item, _)) in items.iter().enumerate() {
        let x = 13 + 20 * n as i32;
        c.push(GuiCommand::Item {
            item: *item,
            x,
            y: 53,
            count: 1,
        });
        c.push(text(x + 11, 62, "64", 0xFFFF_FFFF, true));
    }
    // A second row of items larger (vanilla draws 16x16; this is the plain slot size too).
    GuiFrame {
        commands: c,
        scale: SCALE,
        width: W,
        height: H,
        cursor: None,
    }
}

fn px(img: &image::RgbaImage, gx: f32, gy: f32) -> [u8; 4] {
    img.get_pixel((gx * SCALE as f32) as u32, (gy * SCALE as f32) as u32)
        .0
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn gui_frame_offscreen() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer =
        Renderer::new_offscreen(W * SCALE, H * SCALE, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);

    let names = [
        "stone",
        "grass_block",
        "diamond_sword",
        "oak_stairs",
        "potion",
        "red_bed",
        "oak_leaves",
        "glass",
        "torch",
    ];
    let items: Vec<(RegistryIndex, &str)> = names
        .iter()
        .map(|n| {
            (
                data.registries
                    .index("minecraft:item", &Identifier::minecraft(n))
                    .unwrap(),
                *n,
            )
        })
        .collect();
    let gui = frame(&items);
    let input = FrameInput {
        camera: Camera {
            pos: glam::DVec3::new(0.0, 100.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_deg: 70.0,
            roll_deg: 0.0,
        },
        entities: &[],
        gui: &gui,
        sky: SkyState {
            time_of_day: 6000,
            rain: 0.0,
            dimension_has_sky: true,
            fog_start: 100.0,
            fog_end: 200.0,
        },
        partial_tick: 0.0,
        block_target: None,
        breaking: &[],
    };
    let stats = renderer.render(&input).unwrap();
    // Second frame: everything resident, measure.
    let stats2 = renderer.render(&input).unwrap();
    let gs = renderer.gui_stats();
    eprintln!(
        "gui frame: first {:.2} ms, second {:.2} ms, {} commands, {} vertices, {} draw calls, {} unsupported, {} atlas resets",
        stats.frame_ms, stats2.frame_ms, gs.commands, gs.vertices, gs.draw_calls, gs.unsupported, gs.atlas_resets
    );
    let img = renderer.read_back().unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-checks");
    std::fs::create_dir_all(&out).unwrap();
    img.save(out.join("gui.png")).unwrap();
    let zoom = |x: u32, y: u32, w: u32, h: u32, name: &str| {
        let crop = image::imageops::crop_imm(&img, x, y, w, h).to_image();
        image::imageops::resize(&crop, w * 3, h * 3, image::imageops::FilterType::Nearest)
            .save(out.join(name))
            .unwrap();
    };
    zoom(18, 98, 372, 50, "gui_items_zoom.png");
    zoom(130, 10, 270, 80, "gui_text_zoom.png");

    // Fills, exact colours.
    assert_eq!(px(&img, 12.5, 12.5), [255, 0, 0, 255], "red fill");
    assert_eq!(
        px(&img, 9.5, 9.5),
        [0x20, 0x20, 0x30, 255],
        "outside the fill"
    );
    // 50% black over red, blended on sRGB values like vanilla: 255 * 0.5 ~= 127.
    // Without view formats (GL) the blend happens in linear space: sRGB(0.5 * 1.0) = 188.
    let half = px(&img, 25.5, 25.5);
    let expected = if renderer.gui_blends_in_srgb() {
        126..=129
    } else {
        185..=190
    };
    assert!(
        expected.contains(&half[0]) && half[1] == 0,
        "blend {half:?}"
    );
    // Gradient: white top, black bottom, mid grey between.
    assert!(px(&img, 50.0, 10.25)[0] > 245);
    assert!(px(&img, 50.0, 29.75)[0] < 10);
    let mid = px(&img, 50.0, 20.0)[0];
    assert!((110..=145).contains(&mid), "gradient middle {mid}");
    // Scissor clips the blue fill to 200..220.
    assert_eq!(
        px(&img, 210.5, 210.5),
        [0, 0, 255, 255],
        "inside the scissor"
    );
    assert_eq!(
        px(&img, 195.5, 210.5),
        [0x20, 0x20, 0x30, 255],
        "left of the scissor"
    );
    assert_eq!(
        px(&img, 225.5, 225.5),
        [0x20, 0x20, 0x30, 255],
        "below the scissor"
    );
    // Translate moved the green square by (5, 5).
    assert_eq!(px(&img, 257.5, 207.5), [0, 255, 0, 255], "translated fill");
    assert_eq!(
        px(&img, 252.5, 202.5),
        [0x20, 0x20, 0x30, 255],
        "untranslated corner"
    );
    // "H" of "Hello World" at (70, 10): its left column is solid white for 7 rows, shadow 1px
    // right/down in quarter brightness.
    assert_eq!(px(&img, 70.5, 12.5), [255, 255, 255, 255], "H stem");
    assert_eq!(
        px(&img, 71.5, 17.5),
        [0x3F, 0x3F, 0x3F, 255],
        "H shadow under the stem"
    );
    // Alpha-0 text is not drawn.
    for x in 250..340 {
        for y in 10..18 {
            assert_eq!(
                px(&img, x as f32 + 0.5, y as f32 + 0.5),
                [0x20, 0x20, 0x30, 255]
            );
        }
    }
    // Hotbar sprite drawn (its frame is not the panel colour).
    assert_ne!(
        px(&img, 100.5, 51.5),
        [0x20, 0x20, 0x30, 255],
        "hotbar frame"
    );
    // Button stretched to 60x30: nine-slice corners stay crisp: the 1px black outline at the
    // left edge and the light inner border next to it.
    let edge = px(&img, 200.25, 89.0);
    assert!(edge[0] < 40, "button outline {edge:?}");
    // Items: stone in slot 0 is grey at the cube centre, the grass block's top is green.
    let stone = px(&img, 21.0, 59.0);
    assert!(
        stone[0] > 60 && stone[0].abs_diff(stone[1]) < 12 && stone[1].abs_diff(stone[2]) < 12,
        "stone {stone:?}"
    );
    let grass = px(&img, 41.0, 57.0);
    assert!(
        grass[1] > grass[0] && grass[1] > grass[2],
        "grass top {grass:?}"
    );
    assert_eq!(gs.unsupported, 0);
    assert!(gs.draw_calls <= 4, "batched: {} draw calls", gs.draw_calls);
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn text_metrics_match_vanilla_widths() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let font = assets.font_default().unwrap();
    use haste_core::gui::TextMetrics;
    // Widths a vanilla Font reports for these strings (5-px glyphs advance 6, i 2, l 3, space 4).
    // S6 i2 n6 g6 l3 e6 p6 l3 a6 y6 e6 r6
    assert_eq!(font.width("Singleplayer"), 62);
    // B6 o6 l3 d6 + 1 per bold character
    assert_eq!(font.width("§lBold"), 25);
    assert_eq!(font.line_height(), 9);
}
