//! The held block in first person must be a solid cube: no pixel of the frame behind the
//! hand may show inside the silhouette the hand draws (see-through holes). The drawn
//! pixels of a convex solid block are convex, so any background pixel inside their convex
//! hull is a hole. Follows vanilla `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`
//! rendering the item through `ItemDisplayContext.FIRST_PERSON_RIGHT_HAND` with back-face
//! culling on wound quads (`VertexConsumer.putBakedQuad`).
//!
//! Needs the client jar, the datagen reports and a Vulkan driver (source scripts/env.sh).

use haste_core::gui::GuiFrame;
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

/// The background pixels inside the convex hull of the pixels the hand changed: the
/// see-through holes. Empty for a solid cube.
fn holes_against_base(base: &image::RgbaImage, img: &image::RgbaImage) -> Vec<(u32, u32)> {
    let drawn: Vec<(u32, u32)> = (0..H * SCALE)
        .flat_map(|y| (0..W * SCALE).map(move |x| (x, y)))
        .filter(|(x, y)| img.get_pixel(*x, *y).0 != base.get_pixel(*x, *y).0)
        .collect();
    // Monotone-chain convex hull of the drawn pixels.
    let mut p = drawn.clone();
    p.sort();
    let cross = |o: (u32, u32), a: (u32, u32), b: (u32, u32)| {
        (a.0 as i64 - o.0 as i64) * (b.1 as i64 - o.1 as i64)
            - (a.1 as i64 - o.1 as i64) * (b.0 as i64 - o.0 as i64)
    };
    let mut lower: Vec<(u32, u32)> = vec![];
    for q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *q) <= 0 {
            lower.pop();
        }
        lower.push(*q);
    }
    let mut upper: Vec<(u32, u32)> = vec![];
    for q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *q) <= 0 {
            upper.pop();
        }
        upper.push(*q);
    }
    lower.pop();
    upper.pop();
    let hull = [lower, upper].concat();
    let in_hull = |h: &[(u32, u32)], x: u32, y: u32| {
        let mut inside = false;
        let n = h.len();
        for i in 0..n {
            let (x1, y1) = h[i];
            let (x2, y2) = h[(i + 1) % n];
            if (y1 > y) != (y2 > y) {
                let t = (y as f64 - y1 as f64) / (y2 as f64 - y1 as f64);
                let xi = x1 as f64 + t * (x2 as f64 - x1 as f64);
                if (x as f64) < xi {
                    inside = !inside;
                }
            }
        }
        inside
    };
    (0..H * SCALE)
        .flat_map(|y| (0..W * SCALE).map(move |x| (x, y)))
        .filter(|(x, y)| {
            in_hull(&hull, *x, *y) && img.get_pixel(*x, *y).0 == base.get_pixel(*x, *y).0
        })
        .collect()
}

/// Every full (convex, opaque) block item held in the main hand covers its whole
/// silhouette at every pitch: no see-through holes, whatever the face culling or the
/// draw order does with the cube's six quads.
#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn held_block_is_a_solid_cube() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer =
        Renderer::new_offscreen(W * SCALE, H * SCALE, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let gui = GuiFrame {
        commands: vec![],
        scale: SCALE,
        width: W,
        height: H,
        cursor: None,
        blur_after: -1,
    };
    let camera = |pitch: f32| Camera {
        pos: glam::DVec3::new(0.0, 100.0, 0.0),
        yaw: 0.0,
        pitch,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    };
    let render = |renderer: &mut Renderer, pitch: f32| {
        renderer
            .render(&FrameInput {
                camera: camera(pitch),
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
            })
            .unwrap();
        renderer.read_back().unwrap()
    };
    for name in [
        "stone",
        "dirt",
        "cobblestone",
        "oak_planks",
        "oak_log",
        "sand",
        "bricks",
    ] {
        for pitch in [-20.0f32, -10.0, 0.0, 10.0, 20.0] {
            renderer.set_first_person(None);
            let base = render(&mut renderer, pitch);
            renderer.set_first_person(Some(haste_render::FirstPerson {
                skin: None,
                slim: false,
                sleeve: true,
                item: Some(item_index(&data, name)),
                foil: Default::default(),
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
            }));
            let img = render(&mut renderer, pitch);
            let drawn = (0..H * SCALE)
                .flat_map(|y| (0..W * SCALE).map(move |x| (x, y)))
                .filter(|(x, y)| img.get_pixel(*x, *y).0 != base.get_pixel(*x, *y).0)
                .count();
            assert!(
                drawn > 5000,
                "held {name} at pitch {pitch}: only {drawn} pixels"
            );
            let holes = holes_against_base(&base, &img);
            assert!(
                holes.is_empty(),
                "held {name} at pitch {pitch}: {} see-through holes inside the cube, first: {:?}",
                holes.len(),
                &holes[..holes.len().min(20)]
            );
        }
    }
    renderer.set_first_person(None);
}
