//! The per-entity draw of the stuck-in-body and shoulder layers: `ArrowLayer` and
//! `BeeStingerLayer` (both `StuckInBodyLayer`) and `ParrotOnShoulderLayer`. The counts and the
//! pose inputs come from the [`StuckInput`] the client hands over each frame (`set_stuck`),
//! with the stinger and parrot models it resolved from the model library.

use std::sync::{Arc, OnceLock};

use haste_core::model::CubeModel;
use haste_core::EntityId;

use super::arrow_layer::push_stuck_arrows;
use super::bee_stinger_layer::push_stuck_stingers;
use super::model_parts::emit_part;
use super::parrot_on_shoulder::{shoulder_offset, shoulder_pose, ShoulderPose};
use super::stuck_in_body::PLAYER_BODY_PARTS;
use super::*;

/// The parrot variants `ParrotRenderer.getVariantTexture` names (0..=4).
const PARROT_VARIANTS: usize = 5;

/// What one living entity shows on its body this frame, with the render-state inputs the
/// stuck items and shoulder parrots are posed from. The client fills it from the entity store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuckInput {
    pub id: EntityId,
    pub arrows: u32,
    pub stingers: u32,
    /// The parrot variant (0..=4) on the left and right shoulder, `None` when empty.
    pub parrots: [Option<u32>; 2],
    pub crouching: bool,
    /// The head's yaw relative to the body and its pitch, in radians.
    pub head_yaw: f32,
    pub head_pitch: f32,
    /// The walk cycle's position and speed.
    pub walk_pos: f32,
    pub walk_speed: f32,
}

/// The models the stuck layers draw besides the body's own (`BeeStingerModel`, `ParrotModel`),
/// at their pixel scale. Set with [`EntityRenderer::set_stuck`].
#[derive(Clone, Debug, Default)]
pub struct StuckModels {
    pub stinger: Option<Arc<CubeModel>>,
    pub parrot: Option<Arc<CubeModel>>,
}

/// The textures the stuck layers draw with, named once.
struct StuckTextures {
    arrow: Identifier,
    stinger: Identifier,
    parrots: [Identifier; PARROT_VARIANTS],
}

fn stuck_textures() -> &'static StuckTextures {
    static TEXTURES: OnceLock<StuckTextures> = OnceLock::new();
    TEXTURES.get_or_init(|| StuckTextures {
        // `ArrowLayer` draws the flight arrow's texture.
        arrow: Identifier::minecraft("entity/projectiles/arrow"),
        // `BeeStingerLayer.BEE_STINGER_LOCATION`.
        stinger: Identifier::minecraft("entity/bee/bee_stinger"),
        parrots: [
            "entity/parrot/parrot_red_blue",
            "entity/parrot/parrot_blue",
            "entity/parrot/parrot_green",
            "entity/parrot/parrot_yellow_blue",
            "entity/parrot/parrot_grey",
        ]
        .map(Identifier::minecraft),
    })
}

impl EntityRenderer {
    /// This frame's stuck state, sorted by entity id, and the layer models. Reuses the buffer,
    /// so nothing allocates once it has grown.
    pub fn set_stuck(&mut self, states: &[StuckInput], models: StuckModels) {
        self.stuck.clear();
        self.stuck.extend_from_slice(states);
        self.stuck_models = models;
    }

    /// The arrows and stingers stuck in the body and the shoulder parrots of `e`, posed through
    /// `root` (the body transform with the model scale). Vanilla order: `ArrowLayer`,
    /// `BeeStingerLayer`, then `ParrotOnShoulderLayer`. The stuck items are seeded by the
    /// entity id, like vanilla's `RandomSource.createThreadLocalInstance(state.id)`.
    pub(super) fn emit_stuck(
        &mut self,
        e: &EntityInstance,
        root: Mat4,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let Ok(i) = self.stuck.binary_search_by_key(&e.id.0, |s| s.id.0) else {
            return;
        };
        let s = self.stuck[i];
        let seed = i64::from(e.id.0);
        let textures = stuck_textures();
        if s.arrows > 0 {
            let rect = self.texture_rect(&textures.arrow, assets);
            let mut out = std::mem::take(&mut self.vertices);
            push_stuck_arrows(
                &mut out,
                root,
                &e.model,
                &e.pose,
                &PLAYER_BODY_PARTS,
                s.arrows,
                seed,
                light,
                rect,
            );
            self.vertices = out;
        }
        if s.stingers > 0 {
            if let Some(stinger) = self.stuck_models.stinger.clone() {
                let rect = self.texture_rect(&textures.stinger, assets);
                let mut out = std::mem::take(&mut self.vertices);
                push_stuck_stingers(
                    &mut out,
                    root,
                    &e.model,
                    &e.pose,
                    &PLAYER_BODY_PARTS,
                    s.stingers,
                    seed,
                    &stinger,
                    light,
                    rect,
                );
                self.vertices = out;
            }
        }
        for (side, parrot) in s.parrots.iter().enumerate() {
            let Some(variant) = *parrot else {
                continue;
            };
            let Some(model) = self.stuck_models.parrot.clone() else {
                break;
            };
            let id = &textures.parrots[(variant as usize).min(PARROT_VARIANTS - 1)];
            let rect = self.texture_rect(id, assets);
            let pose = shoulder_pose(
                &model,
                ShoulderPose {
                    head_pitch: s.head_pitch,
                    head_yaw: s.head_yaw,
                    walk_pos: s.walk_pos,
                    walk_speed: s.walk_speed,
                },
            );
            // Index 0 is the left shoulder (`Player.DATA_SHOULDER_PARROT_LEFT`).
            let parent = root * Mat4::from_translation(shoulder_offset(side == 0, s.crouching));
            let mut out = std::mem::take(&mut self.vertices);
            emit_part(
                &mut out,
                &model,
                &model.root,
                &pose,
                parent,
                rect,
                light,
                0.0,
            );
            self.vertices = out;
        }
    }
}
