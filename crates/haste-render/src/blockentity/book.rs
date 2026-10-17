// The book model shared by the lectern and the enchanting table (vanilla `BookModel`): its layer
// (`createBodyLayer`), its pose (`setupAnim`, `State.forAnimation`) and texture.

use std::sync::OnceLock;

use glam::Vec3;
use haste_core::model::{CubeModel, ModelPart};
use haste_core::Identifier;

use glam::Mat4;

use super::{emit_object_part, part, pose_of, ObjectPose};
use crate::entity::EntityVertex;

/// Vanilla `BookModel.createBodyLayer` (64x32). `left_lid`, `right_lid`, `seam`, `left_pages`,
/// `right_pages` and the two `flip_page`s; the pose (`BookModel.setupAnim`) is per instance.
pub(super) fn book_model() -> &'static CubeModel {
    static MODEL: OnceLock<CubeModel> = OnceLock::new();
    MODEL.get_or_init(|| CubeModel {
        texture: Identifier::minecraft("entity/enchantment/enchanting_table_book"),
        tex_width: 64,
        tex_height: 32,
        root: ModelPart {
            name: "root".into(),
            pivot: Vec3::ZERO,
            rotation: Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: vec![
                part("left_lid", Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO)
                    .cube(
                        Vec3::new(-6.0, -5.0, -0.005),
                        Vec3::new(6.0, 10.0, 0.005),
                        0.0,
                        [0, 0],
                    )
                    .done(Vec::new()),
                part("right_lid", Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -5.0, -0.005),
                        Vec3::new(6.0, 10.0, 0.005),
                        0.0,
                        [16, 0],
                    )
                    .done(Vec::new()),
                part(
                    "seam",
                    Vec3::ZERO,
                    Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0),
                )
                .cube(
                    Vec3::new(-1.0, -5.0, 0.0),
                    Vec3::new(2.0, 10.0, 0.005),
                    0.0,
                    [12, 0],
                )
                .done(Vec::new()),
                part("left_pages", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, -0.99),
                        Vec3::new(5.0, 8.0, 1.0),
                        0.0,
                        [0, 10],
                    )
                    .done(Vec::new()),
                part("right_pages", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, -0.01),
                        Vec3::new(5.0, 8.0, 1.0),
                        0.0,
                        [12, 10],
                    )
                    .done(Vec::new()),
                part("flip_page1", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, 0.0),
                        Vec3::new(5.0, 8.0, 0.005),
                        0.0,
                        [24, 10],
                    )
                    .done(Vec::new()),
                part("flip_page2", Vec3::ZERO, Vec3::ZERO)
                    .cube(
                        Vec3::new(0.0, -4.0, 0.0),
                        Vec3::new(5.0, 8.0, 0.005),
                        0.0,
                        [24, 10],
                    )
                    .done(Vec::new()),
            ],
        },
    })
}

/// The texture a book is drawn with (vanilla `EnchantTableRenderer.BOOK_TEXTURE`, in the block
/// atlas under the `entity` prefix).
pub fn book_texture() -> Identifier {
    Identifier::minecraft("entity/enchantment/enchanting_table_book")
}

/// Vanilla `BookModel.State.forAnimation`'s openness: `(sin(progress * 0.02) * 0.1 + 1.25) *
/// openness` radians.
pub fn book_openness(progress: f32, openness: f32) -> f32 {
    (((progress * 0.02) as f64).sin() as f32 * 0.1 + 1.25) * openness
}

/// The book's per-instance pose, vanilla `BookModel.setupAnim`: the lids and pages turn by the
/// openness about y (the left lid from `PI`, the right lid from 0), the two flip pages by
/// `openness - openness * 2 * flip`, and the pages slide by `sin(openness)` along x.
pub(super) fn book_pose(openness: f32, flip1: f32, flip2: f32) -> (ObjectPose, Vec3) {
    let pose = pose_of(&[
        (
            "left_lid",
            Vec3::new(0.0, std::f32::consts::PI + openness, 0.0),
        ),
        ("right_lid", Vec3::new(0.0, -openness, 0.0)),
        ("left_pages", Vec3::new(0.0, openness, 0.0)),
        ("right_pages", Vec3::new(0.0, -openness, 0.0)),
        (
            "flip_page1",
            Vec3::new(0.0, openness - openness * 2.0 * flip1, 0.0),
        ),
        (
            "flip_page2",
            Vec3::new(0.0, openness - openness * 2.0 * flip2, 0.0),
        ),
    ]);
    let slide = (openness as f64).sin() as f32;
    (pose, Vec3::new(slide, 0.0, 0.0))
}

/// Draws the posed book (vanilla `Model.renderToBuffer` after `BookModel.setupAnim`): the slide
/// `setupAnim` gives the pages (`x = sin(openness)`) moves only `left_pages`, `right_pages` and
/// the two flip pages; the lids and the seam stay at their pivots.
pub(super) fn emit_book(
    out: &mut Vec<EntityVertex>,
    pose: &ObjectPose,
    slide: Vec3,
    root: Mat4,
    light: (u8, u8),
    rect: [u32; 4],
) {
    let model = book_model();
    for p in &model.root.children {
        let pages = p.name.ends_with("_pages") || p.name.starts_with("flip_page");
        let offset = if pages { slide } else { Vec3::ZERO };
        emit_object_part(out, model, p, pose, root, rect, offset, light);
    }
}
