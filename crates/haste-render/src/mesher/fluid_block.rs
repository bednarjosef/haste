//! The fluid in one block cell of a section (vanilla `SectionCompiler.compile` handing a
//! block's fluid state to `FluidRenderer.tesselate`): the quads from `crate::fluid`, lit and
//! tinted, into the section's output.

use super::fluid_view::FluidView;
use super::{
    block_tint, pack_ao_shade, pack_light, shade_index, MeshContext, MeshSink, TINT_WHITE,
};

/// Max of two packed (sky, block) light values per channel.
fn max_light(a: (u8, u8), b: (u8, u8)) -> u8 {
    pack_light(a.0.max(b.0), a.1.max(b.1))
}

/// Emits the fluid surface of one block (see `fluid.rs` for the rules). Quads
/// go through the context's reusable scratch Vec (no per-block allocation).
pub(super) fn mesh_fluid(
    ctx: &mut MeshContext,
    lx: i32,
    ly: i32,
    lz: i32,
    fluid: crate::fluid::FluidState,
    sink: &mut MeshSink,
) {
    let block_pos = ctx.origin.add(lx, ly, lz);
    let mut quads = std::mem::take(&mut *ctx.fluid_scratch);
    quads.clear();
    {
        let view = FluidView {
            snapshot: ctx.snapshot,
            baked: ctx.baked,
            occluders: &ctx.tables.fluid_occluders,
        };
        crate::fluid::fluid_quads(&view, &ctx.tables.fluids, block_pos, fluid, &mut quads);
    }
    if !quads.is_empty() {
        let tint = match ctx.tables.fluids.defs.def(fluid.fluid).tint {
            crate::fluid::FluidTint::BiomeWater => block_tint(
                ctx.snapshot,
                ctx.tables,
                crate::tint::TintKind::Water,
                block_pos,
            ),
            crate::fluid::FluidTint::None => TINT_WHITE,
        };
        let layer = crate::fluid::fluid_layer(&ctx.tables.fluids.defs, fluid.fluid);
        let own = ctx.snapshot.light(block_pos);
        let top_light = max_light(own, ctx.snapshot.light(block_pos.add(0, 1, 0)));
        let bottom_light = max_light(own, ctx.snapshot.light(block_pos.add(0, -1, 0)));
        // Fluid light is a whole level (no blend): *16 into the smooth units
        // the chunk vertex carries.
        let light16 = |l: u8| {
            [
                (l >> 4) as u32 * crate::vertex::LIGHT16_PER_LEVEL as u32,
                (l & 0xF) as u32 * crate::vertex::LIGHT16_PER_LEVEL as u32,
            ]
        };
        let top16 = light16(top_light);
        let bottom16 = light16(bottom_light);
        let offset = [lx as f32, ly as f32, lz as f32];
        for q in &quads {
            let [sky16, block16] = if q.dir == haste_assets::Direction::Down {
                bottom16
            } else {
                top16
            };
            let packed = [
                sky16 as u8,
                pack_ao_shade(0, shade_index(Some(q.dir))),
                block16 as u8,
            ];
            let positions = q
                .positions
                .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]);
            sink.push_quad(layer, &positions, &q.uvs, [packed; 4], tint, q.sprite);
        }
    }
    *ctx.fluid_scratch = quads;
}
