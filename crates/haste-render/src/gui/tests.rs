use super::*;
use glam::Vec3;
use haste_assets::{ItemGeometry, ItemModel};

/// Vanilla's hotbar panel interior over a mid blue: 0.729 * 39/255 + 0.271 * 48/255 = 41.4,
/// and the same in the other two channels (48.6, 55.6) -> 41, 49, 56.
#[test]
fn blend_gui_pixel_matches_vanilla_srgb_math() {
    assert_eq!(
        blend_gui_pixel([39, 37, 5, 186], [48, 80, 192]),
        [41, 49, 56, 255]
    );
    // A fully opaque sprite pixel is copied through.
    assert_eq!(
        blend_gui_pixel([10, 20, 30, 255], [48, 80, 192]),
        [10, 20, 30, 255]
    );
    // A fully transparent one leaves the background alone.
    assert_eq!(
        blend_gui_pixel([255, 255, 255, 0], [48, 80, 192]),
        [48, 80, 192, 255]
    );
    // Alpha: dst_a' = a + (1 - a) * dst_a, so a semi-transparent pixel over transparency
    // only reaches its own alpha.
    assert_eq!(blend_gui_alpha(128, 0), 128);
    assert_eq!(blend_gui_alpha(128, 51), 153);
    assert_eq!(blend_gui_alpha(0, 200), 200);
    assert_eq!(blend_gui_alpha(255, 200), 255);
}

/// The review's case: a 55 % alpha grey over a coloured background. Vanilla mixes the stored
/// bytes: 0.549 * 128/255 + 0.451 * 200/255 = 0.629 -> 160, 0.417 -> 106, 0.345 -> 88.
#[test]
fn blend_gui_pixel_55_percent_grey_over_colour() {
    assert_eq!(
        blend_gui_pixel([128, 128, 128, 140], [200, 80, 40]),
        [160, 106, 88, 255]
    );
    // Half-way grey over the same background.
    assert_eq!(
        blend_gui_pixel([128, 128, 128, 128], [200, 80, 40]),
        [164, 104, 84, 255]
    );
}

/// The sRGB-target fallback blends in linear space instead, which is not vanilla: the same
/// 55 % grey lands 6/255 brighter in red and 12 brighter in blue (0.549 * lin(128/255) +
/// 0.451 * lin(200/255) = 0.379 -> 166), so every translucent GUI pixel (the hotbar panel,
/// the vignette) takes on the background's encoding. On the GPU the drift is far larger: the
/// hotbar panel texel over a blue sky measured 106 in blue against vanilla's 55.
#[test]
fn blend_gui_pixel_linear_is_not_vanilla() {
    assert_eq!(
        blend_gui_pixel_linear([128, 128, 128, 140], [200, 80, 40]),
        [166, 110, 100, 255]
    );
    assert_ne!(
        blend_gui_pixel([128, 128, 128, 140], [200, 80, 40]),
        blend_gui_pixel_linear([128, 128, 128, 140], [200, 80, 40])
    );
    // Opaque pixels survive the fallback exactly: encode(decode(x)) == x.
    for c in [0u8, 1, 17, 64, 128, 200, 254, 255] {
        assert_eq!(
            blend_gui_pixel_linear([c, c, c, 255], [c, c, c]),
            [c, c, c, 255]
        );
    }
}

/// The GUI only converts colour when its target is sRGB-encoded; with a non-sRGB view it
/// writes vanilla's stored values directly.
#[test]
fn gui_writes_linear_only_for_srgb_targets() {
    assert!(gui_writes_linear(wgpu::TextureFormat::Bgra8UnormSrgb));
    assert!(gui_writes_linear(wgpu::TextureFormat::Rgba8UnormSrgb));
    assert!(!gui_writes_linear(wgpu::TextureFormat::Bgra8Unorm));
    assert!(!gui_writes_linear(wgpu::TextureFormat::Rgba8Unorm));
}

#[test]
fn shadow_colour_is_a_quarter() {
    assert_eq!(shadow_color(0xFFFF_FFFF), 0xFF3F_3F3F);
    assert_eq!(shadow_color(0x80FF_8040), 0x803F_2010);
}

#[test]
fn additive_blend_adds_source_to_destination() {
    // Vanilla's BlendFunction.ADDITIVE: ONE, ONE, both the colour and the alpha equation.
    let a = blend_state(GuiBlend::Additive);
    assert_eq!(a.color.src_factor, wgpu::BlendFactor::One);
    assert_eq!(a.color.dst_factor, wgpu::BlendFactor::One);
    assert_eq!(a.color.operation, wgpu::BlendOperation::Add);
    assert_eq!(a.alpha.src_factor, wgpu::BlendFactor::One);
    assert_eq!(a.alpha.dst_factor, wgpu::BlendFactor::One);
    assert_eq!(a.alpha.operation, wgpu::BlendOperation::Add);
    // The other modes are unchanged.
    assert_eq!(
        blend_state(GuiBlend::Normal).color.src_factor,
        wgpu::BlendFactor::SrcAlpha
    );
    assert_eq!(
        blend_state(GuiBlend::Multiply).color.dst_factor,
        wgpu::BlendFactor::OneMinusSrc
    );
    assert_eq!(
        blend_state(GuiBlend::Invert).color.src_factor,
        wgpu::BlendFactor::OneMinusDst
    );
}

#[test]
fn scissor_maps_to_framebuffer_and_clamps() {
    assert_eq!(
        to_framebuffer([10.0, 20.0, 30.0, 25.0], 2.0, (100, 100)),
        [20, 40, 40, 10]
    );
    assert_eq!(
        to_framebuffer([-5.0, 0.0, 500.0, 10.0], 1.0, (100, 50)),
        [0, 0, 100, 10]
    );
}

#[test]
fn shelf_packs_rows_and_reports_full() {
    let mut s = Shelf::default();
    let size = GUI_ATLAS_SIZE;
    assert_eq!(s.alloc_in(4000, 10, size), Some([0, 0]));
    assert_eq!(s.alloc_in(200, 20, size), Some([0, 10]));
    assert_eq!(s.alloc_in(size, size, size), None);
}

#[test]
fn shelf_continues_into_a_grown_atlas() {
    // `grow_atlas` doubles the side and keeps the shelf: the next row starts below the
    // old rows and may use the new width.
    let mut s = Shelf::default();
    assert_eq!(s.alloc_in(1000, 600, 1024), Some([0, 0]));
    assert_eq!(s.alloc_in(100, 500, 1024), None);
    assert_eq!(s.alloc_in(1500, 500, 2048), Some([0, 600]));
}

#[test]
fn flat_items_are_full_bright_and_block_faces_shade_like_vanilla() {
    let (l0, l1) = items_flat_lights();
    let front = lambert_shade(Vec3::Z, l0, l1);
    assert!(front > 0.95, "flat item front shade {front}");
    // Block in the GUI: top brightest, the two visible sides darker and different.
    let model = ItemModel {
        parts: vec![haste_assets::ItemModelPart {
            geometry: ItemGeometry::Elements(
                [
                    haste_assets::Direction::Up,
                    haste_assets::Direction::South,
                    haste_assets::Direction::East,
                    haste_assets::Direction::North,
                    haste_assets::Direction::West,
                    haste_assets::Direction::Down,
                ]
                .into_iter()
                .map(|d| haste_assets::BakedQuad::from_element(d, [0.0; 3], [1.0; 3], None, None))
                .map(|mut q| {
                    q.texture = "minecraft:block/stone".into();
                    q
                })
                .collect(),
            ),
            tints: vec![],
            gui: haste_assets::DisplayTransform {
                rotation: [30.0, 225.0, 0.0],
                translation: [0.0; 3],
                scale: [0.625; 3],
            },
            firstperson: haste_assets::DisplayTransform::IDENTITY,
            thirdperson: haste_assets::DisplayTransform::IDENTITY,
            thirdperson_left: haste_assets::DisplayTransform::IDENTITY,
            head: haste_assets::DisplayTransform::IDENTITY,
            ground: haste_assets::DisplayTransform::IDENTITY,
            fixed: haste_assets::DisplayTransform::IDENTITY,
            on_shelf: haste_assets::DisplayTransform::IDENTITY,
            gui_light_front: false,
            transformation: None,
        }],
    };
    let p = prepare_item(&model);
    assert_eq!(p.quads.len(), 3, "three faces visible");
    let shades: Vec<u8> = p.quads.iter().map(|q| q.color[0]).collect();
    let top = p
        .quads
        .iter()
        .find(|q| q.pos.iter().all(|v| v[1] < 8.5))
        .expect("top face above the centre")
        .color[0];
    assert_eq!(top, *shades.iter().max().unwrap(), "{shades:?}");
    // The cube fits the slot: 0.625 * 16 = 10 px wide at most (sqrt 2 diagonal ~14.1).
    for q in &p.quads {
        for v in q.pos {
            assert!(
                (0.0..=16.0).contains(&v[0]) && (0.0..=16.0).contains(&v[1]),
                "{v:?}"
            );
        }
    }
}

/// The borrowed atlas probes hash and compare like the owned keys they stand for: a sprite
/// probe finds only the sprite entry, a texture probe only the texture entry of the same id.
#[test]
fn tex_key_ref_finds_the_owned_key() {
    use super::atlas::{TexKey, TexKeyRef};
    let id = Identifier::minecraft("hud/heart/full");
    let mut keys: hashbrown::HashMap<TexKey, u32> = hashbrown::HashMap::new();
    keys.insert(TexKey::Sprite(id.clone()), 1);
    keys.insert(TexKey::Texture(id.clone()), 2);
    keys.insert(TexKey::Missing, 3);
    assert_eq!(keys.get(&TexKeyRef::Sprite(&id)), Some(&1));
    assert_eq!(keys.get(&TexKeyRef::Texture(&id)), Some(&2));
    assert_eq!(keys.get(&TexKeyRef::Key(&TexKey::Missing)), Some(&3));
    assert_eq!(
        keys.get(&TexKeyRef::Key(&TexKey::Sprite(id.clone()))),
        Some(&1)
    );
    let other = Identifier::minecraft("hud/heart/empty");
    assert_eq!(keys.get(&TexKeyRef::Sprite(&other)), None);
}
