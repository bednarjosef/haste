//! Tests for `skin` (default skins) and `avatar` (model parts, cloak, cape swing).
use glam::{DVec3, Mat3, Vec3};
use uuid::Uuid;

use crate::avatar::{
    cape_rotation, cape_state, hide_layers, CapeInput, CapeState, Cloak, ModelParts,
    PlayerModelPart,
};
use crate::skin::{default_skin, java_uuid_hash, PlayerModelType};
use haste_core::model::PoseOverrides;

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap()
}

/// Expected values computed independently from Java's `UUID.hashCode` (xor of the halves,
/// folded) and `Math.floorMod(hash, 18)`.
#[test]
fn default_skin_follows_the_uuid_hash() {
    let cases = [
        // Notch: hash -369792882, floorMod 18 = 0 -> slim alex
        (
            "069a79f4-44e9-4726-a5be-fca90e38aaf5",
            -369792882,
            "entity/player/slim/alex",
            true,
        ),
        // jeb_: 1946714239 -> 1 -> slim ari
        (
            "853c80ef-3c37-49fd-aa49-938b674adae6",
            1946714239,
            "entity/player/slim/ari",
            true,
        ),
        // 61699b2e-...: -291457983 -> 15 -> wide steve
        (
            "61699b2e-d327-4a01-9f1e-0ea8c3f06bc6",
            -291457983,
            "entity/player/wide/steve",
            false,
        ),
        // f84c6a79-...: -1639111118 -> 4 -> slim makena
        (
            "f84c6a79-0a4e-45e0-879b-cd49ebd4c4e2",
            -1639111118,
            "entity/player/slim/makena",
            true,
        ),
        // the nil UUID hashes to 0 -> slim alex; ...0011 to 17 -> wide zuri
        (
            "00000000-0000-0000-0000-000000000000",
            0,
            "entity/player/slim/alex",
            true,
        ),
        (
            "00000000-0000-0000-0000-000000000011",
            17,
            "entity/player/wide/zuri",
            false,
        ),
    ];
    for (id, hash, texture, slim) in cases {
        assert_eq!(java_uuid_hash(uuid(id)), hash, "{id}");
        let skin = default_skin(uuid(id));
        assert_eq!(
            skin.body.to_string(),
            format!("minecraft:{texture}"),
            "{id}"
        );
        assert_eq!(skin.model.is_slim(), slim, "{id}");
        assert!(skin.cape.is_none());
    }
}

#[test]
fn model_type_from_services_name() {
    assert_eq!(
        PlayerModelType::by_legacy_services_name(Some("slim")),
        PlayerModelType::Slim
    );
    assert_eq!(
        PlayerModelType::by_legacy_services_name(Some("default")),
        PlayerModelType::Wide
    );
    assert_eq!(
        PlayerModelType::by_legacy_services_name(None),
        PlayerModelType::Wide
    );
}

#[test]
fn model_parts_hide_the_matching_layers() {
    assert!(ModelParts::ALL.shown(PlayerModelPart::Hat));
    assert!(!ModelParts(0).shown(PlayerModelPart::Cape));
    let mut pose = PoseOverrides::default();
    hide_layers(ModelParts::ALL, &mut pose);
    assert_eq!(pose.hidden().count(), 0);
    // everything but the hat (bit 6) and the right sleeve (bit 3)
    hide_layers(ModelParts(0x7f & !(1 << 6) & !(1 << 3)), &mut pose);
    assert_eq!(
        pose.hidden().collect::<Vec<_>>(),
        vec![
            haste_core::parts::intern("hat"),
            haste_core::parts::intern("right_sleeve")
        ]
    );
}

#[test]
fn cloak_trails_by_a_quarter_and_snaps_far() {
    let mut c = Cloak::default();
    // vanilla starts the cloak at the origin: a far first position snaps
    c.tick(DVec3::new(100.0, 64.0, 4.0));
    assert_eq!(c.pos, DVec3::new(100.0, 64.0, 1.0));
    assert_eq!(c.pos_o, DVec3::new(100.0, 64.0, 0.0));
    c.tick(DVec3::new(104.0, 64.0, 5.0));
    assert_eq!(c.pos, DVec3::new(101.0, 64.0, 2.0));
    assert_eq!(c.at(0.5), DVec3::new(100.5, 64.0, 1.5));
}

#[test]
fn cape_state_follows_extract_cape_state() {
    // Standing still: no flap, no lean.
    let rest = cape_state(&CapeInput {
        cloak: DVec3::new(0.0, 64.0, 0.0),
        pos: DVec3::new(0.0, 64.0, 0.0),
        body_yaw: 0.0,
        fall_flying_scale: 0.0,
        bob: 0.0,
        walk_dist: 0.0,
    });
    assert_eq!(rest, CapeState::default());
    // Walking forward (+z at yaw 0) with the cloak 0.3 behind: lean 30, clamped flap from a
    // cloak 1 block above (10 -> 10), and no sideways lean.
    let walk = cape_state(&CapeInput {
        cloak: DVec3::new(0.0, 65.0, -0.3),
        pos: DVec3::new(0.0, 64.0, 0.0),
        body_yaw: 0.0,
        fall_flying_scale: 0.0,
        bob: 0.0,
        walk_dist: 0.0,
    });
    // forward is (sin 0, -cos 0) = (0, -1): lean = (-0.3 * -1) * 100
    assert!((walk.lean - 30.0).abs() < 1e-3, "{walk:?}");
    assert!((walk.flap - 10.0).abs() < 1e-3);
    assert!(walk.lean2.abs() < 1e-3);
    // Limits: flap -6..32, lean 0..150, lean2 -20..20; the walk term adds sin(6 w) * 32 * bob.
    let wild = cape_state(&CapeInput {
        cloak: DVec3::new(5.0, 50.0, -5.0),
        pos: DVec3::new(0.0, 64.0, 0.0),
        body_yaw: 0.0,
        fall_flying_scale: 0.0,
        bob: 0.1,
        walk_dist: 0.25,
    });
    assert!((wild.lean - 150.0).abs() < 1e-3);
    assert!((wild.lean2 + 20.0).abs() < 1e-3);
    let flap = -6.0 + (0.25f32 * 6.0).sin() * 32.0 * 0.1;
    assert!((wild.flap - flap).abs() < 1e-4);
    // Fall flying scales the lean away.
    let flying = cape_state(&CapeInput {
        fall_flying_scale: 1.0,
        ..CapeInput {
            cloak: DVec3::new(0.0, 64.0, -0.3),
            pos: DVec3::new(0.0, 64.0, 0.0),
            body_yaw: 0.0,
            fall_flying_scale: 0.0,
            bob: 0.0,
            walk_dist: 0.0,
        }
    });
    assert_eq!(flying.lean, 0.0);
}

/// The additive cape rotation, added to the layer's base (0, pi, 0) and composed the renderer's
/// way (Z, then Y, then X), reproduces `PlayerCapeModel.setupAnim`'s
/// X(6 + lean/2 + flap) Z(lean2/2) Y(180 - lean2/2).
#[test]
fn cape_rotation_matches_rotate_by() {
    for s in [
        CapeState::default(),
        CapeState {
            flap: 12.0,
            lean: 40.0,
            lean2: -15.0,
        },
        CapeState {
            flap: -6.0,
            lean: 150.0,
            lean2: 20.0,
        },
    ] {
        let r = Vec3::new(0.0, std::f32::consts::PI, 0.0) + cape_rotation(&s);
        let got =
            Mat3::from_rotation_z(r.z) * Mat3::from_rotation_y(r.y) * Mat3::from_rotation_x(r.x);
        let want = Mat3::from_rotation_x((6.0 + s.lean / 2.0 + s.flap).to_radians())
            * Mat3::from_rotation_z((s.lean2 / 2.0).to_radians())
            * Mat3::from_rotation_y((180.0 - s.lean2 / 2.0).to_radians());
        for (g, w) in got.to_cols_array().iter().zip(want.to_cols_array()) {
            assert!((g - w).abs() < 1e-5, "{s:?}: {got:?} vs {want:?}");
        }
    }
}

/// The cape layer comes from the recorder's dump (`player#cape`) with the half-height texture.
#[test]
fn cape_model_from_the_dump() {
    if crate::model_dump::library().is_none() {
        eprintln!("skipped: no entity model dump in the cache");
        return;
    }
    let tex = haste_core::Identifier::minecraft("capes/abc");
    let m = crate::skin::cape_model(&tex).expect("player#cape layer");
    assert_eq!(m.texture, tex);
    assert_eq!((m.tex_width, m.tex_height), (64, 32));
    let body = m.root.children.iter().find(|p| p.name == "body").unwrap();
    assert_eq!(body.pivot, Vec3::new(0.0, -24.0, 0.0));
    let cape = body.children.iter().find(|p| p.name == "cape").unwrap();
    assert_eq!(cape.pivot, Vec3::new(0.0, 0.0, 2.0));
    assert_eq!(cape.cubes[0].size, Vec3::new(10.0, 16.0, 1.0));
}
