//! The fluid in one block cell of a section (vanilla `SectionCompiler.compile` handing a
//! block's fluid state to `FluidRenderer.tesselate`): the quads from `crate::fluid`, lit and
//! tinted, into the section's output.

use super::fluid_view::FluidView;
use super::merge::{greedy_merge_enabled, merge_face_geom, MergeKey, MergeQuad, StagedQuad};
use super::sink::{axis_facing, layer_index};
use super::{block_tint, pack_ao_shade, pack_light, shade_index, MeshContext, TINT_WHITE};

/// Max of two packed (sky, block) light values per channel.
fn max_light(a: (u8, u8), b: (u8, u8)) -> u8 {
    pack_light(a.0.max(b.0), a.1.max(b.1))
}

/// Emits the fluid surface of one block (see `fluid.rs` for the rules). Quads
/// go through the context's reusable scratch Vec (no per-block allocation), then into
/// the section's staged quads like the block models' faces: a still, flat surface (an
/// ocean or a lake) has uniform light and tint and the still sprite's full 0..1 UVs, so it
/// greedy-merges into a few large quads where vanilla draws one per block. The pixels stay
/// the same (the shader tiles the sprite across a merged quad), and so does the order:
/// coplanar fluid faces of different blocks never overlap, and a surface's backward copy
/// faces the other way.
pub(super) fn mesh_fluid(
    ctx: &mut MeshContext,
    lx: i32,
    ly: i32,
    lz: i32,
    fluid: crate::fluid::FluidState,
    staged: &mut Vec<StagedQuad>,
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
            // The merge key's direction is the facing group the merged fan files under, so
            // it must be the geometric facing: the backward up face is an up face wound
            // to face down.
            let merge = greedy_merge_enabled()
                .then(|| merge_face_geom(q.dir, &q.positions, &q.uvs))
                .flatten()
                .zip(axis_facing(&positions));
            match merge {
                Some((face, facing)) => staged.push(StagedQuad::Merge(MergeQuad {
                    key: MergeKey {
                        layer: layer_index(layer) as u8,
                        dir: facing.index() as u8,
                        sprite: q.sprite,
                        tint,
                        light: packed,
                        face,
                    },
                    cell: [lx as u8, ly as u8, lz as u8],
                })),
                None => staged.push(StagedQuad::Direct {
                    layer,
                    positions,
                    uvs: q.uvs,
                    lights: [packed; 4],
                    tint,
                    sprite: q.sprite,
                }),
            }
        }
    }
    *ctx.fluid_scratch = quads;
}
