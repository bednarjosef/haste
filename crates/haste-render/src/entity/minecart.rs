//! The minecart renderer (vanilla `AbstractMinecartRenderer` with `MinecartModel`, plus
//! `TntMinecartRenderer`'s cargo flash): the cart's five boxes over
//! `textures/entity/minecart/minecart.png`, its display block (chest, furnace, hopper, TNT,
//! spawner, command block, or the custom block from the synched data), the track-following
//! rotation and hurt wobble, and the TNT cart's swell and white flash.
//!
//! The old-behavior render path only (see `haste-entity`'s `minecart` module): the cart is
//! posed from the rail positions the game side snapped per frame.

use super::*;

/// One minecart's display-block geometry, resolved once per state id: either a blockstate's
/// model (quads with their variant rotation), or a chest drawn through the block entity
/// chest model (a chest blockstate has no model elements; vanilla's `BlockModelSet` renders
/// it through the chest model the same way).
pub(crate) enum DisplayGeometry {
    /// The chest family: which texture set to draw the single-half chest model with.
    Chest(crate::blockentity::ChestMaterial),
    /// A blockstate's baked quads, each with its model ref's x/y rotation.
    Model(Arc<Vec<(haste_assets::BakedQuad, i32, i32)>>),
}

/// `TntRenderer.getSwellAmount`: `(1 - fuse/10)` clamped to 0..1, to the fourth power, times
/// 0.3 — the TNT cart's cargo swells in the last ten fuse ticks.
pub(crate) fn tnt_swell(fuse: f32) -> f32 {
    let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
    g * g * g * g * 0.3
}

/// `TntRenderer.isLit`: the TNT flash is on while the fuse is counting and its
/// `(fuse / 5)`-tick phase is even.
pub(crate) fn tnt_lit(fuse: f32) -> bool {
    if fuse < 0.0 {
        false
    } else {
        (fuse / 5.0) as i32 % 2 == 0
    }
}

/// `OverlayTexture`'s white row at full progress (`pack(u(1.0F), 10)`): alpha
/// `(1 - 15/15 * 0.75) = 0.25` white over the shaded colour.
pub(crate) const TNT_WHITE_OVERLAY: f32 = 63.0 / 255.0;

impl EntityRenderer {
    /// Poses one minecart, following vanilla `AbstractMinecartRenderer.submit` (the old
    /// render path): the per-entity jitter, the rail-snapped position and rotation, the hurt
    /// wobble, the cargo block, then the cart model at scale (-1, -1, 1).
    pub(super) fn emit_minecart(
        &mut self,
        e: &EntityInstance,
        cart: &haste_core::model::MinecartDecoration,
        camera: glam::DVec3,
        light: (u8, u8),
        assets: &mut Assets,
    ) {
        let base = (e.pos - camera).as_vec3();
        // The per-entity jitter (`state.offsetSeed`, built from the entity id) keeps carts
        // on the same track from z-fighting.
        let seed = offset_seed(e.id.0);
        let jitter = Vec3::new(
            (((seed >> 16 & 7) as f32 + 0.5) / 8.0 - 0.5) * 0.004,
            (((seed >> 20 & 7) as f32 + 0.5) / 8.0 - 0.5) * 0.004,
            (((seed >> 24 & 7) as f32 + 0.5) / 8.0 - 0.5) * 0.004,
        );
        let mut m = Mat4::from_translation(base + jitter);
        // `oldRender`: the cart sits on the rail's line, rotated to face along it; off the
        // rails (or on a zero-length direction) the entity's own yaw/pitch pose it.
        let (mut rotation, mut x_rot) = (e.yaw, e.pitch);
        if let Some((pos, front, back)) = cart.rail {
            let mid = (front.y + back.y) / 2.0;
            m *= Mat4::from_translation(Vec3::new(
                (pos.x - e.pos.x) as f32,
                (mid - e.pos.y) as f32,
                (pos.z - e.pos.z) as f32,
            ));
            let dir = (back - front).as_vec3();
            if dir.length_squared() > 0.0 {
                let dir = dir.normalize();
                rotation = dir.z.atan2(dir.x).to_degrees();
                x_rot = dir.y.atan() * 73.0;
            }
        }
        m *= Mat4::from_translation(Vec3::new(0.0, 0.375, 0.0))
            * Mat4::from_rotation_y((180.0 - rotation).to_radians())
            * Mat4::from_rotation_z(-x_rot.to_radians());
        // The hurt wobble (`hurtTime` counting down from 10, scaled by the accumulated
        // damage, around the cart's long axis).
        if cart.hurt_time > 0.0 {
            let wobble =
                cart.hurt_time.sin() * cart.hurt_time * cart.damage_time / 10.0 * cart.hurt_dir;
            m *= Mat4::from_rotation_x(wobble.to_radians());
        }
        let mut out = std::mem::take(&mut self.vertices);
        // The cargo block, before the cart (both under the same pose): scaled to 0.75,
        // offset by the display offset, turned a quarter turn.
        if let Some(display) = &cart.display {
            let mut cm = m
                * Mat4::from_scale(Vec3::splat(0.75))
                * Mat4::from_translation(Vec3::new(
                    -0.5,
                    (cart.display_offset - 8) as f32 / 16.0,
                    0.5,
                ))
                * Mat4::from_rotation_y(90.0_f32.to_radians());
            // `TntMinecartRenderer.submitMinecartContents`: the TNT cargo swells in the last
            // ten fuse ticks (`TntRenderer.getSwellAmount`).
            let tnt = cart.fuse > -1.0 && cart.fuse < 10.0;
            let (white, swell) = if tnt {
                (tnt_lit(cart.fuse), tnt_swell(cart.fuse))
            } else {
                (false, 0.0)
            };
            if tnt {
                cm *= Mat4::from_translation(Vec3::new(-swell * 0.5, 0.0, -swell * 0.5))
                    * Mat4::from_scale(Vec3::splat(1.0 + swell));
            }
            self.emit_minecart_contents(&mut out, display, cm, light, white, assets);
        }
        // The cart itself: `MinecartModel` under scale (-1, -1, 1) (entity model units).
        let rect = self.texture_rect(&e.model.texture, assets);
        let root = m * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0);
        emit_part_tinted(
            &mut out,
            &e.model,
            &e.model.root,
            &e.pose,
            root,
            rect,
            light,
            0.0,
            [1.0; 3],
        );
        self.vertices = out;
        // `EntityRenderer.extractNameTags` runs for minecarts too: a custom name shows
        // above the cart (`shouldShowName`).
        self.push_minecart_nametag(e, camera);
    }

    /// The minecart's cargo (`submitMinecartContents`): the display block's geometry, cached
    /// per state id, posed under `cm`.
    fn emit_minecart_contents(
        &mut self,
        out: &mut Vec<EntityVertex>,
        display: &haste_core::model::BlockDisplay,
        cm: Mat4,
        light: (u8, u8),
        white: bool,
        assets: &mut Assets,
    ) {
        let geometry: Option<Arc<DisplayGeometry>> = match self.minecart_quads.get(&display.state.0)
        {
            Some(g) => g.clone(),
            None => {
                let g = display_geometry(assets, display).map(Arc::new);
                self.minecart_quads.insert(display.state.0, g.clone());
                g
            }
        };
        let Some(geometry) = geometry else {
            return;
        };
        match &*geometry {
            DisplayGeometry::Chest(material) => {
                // A chest's own model (`ChestModel`, single half, closed) posed like
                // `ChestRenderer` does for the facing=north state the minecart names.
                let model = crate::blockentity::single_chest_model();
                let rect = self.texture_rect(
                    crate::blockentity::chest_texture(
                        *material,
                        crate::blockentity::ChestHalf::Single,
                    ),
                    assets,
                );
                let root = cm
                    * Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                    * Mat4::from_rotation_y(std::f32::consts::PI)
                    * Mat4::from_translation(Vec3::new(-0.5, 0.0, -0.5))
                    * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
                for part in &model.root.children {
                    crate::blockentity::emit_object_part(
                        out,
                        model,
                        part,
                        &[],
                        root,
                        rect,
                        Vec3::ZERO,
                        light,
                    );
                }
            }
            DisplayGeometry::Model(quads) => {
                for (q, rx, ry) in quads.iter() {
                    // The variant's x/y rotation around the block centre (baked into the
                    // matrix like the moved piston's).
                    let mut rot = Mat4::IDENTITY;
                    if *rx != 0 {
                        rot *= Mat4::from_rotation_x((*rx as f32).to_radians());
                    }
                    if *ry != 0 {
                        rot *= Mat4::from_rotation_y((*ry as f32).to_radians());
                    }
                    let rot = Mat4::from_translation(Vec3::splat(0.5))
                        * rot
                        * Mat4::from_translation(-Vec3::splat(0.5));
                    let m = cm * rot;
                    let nm = Mat3::from_mat4(m).inverse().transpose();
                    let rect = self.texture_rect(&parse_tex(&q.texture), assets);
                    let o = q.direction.offset();
                    push_display_quad(
                        out,
                        m,
                        nm,
                        q.positions.map(Vec3::from),
                        q.uvs,
                        Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                        light,
                        rect,
                        white,
                    );
                }
            }
        }
    }

    /// A minecart's custom name tag, drawn like every entity's (`EntityRenderer
    /// .extractNameTags` + `shouldShowName`).
    fn push_minecart_nametag(&mut self, e: &EntityInstance, camera: glam::DVec3) {
        let Some(name) = e.nametag.as_ref().filter(|_| !e.invisible) else {
            return;
        };
        let dist_sq = (e.pos - camera).length_squared();
        if name_tag_visible(dist_sq, e.is_discrete) {
            self.nametags.push(Nametag {
                anchor: e.pos + glam::DVec3::new(0.0, e.name_tag_y as f64 + 0.5, 0.0),
                text: name.clone(),
                see_through: !e.is_discrete,
                below_name: e.below_name.clone(),
            });
        }
    }
}

/// `AbstractMinecartRenderer.submit`'s jitter seed: vanilla
/// `entity.getId() * 493286711L`, then `seed * seed * 4392167121L + seed * 98761L` (Java
/// long arithmetic wraps).
pub(crate) fn offset_seed(entity_id: i32) -> i64 {
    let seed = (entity_id as i64).wrapping_mul(493286711);
    seed.wrapping_mul(seed)
        .wrapping_mul(4392167121)
        .wrapping_add(seed.wrapping_mul(98761))
}

/// One display block's geometry: the chest family through the chest model, everything else
/// through its blockstate's models (`BlockModelResolver.update`); `None` for a block whose
/// model cannot be resolved (nothing is drawn, once, never a panic).
fn display_geometry(
    assets: &mut Assets,
    shown: &haste_core::model::BlockDisplay,
) -> Option<DisplayGeometry> {
    if shown.block.namespace() == "minecraft" {
        let material = match shown.block.path() {
            "chest" => Some(crate::blockentity::ChestMaterial::Normal),
            "trapped_chest" => Some(crate::blockentity::ChestMaterial::Trapped),
            "ender_chest" => Some(crate::blockentity::ChestMaterial::Ender),
            _ => None,
        };
        if let Some(mut material) = material {
            if material == crate::blockentity::ChestMaterial::Normal
                && crate::blockentity::xmas_textures()
            {
                material = crate::blockentity::ChestMaterial::Christmas;
            }
            return Some(DisplayGeometry::Chest(material));
        }
    }
    let def = assets
        .blockstate(&shown.block)
        .map_err(|e| {
            tracing::warn!("minecart display {}: {e:#}", shown.block);
        })
        .ok()?;
    let refs = def.model_refs_for(&shown.props);
    let mut quads = Vec::new();
    for r in refs {
        let id = Identifier::parse(&r.model).ok()?;
        let q = assets
            .model_quads(&id)
            .map_err(|e| tracing::warn!("minecart display model {}: {e:#}", r.model))
            .ok()?;
        for quad in q {
            quads.push((quad, r.x, r.y));
        }
    }
    Some(DisplayGeometry::Model(Arc::new(quads)))
}

/// One block-model quad through `m`, shaded like entities, with vanilla's white TNT overlay
/// mixed in when `white` (see [`TNT_WHITE_OVERLAY`]).
#[allow(clippy::too_many_arguments)]
fn push_display_quad(
    out: &mut Vec<EntityVertex>,
    m: Mat4,
    normal_matrix: Mat3,
    p: [Vec3; 4],
    uv: [[f32; 2]; 4],
    n: Vec3,
    light: (u8, u8),
    rect: [u32; 4],
    white: bool,
) {
    let p = p.map(|v| m.transform_point3(v));
    let nw = (normal_matrix * n).normalize_or_zero();
    let s = entity_shade(nw);
    let color = if white {
        [s + TNT_WHITE_OVERLAY * (1.0 - s); 3]
    } else {
        [s; 3]
    };
    let color = [color[0], color[1], color[2], 1.0];
    let uv = uv.map(|t| {
        [
            rect[0] as f32 + t[0] * rect[2] as f32,
            rect[1] as f32 + t[1] * rect[3] as f32,
        ]
    });
    let ccw = (p[1] - p[0]).cross(p[2] - p[0]).dot(nw) >= 0.0;
    let order: [usize; 6] = if ccw {
        [0, 1, 2, 0, 2, 3]
    } else {
        [0, 2, 1, 0, 3, 2]
    };
    let r16 = rect.map(|v| v as u16);
    for i in order {
        out.push(EntityVertex {
            pos: p[i].to_array(),
            uv: uv[i],
            color,
            rect: r16,
            light: [light.0 as u16, light.1 as u16, 0, 0],
        });
    }
}
