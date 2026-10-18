// Banners (vanilla `BannerRenderer`): the model per attachment (`BannerModel` +
// `BannerFlagModel`), the transform (`modelTransformation`), the flag's wave and the layered
// pattern submit (`submitBanner` then `submitPatterns`).

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use haste_core::model::{Cube, CubeModel, ModelPart, PoseOverrides};
use haste_core::{parts, BlockPos, Identifier};

use super::{BlockEntityInstance, BlockEntityKind, DYE_DIFFUSE_COLOR};
use crate::entity::EntityVertex;

/// Vanilla `BannerModel.createBodyLayer` + `BannerFlagModel.createFlagLayer` as one model per
/// attachment: standing has the pole (texOffs 44,0) and the bar at y -44; wall banners drop
/// the pole and hang the bar at y -20.5, z 9.5. The flag (texOffs 0,0) hangs from its pivot
/// (standing (0, -44, 0), wall (0, -20.5, 10.5)) and waves by `BannerFlagModel.setupAnim`.
pub(super) fn banner_model(wall: bool) -> &'static CubeModel {
    static STANDING: OnceLock<CubeModel> = OnceLock::new();
    static WALL: OnceLock<CubeModel> = OnceLock::new();
    let model = if wall { &WALL } else { &STANDING };
    model.get_or_init(|| {
        let mut children = Vec::new();
        if !wall {
            children.push(ModelPart {
                name: "pole".into(),
                id: parts::intern("pole"),
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                // addBox(-1, -42, -1, 2, 42, 2), texOffs(44, 0)
                cubes: vec![Cube {
                    origin: Vec3::new(-1.0, -42.0, -1.0),
                    size: Vec3::new(2.0, 42.0, 2.0),
                    inflate: 0.0,
                    uv: [44, 0],
                    mirror: false,
                }],
                children: Vec::new(),
            });
        }
        let bar_y = if wall { -20.5 } else { -44.0 };
        let bar_z = if wall { 9.5 } else { -1.0 };
        children.push(ModelPart {
            name: "bar".into(),
            id: parts::intern("bar"),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            // addBox(-10, bar_y, bar_z, 20, 2, 2), texOffs(0, 42)
            cubes: vec![Cube {
                origin: Vec3::new(-10.0, bar_y, bar_z),
                size: Vec3::new(20.0, 2.0, 2.0),
                inflate: 0.0,
                uv: [0, 42],
                mirror: false,
            }],
            children: Vec::new(),
        });
        let flag_pivot = if wall {
            Vec3::new(0.0, -20.5, 10.5)
        } else {
            Vec3::new(0.0, -44.0, 0.0)
        };
        children.push(ModelPart {
            name: "flag".into(),
            id: parts::intern("flag"),
            pivot: flag_pivot,
            rotation: Vec3::ZERO,
            visible: true,
            // addBox(-10, 0, -2, 20, 40, 1), texOffs(0, 0)
            cubes: vec![Cube {
                origin: Vec3::new(-10.0, 0.0, -2.0),
                size: Vec3::new(20.0, 40.0, 1.0),
                inflate: 0.0,
                uv: [0, 0],
                mirror: false,
            }],
            children: Vec::new(),
        });
        CubeModel {
            texture: Identifier::minecraft("entity/banner/banner_base"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
                id: parts::ROOT,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: Vec::new(),
                children,
            },
        }
    })
}

/// Vanilla `BannerRenderer.modelTransformation`: translate (0.5, 0, 0.5), rotate `-angle`
/// about y, scale (2/3, -2/3, -2/3) — the y/z flip maps the model's y-down entity convention
/// onto the world — and the model renderer's 1/16 from pixels to blocks. `angle` is the
/// `rotation` segment (standing) or the wall `facing.toYRot()`.
pub(super) fn banner_root(pos: BlockPos, angle: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
        * Mat4::from_rotation_y((-angle).to_radians())
        * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0))
        * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// Vanilla `BannerFlagModel.setupAnim`: the flag swings by
/// `(-0.0125 + 0.01 * cos(2 * PI * phase)) * PI` radians about its pivot's x axis.
pub fn banner_flag_rotation(phase: f32) -> f32 {
    (-0.0125 + 0.01 * (std::f32::consts::TAU * phase).cos()) * std::f32::consts::PI
}

/// Vanilla `BannerRenderer.extractRenderState`'s phase:
/// `(floorMod(x*7 + y*9 + z*13 + gameTime, 100) + partial) / 100`.
pub fn banner_wave_phase(pos: BlockPos, game_time: i64, partial: f32) -> f32 {
    let hash = pos.x * 7 + pos.y * 9 + pos.z * 13;
    (hash.wrapping_add(game_time as i32).rem_euclid(100) as f32 + partial) / 100.0
}
/// Poses one banner's layers, following vanilla `BannerRenderer.submitBanner`: the model
/// (pole, bar, flag) through `modelTransformation`, the flag waving by the phase, then one
/// pass per pattern layer (the base sprite untinted, the `base` pattern in the banner's
/// colour, then up to 16 patterns in their own), each through the same geometry with its
/// texture and diffuse tint. Draw order is back to front: bar and pole first, then the flag.
pub(crate) fn emit_banner(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    camera: glam::DVec3,
    angle: f32,
    rect_of: &mut dyn FnMut(&Identifier) -> [u32; 4],
) {
    use crate::entity::emit_part_tinted;
    let wall = matches!(inst.kind, BlockEntityKind::Banner { wall: true });
    let model = banner_model(wall);
    let (sky, block) = inst.light;
    let light = (block, sky);
    let mut pose = PoseOverrides::default();
    pose.set_rotation(
        parts::intern("flag"),
        Vec3::new(banner_flag_rotation(inst.banner_phase), 0.0, 0.0),
    );
    let root = banner_root(inst.pos, angle, camera);
    let Some(data) = inst.banner.clone() else {
        return;
    };
    // (texture, tint as linear rgb) per layer, vanilla's submitBanner then submitPatterns.
    let mut layers: Vec<(Identifier, [f32; 3])> = Vec::new();
    let diffuse = |c: u32| {
        [
            ((c >> 16) & 0xFF) as f32 / 255.0,
            ((c >> 8) & 0xFF) as f32 / 255.0,
            (c & 0xFF) as f32 / 255.0,
        ]
    };
    // The base sprite (banner_base) is drawn white; the `base` pattern in the banner colour.
    layers.push((model.texture.clone(), [1.0; 3]));
    layers.push((
        Identifier::minecraft("entity/banner/base"),
        diffuse(DYE_DIFFUSE_COLOR[data.base as usize % 16]),
    ));
    for layer in data.layers.iter().take(16) {
        // The pattern's texture is `entity/banner/<asset id>`; the registry id's path is the
        // asset id for every vanilla pattern.
        let tex = Identifier::minecraft(&format!("entity/banner/{}", layer.pattern.path()));
        layers.push((tex, diffuse(DYE_DIFFUSE_COLOR[layer.color as usize % 16])));
    }
    for (texture, tint) in &layers {
        let rect = rect_of(texture);
        for part in &model.root.children {
            emit_part_tinted(out, model, part, &pose, root, rect, light, 0.0, *tint);
        }
    }
}
