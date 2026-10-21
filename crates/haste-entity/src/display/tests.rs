//! The display entities' data decoding, interpolation and transformation, against the
//! numbers vanilla's `Display`, `Transformation` and JOML produce.

use std::f32::consts::FRAC_1_SQRT_2;

use glam::{DVec3, Vec3, Vec4};
use haste_core::display::{Billboard, DisplayContent, ItemDisplayContext};
use haste_core::entity::{EntitySpawn, ItemStackData, MetadataEntry, MetadataValue};
use haste_core::{BlockStateId, Component, EntityId, RegistryIndex};

use super::text_display::{lerp_int, TextRenderState, BACKGROUND_COLOR, TEXT_OPACITY};
use super::transformation::{rotation_matrix, slerp};
use super::*;
use crate::dims::EntityDimensions;
use crate::lerp::{LerpState, LerpTarget};
use crate::store::EntityStore;

fn entries(pairs: &[(u8, MetadataValue)]) -> Vec<MetadataEntry> {
    pairs
        .iter()
        .map(|(index, value)| MetadataEntry {
            index: *index,
            value: value.clone(),
        })
        .collect()
}

fn floats(type_id: i32, v: &[f32]) -> MetadataValue {
    let raw: Vec<u8> = v.iter().flat_map(|f| f.to_be_bytes()).collect();
    MetadataValue::Other {
        type_id,
        raw: raw.into(),
    }
}

fn vector3(v: [f32; 3]) -> MetadataValue {
    floats(39, &v)
}

fn quaternion(q: [f32; 4]) -> MetadataValue {
    floats(40, &q)
}

fn spawn(kind: RegistryIndex) -> EntitySpawn {
    EntitySpawn {
        id: EntityId(7),
        uuid: uuid::Uuid::nil(),
        kind,
        pos: DVec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    }
}

/// A display entity of `kind` at the origin (the type registry is not needed: the state is
/// attached by hand).
fn display_entity(kind: DisplayKind) -> Entity {
    let mut e = Entity::new(&spawn(RegistryIndex(0)), &EntityDimensions::default());
    e.display = Some(Box::new(DisplayState::new(kind)));
    e
}

/// Applies a metadata packet the way `EntityStore::set_metadata` does.
fn set(e: &mut Entity, pairs: &[(u8, MetadataValue)]) {
    let list = entries(pairs);
    e.metadata.apply(&list);
    on_metadata(e, &list);
}

/// One client tick: `commonTick`'s `tickCount++`, then `Display.tick`.
fn tick(e: &mut Entity) {
    e.tick_count += 1;
    let Entity {
        display,
        metadata,
        tick_count,
        ..
    } = e;
    display.as_mut().unwrap().tick(metadata, *tick_count);
}

fn assert_vec3(a: Vec3, b: Vec3) {
    assert!((a - b).abs().max_element() < 1e-5, "{a} != {b}");
}

#[test]
fn ids_decode_like_vanilla_by_id_maps() {
    // `BillboardConstraints.BY_ID` and `ItemDisplayContext.BY_ID`: out of range is id 0.
    assert_eq!(Billboard::from_id(3), Billboard::Center);
    assert_eq!(Billboard::from_id(1), Billboard::Vertical);
    assert_eq!(Billboard::from_id(9), Billboard::Fixed);
    assert_eq!(ItemDisplayContext::from_id(8), ItemDisplayContext::Fixed);
    assert_eq!(ItemDisplayContext::from_id(6), ItemDisplayContext::Gui);
    assert_eq!(ItemDisplayContext::from_id(-3), ItemDisplayContext::None);
    assert_eq!(
        DisplayKind::for_name("minecraft:text_display"),
        Some(DisplayKind::Text)
    );
    assert_eq!(DisplayKind::for_name("minecraft:pig"), None);
}

#[test]
fn nothing_renders_before_the_first_render_state() {
    // `Display.renderState` stays null until a render-state field changes: an entity whose
    // data are all defaults draws nothing (`DisplayRenderer.submit`).
    let mut e = display_entity(DisplayKind::Block);
    tick(&mut e);
    assert!(decoration(&e, 0.0).is_none());
    set(
        &mut e,
        &[(SUBCLASS_FIRST, MetadataValue::BlockState(BlockStateId(1)))],
    );
    assert!(decoration(&e, 0.0).is_none(), "applied on the next tick");
    tick(&mut e);
    let (d, radius, strength) = decoration(&e, 0.0).unwrap();
    assert_eq!(
        d.content,
        DisplayContent::Block {
            state: BlockStateId(1),
            light_emission: 0
        }
    );
    // The synched defaults: identity transformation, fixed billboard, no brightness override,
    // view range 1, no culling box, shadow radius 0 and strength 1.
    assert_eq!(d.transformation, glam::Mat4::IDENTITY);
    assert_eq!(d.billboard, Billboard::Fixed);
    assert_eq!(d.brightness, None);
    assert_eq!((d.view_range, d.width, d.height), (1.0, 0.0, 0.0));
    assert_eq!(d.glow_color_override, -1);
    assert_eq!((radius, strength), (0.0, 1.0));
}

#[test]
fn synched_data_decodes() {
    let mut e = display_entity(DisplayKind::Item);
    let (s, c) = (0.5f32.sin(), 0.5f32.cos());
    set(
        &mut e,
        &[
            (TRANSLATION, vector3([1.0, 2.0, 3.0])),
            (SCALE, vector3([2.0, 2.0, 2.0])),
            (LEFT_ROTATION, quaternion([0.0, s, 0.0, c])),
            (BILLBOARD, MetadataValue::Byte(3)),
            // `Brightness(block 15, sky 7).pack()`: `block << 4 | sky << 20`.
            (
                BRIGHTNESS_OVERRIDE,
                MetadataValue::VarInt(15 << 4 | 7 << 20),
            ),
            (VIEW_RANGE, MetadataValue::Float(2.5)),
            (SHADOW_RADIUS, MetadataValue::Float(0.75)),
            (SHADOW_STRENGTH, MetadataValue::Float(0.5)),
            (WIDTH, MetadataValue::Float(3.0)),
            (HEIGHT, MetadataValue::Float(1.5)),
            (GLOW_COLOR_OVERRIDE, MetadataValue::VarInt(0xFF0000)),
            (
                SUBCLASS_FIRST,
                MetadataValue::ItemStack(ItemStackData {
                    count: 1,
                    item: Some(RegistryIndex(42)),
                    components_raw: Default::default(),
                }),
            ),
            (ITEM_DISPLAY_CONTEXT, MetadataValue::Byte(8)),
        ],
    );
    tick(&mut e);
    let (d, radius, strength) = decoration(&e, 0.3).unwrap();
    assert_eq!(d.billboard, Billboard::Center);
    assert_eq!(d.brightness, Some((15, 7)));
    assert_eq!((d.view_range, d.width, d.height), (2.5, 3.0, 1.5));
    assert_eq!(d.glow_color_override, 0xFF0000);
    assert_eq!((radius, strength), (0.75, 0.5));
    assert_eq!(
        d.content,
        DisplayContent::Item {
            item: Some(RegistryIndex(42)),
            context: ItemDisplayContext::Fixed
        }
    );
    // translation(1,2,3) * rotateY(1 rad) * scale(2): the unit x axis lands at
    // (2cos1, 0, -2sin1) + (1,2,3).
    let p = d.transformation.transform_point3(Vec3::X);
    assert_vec3(
        p,
        Vec3::new(1.0 + 2.0 * 1f32.cos(), 2.0, 3.0 - 2.0 * 1f32.sin()),
    );
}

#[test]
fn transformation_composes_like_joml() {
    // `Transformation.compose`: translation, then the left rotation, the scale, the right
    // rotation, each applied after (inside) the previous one.
    let t = Transformation {
        translation: Vec3::new(0.0, 1.0, 0.0),
        left_rotation: Vec4::new(0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2),
        scale: Vec3::new(2.0, 1.0, 1.0),
        right_rotation: Vec4::new(0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2),
    };
    // x -> rotY(90): -z -> scale x2 leaves z -> rotZ(90) leaves z -> +1 y.
    assert_vec3(
        t.matrix().transform_point3(Vec3::X),
        Vec3::new(0.0, 1.0, -1.0),
    );
    // z -> rotY(90): +x -> scale: 2x -> rotZ(90): 2y -> +1 y.
    assert_vec3(
        t.matrix().transform_point3(Vec3::Z),
        Vec3::new(0.0, 3.0, 0.0),
    );
    // JOML's homogeneous rotation: a non-unit quaternion scales by its squared norm.
    let m = rotation_matrix(Vec4::new(0.0, 0.0, 0.0, 2.0));
    assert_vec3(m.transform_point3(Vec3::ONE), Vec3::splat(4.0));
}

#[test]
fn slerp_follows_joml() {
    let identity = Vec4::new(0.0, 0.0, 0.0, 1.0);
    // 180 degrees about z, halfway: 90 degrees about z.
    let half = slerp(identity, Vec4::new(0.0, 0.0, 1.0, 0.0), 0.5);
    assert!((half - Vec4::new(0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2)).length() < 1e-6);
    // The shorter arc: a negated target slerps the same way.
    let q = Vec4::new(0.0, 0.0, -FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
    let s = slerp(identity, q, 0.5);
    let angle = 2.0 * s.z.atan2(s.w);
    assert!(
        (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-6,
        "{angle}"
    );
    // Nearly parallel: a plain lerp, no normalization (JOML's 1e-6 branch).
    let near = slerp(identity, identity * 2.0, 0.25);
    assert!((near - Vec4::new(0.0, 0.0, 0.0, 1.25)).length() < 1e-6);
}

#[test]
fn interpolation_progress_runs_from_the_start_tick() {
    // `calculateInterpolationProgress`: (tickCount - start + partial) / duration, clamped;
    // 1 without a duration.
    let mut e = display_entity(DisplayKind::Block);
    set(
        &mut e,
        &[(SUBCLASS_FIRST, MetadataValue::BlockState(BlockStateId(1)))],
    );
    tick(&mut e);
    let d = e.display.as_ref().unwrap();
    assert_eq!(d.interpolation_progress(e.tick_count, 0.5), 1.0);
    set(
        &mut e,
        &[
            (INTERPOLATION_DURATION, MetadataValue::VarInt(10)),
            (INTERPOLATION_START_DELTA, MetadataValue::VarInt(2)),
        ],
    );
    tick(&mut e);
    let start = e.tick_count as i64 + 2;
    let d = e.display.as_ref().unwrap();
    assert_eq!(d.interpolation_start_client_tick, start);
    assert_eq!(d.interpolation_progress(e.tick_count, 0.5), 0.0);
    assert_eq!(d.interpolation_progress(e.tick_count + 5, 0.5), 0.35);
    assert_eq!(d.last_progress(), 0.35);
    assert_eq!(d.interpolation_progress(e.tick_count + 40, 0.0), 1.0);
}

#[test]
fn a_new_transformation_interpolates_from_where_the_last_frame_was() {
    let mut e = display_entity(DisplayKind::Block);
    set(
        &mut e,
        &[
            (SUBCLASS_FIRST, MetadataValue::BlockState(BlockStateId(1))),
            (INTERPOLATION_DURATION, MetadataValue::VarInt(10)),
        ],
    );
    tick(&mut e);
    let at = |e: &Entity, partial| {
        let (d, ..) = decoration(e, partial).unwrap();
        d.transformation.transform_point3(Vec3::ZERO)
    };
    // The first state is fresh (nothing to interpolate from): constant.
    assert_vec3(at(&e, 0.0), Vec3::ZERO);
    // Translate to x = 10 starting now: progress (0 + partial) / 10 along the way.
    set(
        &mut e,
        &[
            (TRANSLATION, vector3([10.0, 0.0, 0.0])),
            (INTERPOLATION_START_DELTA, MetadataValue::VarInt(0)),
        ],
    );
    tick(&mut e);
    assert_vec3(at(&e, 0.5), Vec3::new(0.5, 0.0, 0.0));
    tick(&mut e);
    tick(&mut e);
    // Two ticks on, partial 0: progress 0.2 (this frame becomes `lastProgress`).
    assert_vec3(at(&e, 0.0), Vec3::new(2.0, 0.0, 0.0));
    // A new target mid-flight starts from the last frame's value (x = 2), not the old end.
    set(
        &mut e,
        &[
            (TRANSLATION, vector3([2.0, 8.0, 0.0])),
            (INTERPOLATION_START_DELTA, MetadataValue::VarInt(0)),
        ],
    );
    tick(&mut e);
    assert_vec3(at(&e, 0.0), Vec3::new(2.0, 0.0, 0.0));
    assert_vec3(at(&e, 5.0), Vec3::new(2.0, 4.0, 0.0));
    assert_vec3(at(&e, 10.0), Vec3::new(2.0, 8.0, 0.0));
}

#[test]
fn shadow_interpolates_only_with_a_duration() {
    let mut e = display_entity(DisplayKind::Block);
    set(&mut e, &[(SHADOW_RADIUS, MetadataValue::Float(1.0))]);
    tick(&mut e);
    set(&mut e, &[(SHADOW_RADIUS, MetadataValue::Float(3.0))]);
    tick(&mut e);
    // No duration: the new value at once (a fresh render state).
    assert_eq!(decoration(&e, 0.5).unwrap().1, 3.0);
    set(
        &mut e,
        &[
            (INTERPOLATION_DURATION, MetadataValue::VarInt(4)),
            (INTERPOLATION_START_DELTA, MetadataValue::VarInt(0)),
            (SHADOW_RADIUS, MetadataValue::Float(1.0)),
        ],
    );
    tick(&mut e);
    // `LinearFloatInterpolator(3, 1)` at (0 + 1) / 4.
    assert_eq!(decoration(&e, 1.0).unwrap().1, 2.5);
}

#[test]
fn text_state_interpolates_opacity_and_background() {
    // `Mth.lerpInt` floors: from -1 to 64 halfway is -1 + floor(32.5) = 31.
    assert_eq!(lerp_int(0.5, -1, 64), 31);
    assert_eq!(lerp_int(0.5, 64, -1), 31);
    let mut e = display_entity(DisplayKind::Text);
    set(
        &mut e,
        &[
            (
                SUBCLASS_FIRST,
                MetadataValue::Component(Component::text("hi")),
            ),
            (INTERPOLATION_DURATION, MetadataValue::VarInt(2)),
        ],
    );
    tick(&mut e);
    let text = |e: &Entity, partial| match decoration(e, partial).unwrap().0.content {
        DisplayContent::Text(t) => t,
        other => panic!("{other:?}"),
    };
    // The synched defaults: width 200, opaque, the 0x40000000 background, no flags.
    let t = text(&e, 0.0);
    assert_eq!(*t.text, Component::text("hi"));
    assert_eq!(
        (t.line_width, t.opacity, t.background, t.flags),
        (200, -1, 0x4000_0000, 0)
    );
    set(
        &mut e,
        &[
            (TEXT_OPACITY, MetadataValue::Byte(64)),
            (
                BACKGROUND_COLOR,
                MetadataValue::VarInt(0xC0FF_8000_u32 as i32),
            ),
            (INTERPOLATION_START_DELTA, MetadataValue::VarInt(0)),
        ],
    );
    tick(&mut e);
    let t = text(&e, 1.0);
    assert_eq!(t.opacity, 31);
    // `ARGB.srgbLerp`, each channel `lerpInt` at 0.5: a 0x40 -> 0xC0 = 0x80, r 0 -> 0xFF =
    // 0x7F, g 0 -> 0x80 = 0x40, b 0.
    assert_eq!(t.background, 0x807F_4000);
    let state = TextRenderState::build(&e.metadata, None, 0.0);
    assert_eq!(state.opacity, (64, 64));
}

#[test]
fn a_zero_step_window_snaps_and_a_longer_one_eases() {
    // `LinearInterpolationHandler` with 0 steps snaps (`snapTo`); with 4 it eases over 4
    // ticks, each covering 1/remaining of what is left.
    let mut lerp = LerpState::new(DVec3::ZERO, 0.0, 0.0);
    lerp.set_steps(0);
    let target = DVec3::new(4.0, 0.0, 0.0);
    let snap = lerp.interpolate_to(Some(target), None, (DVec3::ZERO, 0.0, 0.0));
    assert_eq!(snap, Some((target, 0.0, 0.0)));
    lerp.set_steps(4);
    let eased = lerp.interpolate_to(Some(DVec3::ZERO), None, (target, 0.0, 0.0));
    assert_eq!(eased, None);
    let (mut pos, mut prev) = (target, target);
    let (mut yaw, mut prev_yaw, mut pitch, mut prev_pitch) = (0.0, 0.0, 0.0, 0.0);
    let mut xs = Vec::new();
    for _ in 0..5 {
        lerp.tick(&mut LerpTarget {
            pos: &mut pos,
            prev_pos: &mut prev,
            yaw: &mut yaw,
            prev_yaw: &mut prev_yaw,
            pitch: &mut pitch,
            prev_pitch: &mut prev_pitch,
        });
        xs.push(pos.x);
    }
    assert_eq!(xs, vec![3.0, 2.0, 1.0, 0.0, 0.0]);
}

/// The kind of an entity type name, through the real registries.json.
fn kind_of(name: &str) -> Option<RegistryIndex> {
    crate::registry::entity_type_names()?
        .iter()
        .find(|(_, n)| **n == name)
        .map(|(k, _)| *k)
}

/// Through the store: a text display snaps to position packets until its teleport duration
/// gives it a window (`Display.onSyncedDataUpdated` -> `setInterpolationLength`).
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn teleport_duration_sets_the_position_window() {
    let Some(kind) = kind_of("minecraft:text_display") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let mut store = EntityStore::new(EntityDimensions::default());
    store.spawn(&spawn(kind));
    let id = EntityId(7);
    assert!(store.get(id).unwrap().display.is_some());
    let target = DVec3::new(8.0, 0.0, 0.0);
    store.teleport(id, target, 0.0, 0.0, false);
    let e = store.get(id).unwrap();
    assert_eq!((e.pos, e.prev_pos), (target, target), "snapped");
    store.set_metadata(
        id,
        &entries(&[(POS_ROT_INTERPOLATION_DURATION, MetadataValue::VarInt(4))]),
    );
    store.teleport(id, DVec3::ZERO, 0.0, 0.0, false);
    assert_eq!(
        store.get(id).unwrap().pos,
        target,
        "eases from the next tick"
    );
    store.tick();
    assert_eq!(store.get(id).unwrap().pos, DVec3::new(6.0, 0.0, 0.0));
}
