//! Held items and GUI slots take the model their `items/<id>.json` names, so plants,
//! saplings, rails, doors and torches are drawn as their flat `builtin/generated` item model
//! (the block's sprite extruded 1/16), never as the block's crossed-plane model. Block items
//! (stone) keep the block model. Needs the client jar, the datagen reports and a Vulkan driver
//! (source scripts/env.sh).

use haste_core::gui::{GuiCommand, GuiFrame};
use haste_core::{Identifier, RegistryIndex};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

const SCALE: u32 = 2;
const W: u32 = 400;
const H: u32 = 300;

fn item_index(data: &haste_data::GameData, name: &str) -> RegistryIndex {
    data.registries
        .index("minecraft:item", &Identifier::minecraft(name))
        .unwrap()
}

fn px(img: &image::RgbaImage, gx: u32, gy: u32) -> [u8; 4] {
    img.get_pixel(gx * SCALE, gy * SCALE).0
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn gui_slots_draw_flat_items_with_the_item_model() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer =
        Renderer::new_offscreen(W * SCALE, H * SCALE, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);

    let flat = ["rail", "short_grass", "oak_sapling", "torch"];
    let mut commands = vec![GuiCommand::Fill {
        x1: 0,
        y1: 0,
        x2: W as i32,
        y2: H as i32,
        color: 0xFF10_1820,
    }];
    for (n, name) in flat.iter().enumerate() {
        commands.push(GuiCommand::Item {
            item: item_index(&data, name),
            x: 13 + 20 * n as i32,
            y: 53,
            count: 1,
        });
    }
    commands.push(GuiCommand::Item {
        item: item_index(&data, "stone"),
        x: 13,
        y: 80,
        count: 1,
    });
    let gui = GuiFrame {
        commands,
        scale: SCALE,
        width: W,
        height: H,
        cursor: None,
        blur_after: -1,
    };
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 100.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let input = FrameInput {
        camera,
        entities: &[],
        block_entities: &[],
        gui: &gui,
        sky: SkyState {
            time_of_day: 6000,
            rain: 0.0,
            dimension_has_sky: true,
            fog_start: 100.0,
            fog_end: 200.0,
            ..Default::default()
        },
        partial_tick: 0.0,
        block_target: None,
        breaking: &[],
        camera_entity: None,
        spin: None,
        weather: None,
    };
    renderer.render(&input).unwrap();
    let img = renderer.read_back().unwrap();
    let bg = [0x10u8, 0x18, 0x20, 255];

    // A flat sprite fills its 16x16 slot where it is opaque: the rail's textured corners rule
    // out a thin block model, and every flat item covers the rows its sprite has (the torch is
    // a 10-row stick).
    for (n, name) in flat.iter().enumerate() {
        let x = 13 + 20 * n as u32;
        let covered = (0..16)
            .filter(|&dy| (0..16).any(|dx| px(&img, x + dx, 54 + dy) != bg))
            .count();
        assert!(covered >= 10, "{name}: only {covered}/16 rows drawn");
    }
    let rail_corner = px(&img, 13 + 1, 53 + 1);
    assert_eq!(rail_corner[3], 255);
    assert!(rail_corner != bg, "rail corner {rail_corner:?}");

    // The block item keeps its block model: a cube centred in the slot with background at the
    // slot's corners (a flat sprite would cover them).
    let stone = [
        px(&img, 14, 81),
        px(&img, 27, 81),
        px(&img, 14, 94),
        px(&img, 27, 94),
    ];
    assert!(
        stone.iter().all(|p| p == &bg),
        "stone corners {stone:?} should be background"
    );
    let stone_centre = px(&img, 20, 87);
    assert!(
        stone_centre[0] > 60 && stone_centre[0].abs_diff(stone_centre[1]) < 12,
        "stone centre {stone_centre:?}"
    );
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn held_item_draws_the_item_model_not_the_block_model() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer =
        Renderer::new_offscreen(W * SCALE, H * SCALE, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let camera = Camera {
        pos: glam::DVec3::new(0.0, 100.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let empty_gui = GuiFrame {
        commands: vec![],
        scale: 1,
        width: W,
        height: H,
        cursor: None,
        blur_after: -1,
    };
    let render = |renderer: &mut Renderer| {
        renderer
            .render(&FrameInput {
                camera,
                entities: &[],
                block_entities: &[],
                gui: &empty_gui,
                sky: SkyState {
                    time_of_day: 6000,
                    rain: 0.0,
                    dimension_has_sky: true,
                    fog_start: 100.0,
                    fog_end: 200.0,
                    ..Default::default()
                },
                partial_tick: 0.0,
                block_target: None,
                breaking: &[],
                camera_entity: None,
                spin: None,
                weather: None,
            })
            .unwrap();
        renderer.read_back().unwrap()
    };
    renderer.set_first_person(None);
    let base = render(&mut renderer);
    // The flat item models fill their quad (the rail's sprite is a full 16x16, the plant's
    // shape covers most of it); a thin crossed-plane block model would leave a few hundred
    // pixels at most.
    for (name, min) in [("short_grass", 2500), ("rail", 4000), ("stone", 5000)] {
        renderer.set_first_person(Some(haste_render::FirstPerson {
            skin: None,
            slim: false,
            item: Some(item_index(&data, name)),
            light: (15, 0),
            attack: haste_render::TickPair::fixed(0.0),
            equip: haste_render::TickPair::fixed(1.0),
            x_bob: haste_render::TickPair::fixed(0.0),
            y_bob: haste_render::TickPair::fixed(0.0),
        }));
        let hand = render(&mut renderer);
        let changed = base
            .pixels()
            .zip(hand.pixels())
            .filter(|(a, b)| a.0 != b.0)
            .count();
        assert!(
            changed > min,
            "held {name}: only {changed} pixels, a block model's quad would be far smaller"
        );
    }
    renderer.set_first_person(None);
}
