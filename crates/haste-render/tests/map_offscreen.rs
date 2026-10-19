//! Maps checked offscreen: a framed map's contents and decorations draw in the frame
//! (not the item model), and the held map shows the two-handed view with the map's
//! background and contents. The map data is fed through `Renderer::set_maps` like the
//! client does; the packed colours come from `MapColor` (water blue with a stone patch and
//! grass), so the assertions can look for the map's own pixels.
//!
//! Needs the client jar, the datagen reports and a Vulkan driver (source scripts/env.sh).

use std::collections::HashMap;
use std::sync::Arc;

use haste_core::model::{BodyTransform, Decoration, EntityInstance, ItemFrameDecoration};
use haste_core::view::BlockView;
use haste_core::{
    BlockPos, BlockStateId, ChunkPos, Direction, EntityId, Identifier, RegistryIndex,
};
use haste_render::{Camera, FrameInput, MapData, MapMarker, Renderer, SkyState};

struct Flat {
    stone: BlockStateId,
}

impl BlockView for Flat {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        if p.y == 0 {
            self.stone
        } else {
            BlockStateId::AIR
        }
    }
    fn is_chunk_loaded(&self, _: ChunkPos) -> bool {
        true
    }
    fn sky_light(&self, p: BlockPos) -> u8 {
        if p.y <= 0 {
            0
        } else {
            15
        }
    }
    fn block_light(&self, _: BlockPos) -> u8 {
        0
    }
    fn min_y(&self) -> i32 {
        -64
    }
    fn height(&self) -> i32 {
        384
    }
}

/// One map: water blue everywhere, a grass quarter and a stone strip, a player marker in
/// the middle and a frame-only marker at the edge. Packed ids: water 12, grass 1, stone
/// 11, brightness NORMAL 1.
fn map_data() -> MapData {
    let mut colors = vec![(12u8 << 2) | 1; 128 * 128];
    for y in 0..32 {
        for x in 0..32 {
            colors[x + y * 128] = (1 << 2) | 1;
        }
    }
    for y in 60..68 {
        for x in 0..128 {
            colors[x + y * 128] = (11 << 2) | 1;
        }
    }
    MapData {
        scale: 0,
        locked: false,
        version: 1,
        colors,
        decorations: vec![
            MapMarker {
                kind: 0,
                x: 0,
                y: 0,
                rot: 0,
                name: None,
            },
            MapMarker {
                kind: 1,
                x: 120,
                y: 120,
                rot: 0,
                name: None,
            },
        ],
    }
}

fn frame_instance(
    id: i32,
    pos: glam::DVec3,
    map: Option<i32>,
    item: RegistryIndex,
) -> EntityInstance {
    EntityInstance {
        id: EntityId(id),
        model: haste_entity::models::box_for(1.0, 1.0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        scale: 1.0,
        pose: Default::default(),
        body: BodyTransform::default(),
        body_scale: glam::Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        shadow_radius: 0.0,
        shadow_strength: 1.0,
        invisible: false,
        display_fire: false,
        width: 0.6,
        height: 1.8,
        is_discrete: false,
        name_tag_y: 1.8,
        hurt: 0.0,
        light: (15, 0),
        overlays: Default::default(),
        item: None,
        decoration: Some(Decoration::ItemFrame(ItemFrameDecoration {
            facing: Direction::North,
            rotation: 0,
            glow: false,
            item: map.is_some().then_some(item),
            map_id: map,
        })),
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    }
}

fn camera() -> Camera {
    Camera {
        pos: glam::DVec3::new(0.5, 1.62, -3.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_deg: 70.0,
        hand_fov_deg: 70.0,
        roll_deg: 0.0,
        bob: haste_render::ViewBob::default(),
    }
}

fn render(
    renderer: &mut Renderer,
    world: &dyn BlockView,
    entities: &mut [EntityInstance],
) -> image::RgbaImage {
    for e in entities.iter_mut() {
        e.sample_light(world);
    }
    let gui = haste_core::gui::GuiFrame {
        scale: 2,
        ..Default::default()
    };
    renderer
        .render(&FrameInput {
            camera: camera(),
            entities,
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
}

/// The framed map draws the map's contents (water blue and the grass corner) and the
/// player marker inside the frame, not the item model; without data the frame falls back
/// to the item model (a flat sprite, whose pixels differ from the map's).
#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn framed_map_draws_contents_and_decorations() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let world = Flat {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap(),
    };
    let filled_map = data
        .registries
        .index("minecraft:item", &Identifier::minecraft("filled_map"))
        .unwrap();
    // The frame hangs on the wall block behind it (like the live check's rig).
    let mut maps = HashMap::new();
    maps.insert(7, map_data());
    renderer.set_maps(Arc::new(maps));
    let mut entities = vec![frame_instance(
        1,
        glam::DVec3::new(0.5, 1.5, 0.5),
        Some(7),
        filled_map,
    )];
    let base = render(&mut renderer, &world, &mut entities);

    // The map's water blue (`MapColor.WATER` at NORMAL) and grass green appear where the
    // frame is; count them over the frame's screen area (a 1x1 frame at 4 blocks fills a
    // region around the centre).
    let mut blue = 0;
    let mut green = 0;
    for y in 120..360 {
        for x in 200..440 {
            let p = base.get_pixel(x, y).0;
            let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
            if b > 120 && b > r + 40 && g < 100 {
                blue += 1;
            }
            if g > 120 && g > r + 30 && g > b + 30 {
                green += 1;
            }
        }
    }
    assert!(blue > 500, "the map's water blue drew {blue} pixels");
    assert!(green > 100, "the map's grass corner drew {green} pixels");

    // A frame without data falls back to the item model (a flat sprite), whose pixels
    // differ from the map's.
    let mut entities = vec![frame_instance(
        2,
        glam::DVec3::new(0.5, 1.5, 0.5),
        None,
        filled_map,
    )];
    let fallback = render(&mut renderer, &world, &mut entities);
    let changed = (0..480)
        .flat_map(|y| (0..640).map(move |x| (x, y)))
        .filter(|&(x, y)| fallback.get_pixel(x, y).0 != base.get_pixel(x, y).0)
        .count();
    assert!(changed > 2000, "the fallback differs by {changed} pixels");
}

/// The held map: the two-handed view with the map's background border, contents and both
/// arms (vanilla `renderTwoHandedMap`).
#[test]
#[ignore = "needs the client jar, datagen reports and a Vulkan driver (source scripts/env.sh)"]
fn held_map_draws_the_two_handed_view() {
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let data = haste_data::GameData::load().unwrap();
    let mut renderer = Renderer::new_offscreen(640, 480, &mut assets, &data.blocks).unwrap();
    renderer.set_item_registry(&data.registries);
    let world = Flat {
        stone: data
            .blocks
            .default_state(&Identifier::minecraft("stone"))
            .unwrap(),
    };
    let mut maps = HashMap::new();
    maps.insert(7, map_data());
    renderer.set_maps(Arc::new(maps));
    let mut none: Vec<EntityInstance> = Vec::new();
    let base = render(&mut renderer, &world, &mut none);
    renderer.set_first_person(Some(haste_render::FirstPerson {
        skin: None,
        slim: false,
        sleeve: true,
        item: None,
        map: Some(7),
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
    let img = render(&mut renderer, &world, &mut none);
    renderer.set_first_person(None);

    let drawn = (0..480)
        .flat_map(|y| (0..640).map(move |x| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0 != base.get_pixel(x, y).0)
        .count();
    assert!(drawn > 20_000, "the held map drew {drawn} pixels");
    // The map's water blue is in the middle of the screen (the map fills the lower half),
    // and the skin's arm colour at the sides (both arms hold it).
    let mut blue = 0;
    for y in 240..400 {
        for x in 240..400 {
            let p = img.get_pixel(x, y).0;
            let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
            if b > 120 && b > r + 40 && g < 100 {
                blue += 1;
            }
        }
    }
    assert!(blue > 200, "the held map's contents drew {blue} pixels");
}
