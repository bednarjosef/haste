//! Entity models as data (`haste_core::model::CubeModel`).
//!
//! Model-space convention for everything this crate emits (renderer renders it):
//! - units are 1/16 block (vanilla model units);
//! - y increases downward (vanilla `PartPose` convention);
//! - the feet sit on the plane y = 0, so a player's head top is at y = -32;
//! - a child part's pivot is relative to its parent's (vanilla `PartPose`), so overlay layers
//!   (hat, jacket, sleeves, pants) sit at [0, 0, 0] under their base part;
//! - parts rotate around their pivot in x, y, z order; `PoseOverrides` rotations are ADDITIVE to
//!   the part's base rotation. The renderer rotates the part named "head" by
//!   (head_yaw - yaw) around Y and pitch around X; the rest of the body follows `yaw`.
//!
//! The humanoid is the standard 64x64 skin layout (public knowledge; the UV offsets are in the
//! entities brief). The player model is authored 32 units tall and rendered at scale 0.9375 (vanilla's
//! player-model scale), i.e. 1.875 blocks against the 1.8 hitbox. Every other entity type gets a
//! generic box built from its dimensions, textured with the `haste:entity_box` placeholder.
use std::sync::{Arc, Mutex, OnceLock};

use ahash::AHashMap;
use haste_core::model::{Cube, CubeModel, ModelPart};
use haste_core::Identifier;

fn part(name: &str, pivot: [f32; 3], cubes: Vec<Cube>, children: Vec<ModelPart>) -> ModelPart {
    ModelPart {
        name: name.to_owned(),
        id: haste_core::parts::intern(name),
        pivot: pivot.into(),
        rotation: glam::Vec3::ZERO,
        visible: true,
        cubes,
        children,
    }
}

fn cube(origin: [f32; 3], size: [f32; 3], uv: [u32; 2], inflate: f32) -> Cube {
    Cube {
        origin: origin.into(),
        size: size.into(),
        inflate,
        uv,
        mirror: false,
    }
}

/// The humanoid/player model. `slim` selects the 3-pixel-wide arm layout (Alex-style skins);
/// otherwise the classic 4-pixel arms (Steve layout).
pub fn humanoid(slim: bool) -> Arc<CubeModel> {
    static MODELS: OnceLock<(Arc<CubeModel>, Arc<CubeModel>)> = OnceLock::new();
    let (classic, slim_model) = MODELS.get_or_init(|| {
        let build = move |slim: bool| {
            // Arm width and right-arm x origin: classic arms are 4 wide at -3, slim are 3 at -2
            // (the left arm mirrors around x = 0, so its origin is symmetric).
            let (aw, rao) = if slim { (3.0, -2.0) } else { (4.0, -3.0) };
            let uv_arm_r: [u32; 2] = [40, 16];
            let uv_arm_l: [u32; 2] = [32, 48];
            let uv_sleeve_r: [u32; 2] = [40, 32];
            let uv_sleeve_l: [u32; 2] = [48, 48];
            let root = part(
                "root",
                [0.0, 0.0, 0.0],
                Vec::new(),
                vec![
                    part(
                        "head",
                        [0.0, -24.0, 0.0],
                        vec![cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [0, 0], 0.0)],
                        vec![part(
                            "hat",
                            [0.0, 0.0, 0.0],
                            vec![cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [32, 0], 0.5)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "body",
                        [0.0, -24.0, 0.0],
                        vec![cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16, 16], 0.0)],
                        vec![part(
                            "jacket",
                            [0.0, 0.0, 0.0],
                            vec![cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16, 32], 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "right_arm",
                        [-5.0, -22.0, 0.0],
                        vec![cube([rao, -2.0, -2.0], [aw, 12.0, 4.0], uv_arm_r, 0.0)],
                        vec![part(
                            "right_sleeve",
                            [0.0, 0.0, 0.0],
                            vec![cube([rao, -2.0, -2.0], [aw, 12.0, 4.0], uv_sleeve_r, 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "left_arm",
                        [5.0, -22.0, 0.0],
                        vec![cube([-1.0, -2.0, -2.0], [aw, 12.0, 4.0], uv_arm_l, 0.0)],
                        vec![part(
                            "left_sleeve",
                            [0.0, 0.0, 0.0],
                            vec![cube([-1.0, -2.0, -2.0], [aw, 12.0, 4.0], uv_sleeve_l, 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "right_leg",
                        [-1.9, -12.0, 0.0],
                        vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0, 16], 0.0)],
                        vec![part(
                            "right_pants",
                            [0.0, 0.0, 0.0],
                            vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0, 32], 0.25)],
                            Vec::new(),
                        )],
                    ),
                    part(
                        "left_leg",
                        [1.9, -12.0, 0.0],
                        vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [16, 48], 0.0)],
                        vec![part(
                            "left_pants",
                            [0.0, 0.0, 0.0],
                            vec![cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0, 48], 0.25)],
                            Vec::new(),
                        )],
                    ),
                ],
            );
            Arc::new(CubeModel {
                texture: Identifier::new("haste", "entity/humanoid"),
                tex_width: 64,
                tex_height: 64,
                root,
            })
        };
        let _ = build;
        (build(false), build(true))
    });
    if slim {
        slim_model.clone()
    } else {
        classic.clone()
    }
}

/// A generic box model from entity dimensions (blocks), cached per quantized size (1/16 units).
/// Cold path: called on spawn and dimension (pose) changes, never per frame.
type BoxCache = Mutex<AHashMap<(u16, u16), Arc<CubeModel>>>;
pub fn box_for(width: f32, height: f32) -> Arc<CubeModel> {
    static CACHE: OnceLock<BoxCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(AHashMap::new()));
    let w = ((width * 16.0).round() as i16).max(1) as u16;
    let h = ((height * 16.0).round() as i16).max(1) as u16;
    let mut guard = cache.lock().unwrap();
    guard
        .entry((w, h))
        .or_insert_with(|| {
            let hw = f32::from(w) * 0.5;
            let (wf, hf) = (f32::from(w), f32::from(h));
            Arc::new(CubeModel {
                texture: Identifier::new("haste", "entity_box"),
                tex_width: 64,
                tex_height: 64,
                root: part(
                    "root",
                    [0.0, 0.0, 0.0],
                    vec![cube([-hw, -hf, -hw], [wf, hf, wf], [0, 0], 0.0)],
                    Vec::new(),
                ),
            })
        })
        .clone()
}

/// Whether a model has a part with this id (humanoid models carry `right_arm`, which only the
/// armed/armoured layers need).
pub(crate) fn model_has_part(
    part: &haste_core::model::ModelPart,
    id: haste_core::parts::PartId,
) -> bool {
    part.id == id || part.children.iter().any(|c| model_has_part(c, id))
}

/// The model an entity draws with, whether it has a head part (the head aim), and its overlay
/// layers (vanilla's per-type renderer choice): players the humanoid, boats their dumped layer,
/// minecarts the minecart model, variant and table-covered mobs their registry row, everything
/// else a generic box from its dimensions.
///
/// Players get the humanoid model (classic layout; skins arrive with renderer's texture work),
/// known mob types the model the recorder dumped for their vanilla layer, everything else a
/// generic box. Babies use the table row's baby layer, like vanilla's `AgeableMobRenderer`.
pub(crate) fn model_for_entity(
    e: &crate::store::Entity,
    minecart: bool,
    variants: &crate::variants::MobVariants,
    world_age: i64,
) -> (
    std::sync::Arc<CubeModel>,
    bool,
    [Option<haste_core::model::EntityOverlay>; 2],
) {
    if e.is_player_type {
        return (humanoid(false), true, Default::default());
    }
    // Registry table first (shared per type, head flag precomputed), then the box. Babies use
    // the row's baby layer, like vanilla's AgeableMobRenderer.submit. Variant mobs
    // (cat/wolf/frog/pig/cow/chicken) instead resolve their look from the synced variant
    // registry through the entity's metadata, falling back to the table's default row when
    // the variant is unknown or absent.
    let baby = crate::registry::is_baby(&e.metadata);
    let variant = crate::variants::model_for(e.kind, variants, &e.metadata, baby).or_else(|| {
        // Metadata-driven variants (axolotl/rabbit/horse/llama/panda/parrot/fox/bee/strider/
        // mooshroom): the same layer, the variant's texture.
        crate::variants::metadata_model_for(
            e.kind,
            &e.metadata,
            baby,
            e.metadata.custom_name_plain(),
            world_age,
        )
    });
    // The tropical fish's shape follows its packed pattern; the shulker's base texture follows
    // its colour byte. Both are plain model swaps, and both must be gated on the entity's own
    // type: their metadata slots (17, 18) collide with other mobs' fields (e.g. a sheep's
    // wool byte sits where a shulker's colour byte would).
    let type_name = crate::registry::entity_type_name(e.kind);
    let variant = variant.or_else(|| {
        if type_name == Some("minecraft:tropical_fish") {
            crate::variants::tropical_fish_model(&e.metadata)
        } else {
            None
        }
    });
    let variant = match type_name {
        Some("minecraft:shulker") => {
            match crate::variants::shulker_texture(&e.metadata) {
                // The colour texture swap is cached per texture (the model is cloned once
                // per colour, never per frame).
                Some(texture) => crate::variants::shulker_model_for(&texture),
                None => variant,
            }
        }
        _ => variant,
    };
    // Boats (`AbstractBoatRenderer`): the dumped boat layer with the trailing yaw baked
    // into its root, the water patch overlay when it floats (a raft has none), no head part
    // (the whole hull follows the entity yaw). The spawn-time family flag keeps the table
    // lookup off every other entity's path; a boat type the table does not name (a mod's)
    // falls through to the generic path.
    let boat = e
        .is_boat
        .then(|| type_name.and_then(crate::boat::entry_for_name))
        .flatten();
    if let Some(boat) = boat {
        let model =
            crate::boat::model(boat).unwrap_or_else(|| box_for(e.dimensions.0, e.dimensions.1));
        let overlays = match crate::boat::water_patch_overlay(boat, &e.boat) {
            Some(patch) => [Some(patch), None],
            None => [None, None],
        };
        return (model, false, overlays);
    }
    let entry = crate::registry::model_entry(e.kind);
    let overlays = crate::variants::overlays_for(e.kind, &e.metadata, baby);
    match (variant, entry) {
        // Minecarts draw through `AbstractMinecartRenderer` with the one `MinecartModel`
        // layer every minecart type bakes (`MinecartModel.createBodyLayer`); cargo and
        // track pose come from the decoration.
        _ if minecart => (crate::minecart::model(), false, Default::default()),
        // A variant model (already the baby's when `baby`); all six variant mobs have a
        // head part, so the head aim stays on.
        (Some(v), _) => (v, true, overlays),
        (None, Some((head, m))) => {
            let baby_model = type_name
                .filter(|_| baby)
                .and_then(crate::registry::baby_model_for_name);
            (baby_model.unwrap_or(m), head, overlays)
        }
        (None, None) => (box_for(e.dimensions.0, e.dimensions.1), false, overlays),
    }
}
