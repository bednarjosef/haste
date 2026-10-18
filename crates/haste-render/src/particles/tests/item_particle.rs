//! Tests for the item particles (vanilla `BreakingItemParticle`) and their sprite table.

use super::*;
use glam::Vec3;
use haste_core::RegistryIndex;

/// An atlas that places every texture at (0, 0) (16x16) except ones whose name contains
/// `overlay` (at x 16), and whose missing tile sits at (32, 0).
struct FlatAtlas;

impl haste_assets::AtlasRegions for FlatAtlas {
    fn region_of(&self, texture_id: &str) -> haste_assets::AtlasRegion {
        let x = if texture_id.contains("overlay") {
            16
        } else {
            0
        };
        haste_assets::AtlasRegion {
            pixel_rect: [x, 0, 16, 16],
            sprite: 0,
            opaque: true,
            translucent: false,
        }
    }
}

/// A types table whose item 0 has two layers (a plain and an overlay sprite), for the
/// spawn tests; item 1 is not in the table (the missing sprite).
fn types_with_items() -> Arc<ParticleTypes> {
    let items = super::super::item_sprite::ItemParticleTable::build(
        &[(RegistryIndex(0), Identifier::minecraft("bread"))],
        &[vec![
            "minecraft:item/bread".to_owned(),
            "minecraft:item/bread_overlay".to_owned(),
        ]],
        &FlatAtlas,
        [32, 0, 16, 16],
    );
    Arc::new(ParticleTypes {
        types: Vec::new(),
        by_name: Default::default(),
        blocks: None,
        items,
        pixels: Vec::new(),
        size: 0,
    })
}

#[test]
fn spawn_item_follows_breaking_item_particle() {
    let mut particles = Particles::new(types_with_items());
    let vel = [0.0f32, 0.05, 0.0];
    particles.spawn_item(RegistryIndex(0), DVec3::new(1.0, 64.0, 2.0), vel);
    assert_eq!(particles.alive(), 1);
    let p = &particles.pool[0];
    assert_eq!(p.behaviour.source, "BreakingItemParticle");
    assert_eq!(p.behaviour.gravity, 1.0, "gravity 1");
    assert_eq!(p.behaviour.friction, 0.98);
    assert_eq!(p.color, [1.0; 3], "untinted");
    // The default quad halved (`quadSize /= 2`).
    assert!((0.05..=0.1).contains(&p.quad), "quad {}", p.quad);
    // The 4x4 uv sub-piece of one of the layers' sprites.
    let sprite = p.block_sprite.expect("an item sprite");
    let [x, w] = [sprite.rect[0], sprite.rect[2]];
    assert!(
        sprite.uv[0] >= x as f32 && sprite.uv[2] <= (x + w) as f32,
        "{:?}",
        sprite.uv
    );
    assert_eq!(
        sprite.uv[2] - sprite.uv[0],
        w as f32 / 4.0,
        "a quarter of the sprite"
    );
    assert_eq!(sprite.uv[3] - sprite.uv[1], sprite.rect[3] as f32 / 4.0);
    // The speed constructor's velocity scaled by 0.1 plus the caller's offset: the
    // constructor rescales into 0.06..=0.18 and then lifts y by 0.1, so the difference
    // stays under 0.028 with the lift's direction preserved.
    let diff = p.vel - Vec3::from(vel);
    assert!(diff.length() <= 0.028 + 1e-6, "diff {}", diff.length());
    assert!(diff.y > 0.0, "the constructor's y lift: {}", diff.y);
    // Lifetime `(int)(4 / (r * 0.9 + 0.1))`: 4..=40 ticks.
    assert!(
        (4.0..=40.0).contains(&p.lifetime),
        "lifetime {}",
        p.lifetime
    );
    // It falls like vanilla's generic tick (gravity 1, friction 0.98).
    let start = p.pos;
    let v = p.vel;
    particles.tick();
    assert_eq!(particles.pool[0].pos, start + v - Vec3::new(0.0, 0.04, 0.0));
}

/// An item past the table (a mod item, or one whose sprite was never packed) draws the
/// missing sprite, vanilla's `getAtlasOrThrow(ITEMS).missingSprite()` fallback.
#[test]
fn unknown_items_draw_the_missing_sprite() {
    let mut particles = Particles::new(types_with_items());
    particles.spawn_item(RegistryIndex(7), DVec3::ZERO, [0.0, 0.05, 0.0]);
    assert_eq!(particles.alive(), 1);
    let sprite = particles.pool[0].block_sprite.expect("an item sprite");
    assert_eq!(sprite.rect, [32, 0, 16, 16], "the missing sprite");
    // An item with no model layers at all does too.
    let items = super::super::item_sprite::ItemParticleTable::build(
        &[(RegistryIndex(0), Identifier::minecraft("chest"))],
        &[Vec::new()],
        &FlatAtlas,
        [32, 0, 16, 16],
    );
    let mut particles = Particles::new(Arc::new(ParticleTypes {
        types: Vec::new(),
        by_name: Default::default(),
        blocks: None,
        items,
        pixels: Vec::new(),
        size: 0,
    }));
    particles.spawn_item(RegistryIndex(0), DVec3::ZERO, [0.0, 0.05, 0.0]);
    assert_eq!(
        particles.pool[0].block_sprite.unwrap().rect,
        [32, 0, 16, 16]
    );
}

/// The default stack's item definition walk: a `select` takes its fallback, a condition
/// its `on_false`, a range dispatch the entry for value 0, a composite every part.
#[test]
fn leaf_models_walk_the_definition_tree() {
    let walk = |json: &str| {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut out = Vec::new();
        super::super::item_sprite::leaf_models(&v, &mut out);
        out
    };
    assert_eq!(
        walk(r#"{"type":"model","model":"a"}"#),
        vec!["a".to_owned()]
    );
    assert_eq!(
        walk(
            r#"{"type":"select","cases":[{"when":"x","model":{"type":"model","model":"b"}}],
               "fallback":{"type":"condition","on_true":{"type":"model","model":"c"},
               "on_false":{"type":"model","model":"d"}}}"#
        ),
        vec!["d".to_owned()]
    );
    assert_eq!(
        walk(
            r#"{"type":"range_dispatch","entries":[{"threshold":0.5,"model":{"type":"model","model":"e"}},
               {"threshold":0.0,"model":{"type":"model","model":"f"}}],"fallback":{"type":"model","model":"g"}}"#
        ),
        vec!["f".to_owned()]
    );
    assert_eq!(
        walk(
            r#"{"type":"composite","models":[{"type":"model","model":"h"},
               {"type":"special","base":"i"}]}"#
        ),
        vec!["h".to_owned()],
        "special renderers have no particle sprite"
    );
}

/// The item table over the real assets: bread's one generated layer, the potion's two
/// (overlay first, the model json's `layer0`), and a spawned crumb drawing bread's
/// sprite.
#[test]
#[ignore = "needs the client jar in the cache"]
fn item_table_resolves_real_sprites() {
    let jar = haste_core::paths::client_jar();
    if !jar.exists() {
        eprintln!("ignored: needs the client jar in the cache");
        return;
    }
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3")
            .unwrap();
    let items = [
        (RegistryIndex(0), Identifier::minecraft("bread")),
        (RegistryIndex(1), Identifier::minecraft("potion")),
    ];
    let types = ParticleTypes::build(&mut assets, &[], None, &items).unwrap();
    assert_eq!(types.items.sprites(RegistryIndex(0)).len(), 1, "bread");
    let bread = types.items.sprites(RegistryIndex(0))[0];
    assert_eq!((bread[2], bread[3]), (16, 16), "bread's sprite");
    // The potion's generated layers: the overlay and the bottle.
    let layers = types.items.sprites(RegistryIndex(1));
    assert_eq!(layers.len(), 2, "potion layers: {layers:?}");
    assert_ne!(layers[0], layers[1]);
    // A crumb of bread draws a quarter of bread's sprite.
    let mut particles = Particles::new(Arc::new(types));
    particles.spawn_item(
        RegistryIndex(0),
        DVec3::new(1.0, 64.0, 2.0),
        [0.0, 0.05, 0.0],
    );
    let sprite = particles.pool[0].block_sprite.unwrap();
    assert_eq!(sprite.rect, bread);
}
