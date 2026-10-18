//! The moving piston renderer (`PistonHeadRenderer`): the moved block model at the
//! interpolated progress offset plus the retracting source piston base.

use super::*;

impl EntityRenderer {
    /// One moving piston, following vanilla `PistonHeadRenderer`: the moved block's model
    /// (from the block entity's NBT state string, `short`/`extended` adjusted like
    /// `extractRenderState` does) at the interpolated progress offset, plus the retracting
    /// source piston's base. The quads are resolved once per state string and cached; an
    /// unreadable model draws nothing, never a panic.
    pub(super) fn emit_piston(
        &mut self,
        inst: &crate::blockentity::BlockEntityInstance,
        camera: glam::DVec3,
        assets: &mut Assets,
    ) {
        use crate::blockentity::{
            piston_drawn_states, piston_extended_progress, piston_quads, PistonQuad,
        };
        let Some(p) = inst.piston.clone() else {
            return;
        };
        // The progress pair already folds the partial tick (the client's anim ends).
        let progress = p.progress.end;
        let (block_state, head_state) = piston_drawn_states(&p, progress);
        let off = piston_extended_progress(&p, progress);
        // Vanilla `getMovementDirection`: the facing while extending, its opposite
        // retracting.
        let movement = if p.extending {
            p.facing
        } else {
            p.facing.opposite()
        };
        let step = p.facing.offset();
        let offset = Vec3::new(
            step.x as f32 * off,
            step.y as f32 * off,
            step.z as f32 * off,
        );
        let (sky, block) = inst.light;
        let light = (block, sky);
        let base = (inst.pos.as_dvec3() - camera).as_vec3();
        let mut out = std::mem::take(&mut self.vertices);
        let mut emit_state = |me: &mut Self, out: &mut Vec<EntityVertex>, state: &str, at: Vec3| {
            let quads: Option<Arc<Vec<PistonQuad>>> = match me.piston_quads.get(state) {
                Some(q) => q.clone(),
                None => {
                    let q = piston_quads(assets, state);
                    me.piston_quads.insert(state.to_string(), q.clone());
                    q
                }
            };
            let Some(quads) = quads else {
                return;
            };
            let m = Mat4::from_translation(base + at);
            let nm = Mat3::from_mat4(m).inverse().transpose();
            for (q, rx, ry) in quads.iter() {
                // The variant's x/y rotation around the block centre, baked into the matrix.
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
                let m = m * rot;
                let rect = me.texture_rect(&parse_tex(&q.texture), assets);
                let o = q.direction.offset();
                push_world_quad(
                    out,
                    m,
                    nm,
                    q.positions.map(Vec3::from),
                    q.uvs,
                    Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                    [1.0; 3],
                    light,
                    rect,
                );
            }
        };
        // Vanilla: the moved block is posed at `pos.relative(movement.getOpposite())`, then
        // translated by the progress offset.
        let back = Vec3::new(
            -movement.offset().x as f32,
            -movement.offset().y as f32,
            -movement.offset().z as f32,
        );
        emit_state(self, &mut out, &block_state, back + offset);
        if let Some(head) = head_state {
            // The base sits at the block entity's own position: vanilla's
            // `basePos = pos.relative(movement)` walks back to where `pos` came from
            // (`pos = bePos.relative(movement.getOpposite())`), so `basePos == bePos`,
            // unoffset.
            emit_state(self, &mut out, &head, Vec3::ZERO);
        }
        self.vertices = out;
    }
}
