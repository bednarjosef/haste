//! The enchantment glint draws on items with foil, in GUI slots and in the hand, and only
//! brightens (vanilla adds the squared glint texel to the lit colour). Needs the client jar,
//! the datagen reports and a Vulkan driver (source scripts/env.sh).

use haste_core::gui::{GuiCommand, GuiFrame};
use haste_core::model::FoilType;
use haste_core::{Identifier, RegistryIndex};
use haste_render::{Camera, FrameInput, Renderer, SkyState};

const SCALE: u32 = 2;
const W: u32 = 200;
const H: u32 = 120;

fn item_index(data: &haste_data::GameData, name: &str) -> RegistryIndex {
    data.registries
        .index("minecraft:item", &Identifier::minecraft(name))
        .unwrap()
}

fn camera() -> Camera {
    Camera {
        pos: glam::DVec3::new(0.0, 100.0, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    }
}

fn render(renderer: &mut Renderer, gui: &GuiFrame) -> image::RgbaImage {
    renderer
        .render(&FrameInput {
            camera: camera(),
            entities: &[],
            block_entities: &[],
            gui,
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
}

/// The summed brightness of the region and whether any pixel in it got darker than `base`.
fn compare(base: &image::RgbaImage, glint: &image::RgbaImage, r: [u32; 4]) -> (i64, i64, bool) {
    let (mut a, mut b, mut darker) = (0i64, 0i64, false);
    for y in r[1]..r[3] {
        for x in r[0]..r[2] {
            let p = base.get_pixel(x, y).0;
            let q = glint.get_pixel(x, y).0;
            a += p[..3].iter().map(|&c| c as i64).sum::<i64>();
            b += q[..3].iter().map(|&c| c as i64).sum::<i64>();
            darker |= (0..3).any(|i| (q[i] as i32) + 1 < p[i] as i32);
        }
    }
    (a, b, darker)
}

#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn glint_brightens_gui_items_and_the_held_item() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer =
        Renderer::new_offscreen(W * SCALE, H * SCALE, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);

    // GUI: a sword (items sheet) and stone (blocks sheet), plain and with foil.
    let slots = |foil: FoilType| GuiFrame {
        commands: vec![
            GuiCommand::Fill {
                x1: 0,
                y1: 0,
                x2: W as i32,
                y2: H as i32,
                color: 0xFF10_1820,
            },
            GuiCommand::Item {
                item: item_index(&data, "diamond_sword"),
                x: 10,
                y: 10,
                count: 1,
                foil,
            },
            GuiCommand::Item {
                item: item_index(&data, "stone"),
                x: 40,
                y: 10,
                count: 1,
                foil,
            },
        ],
        scale: SCALE,
        width: W,
        height: H,
        cursor: None,
        blur_after: -1,
    };
    renderer.set_first_person(None);
    let plain = render(&mut renderer, &slots(FoilType::None));
    let glint = render(&mut renderer, &slots(FoilType::Standard));
    for (name, x) in [("diamond_sword", 10), ("stone", 40)] {
        let r = [x * SCALE, 10 * SCALE, (x + 16) * SCALE, 26 * SCALE];
        let (a, b, darker) = compare(&plain, &glint, r);
        assert!(b > a + a / 50, "{name}: glint {b} vs plain {a}");
        assert!(!darker, "{name}: the glint only adds");
    }
    // The background around the slots is untouched.
    let bg = [100 * SCALE, 60 * SCALE, 120 * SCALE, 80 * SCALE];
    let (a, b, _) = compare(&plain, &glint, bg);
    assert_eq!(a, b);

    // The held item: the same sword in the first-person hand.
    let empty = GuiFrame {
        commands: vec![],
        scale: 1,
        width: W,
        height: H,
        cursor: None,
        blur_after: -1,
    };
    let hand = |foil: FoilType| haste_render::FirstPerson {
        skin: None,
        slim: false,
        sleeve: true,
        item: Some(item_index(&data, "diamond_sword")),
        foil,
        map: None,
        offhand_empty: true,
        light: (15, 0),
        use_eat: false,
        use_remaining: 0,
        use_duration: 0,
        attack: haste_render::TickPair::fixed(0.0),
        equip: haste_render::TickPair::fixed(1.0),
        x_bob: haste_render::TickPair::fixed(0.0),
        y_bob: haste_render::TickPair::fixed(0.0),
    };
    renderer.set_first_person(Some(hand(FoilType::None)));
    let plain = render(&mut renderer, &empty);
    renderer.set_first_person(Some(hand(FoilType::Standard)));
    let glint = render(&mut renderer, &empty);
    let all = [0, 0, W * SCALE, H * SCALE];
    let (a, b, darker) = compare(&plain, &glint, all);
    assert!(b > a, "held sword: glint {b} vs plain {a}");
    assert!(!darker, "the held glint only adds");
}
