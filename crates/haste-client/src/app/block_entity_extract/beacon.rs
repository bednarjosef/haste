use std::sync::Arc;

use haste_protocol::haste_core::BlockPos;
use haste_world::World;

use super::BlockEntityExtractor;

impl BlockEntityExtractor {
    /// The beacon's beam sections at `pos`: vanilla's own client-side walk
    /// (`BeaconBlockEntity.tick` over the stained glass above), done in one pass and re-walked
    /// whenever the column above the beacon changes (the key is a hash of the column's states;
    /// vanilla re-checks 10 blocks a tick, we need only the result).
    pub(super) fn beacon_data(
        &mut self,
        pos: BlockPos,
        world: &World,
        blocks: &haste_data::BlockStates,
        data: &haste_data::GameData,
    ) -> Option<Arc<haste_render::BeamData>> {
        use haste_core::view::{BlockPhysics as _, BlockView as _};
        // The column's states from the beacon block itself (vanilla's walk starts at it: the
        // beacon is a white `BeaconBeamBlock`, so it seeds the first section) up to the
        // heightmap top, hashed as the cache key.
        let top = world
            .chunk(pos.chunk())
            .and_then(|c| c.height_at((pos.x & 15) as u8, (pos.z & 15) as u8))
            .unwrap_or(pos.y);
        let mut key = 0u64;
        let states: Vec<_> = (pos.y..=top)
            .map(|y| {
                let s = world.block_state(BlockPos::new(pos.x, y, pos.z));
                key = key.wrapping_mul(31).wrapping_add(s.0 as u64);
                s
            })
            .collect();
        if let Some((k, data)) = self.beacon_beams.get(&pos) {
            if *k == key {
                return Some(data.clone());
            }
        }
        // Vanilla's walk: the first one or two stained-glass blocks start sections (a colour
        // change beyond that starts one in the averaged colour), any other block with light
        // dampening 15 (except bedrock) ends the beam, and every further block extends the
        // last section.
        let mut sections: Vec<haste_render::BeamSection> = Vec::new();
        for &s in &states {
            let block = blocks.block_of(s);
            if let Some(color) = beacon_beam_color(block) {
                match sections.len() {
                    0 | 1 => sections.push(haste_render::BeamSection { color, height: 1 }),
                    _ => {
                        let last_color = sections.last().unwrap().color;
                        if last_color == color {
                            sections.last_mut().unwrap().height += 1;
                        } else {
                            sections.push(haste_render::BeamSection {
                                color: average_argb(last_color, color),
                                height: 1,
                            });
                        }
                    }
                }
            } else {
                let ends = match sections.last() {
                    None => true,
                    Some(_) => {
                        // Vanilla: `state.getLightDampening() >= 15 && !state.is(bedrock)` —
                        // the oracle's opaque flag is the same fully-light-blocking set.
                        data.is_opaque_full(s)
                            && block != &haste_core::Identifier::minecraft("bedrock")
                    }
                };
                if ends {
                    sections.clear();
                    break;
                }
                sections.last_mut().unwrap().height += 1;
            }
        }
        let data = Arc::new(haste_render::BeamData { sections });
        self.beacon_beams.insert(pos, (key, data.clone()));
        Some(data)
    }
}

/// A stained-glass block's (or the beacon's own) beam colour (vanilla `BeaconBeamBlock.getColor`
/// -> `DyeColor.getTextureDiffuseColor`), or `None` for other blocks.
pub(super) fn beacon_beam_color(block: &haste_core::Identifier) -> Option<u32> {
    const GLASS: [(&str, u32); 16] = [
        ("white_stained_glass", 0),
        ("orange_stained_glass", 1),
        ("magenta_stained_glass", 2),
        ("light_blue_stained_glass", 3),
        ("yellow_stained_glass", 4),
        ("lime_stained_glass", 5),
        ("pink_stained_glass", 6),
        ("gray_stained_glass", 7),
        ("light_gray_stained_glass", 8),
        ("cyan_stained_glass", 9),
        ("purple_stained_glass", 10),
        ("blue_stained_glass", 11),
        ("brown_stained_glass", 12),
        ("green_stained_glass", 13),
        ("red_stained_glass", 14),
        ("black_stained_glass", 15),
    ];
    if block.namespace() != "minecraft" {
        return None;
    }
    // The beacon block itself is a white beam block (vanilla `BeaconBlock.getColor`).
    let color = if block.path() == "beacon" {
        0
    } else {
        GLASS
            .iter()
            .find(|(name, _)| *name == block.path())
            .map(|(_, dye)| *dye)?
    };
    Some(haste_render::blockentity::DYE_DIFFUSE_COLOR[color as usize])
}

/// Vanilla `ARGB.average`: the per-channel mean of two colours.
pub(super) fn average_argb(a: u32, b: u32) -> u32 {
    let ch = |s: u32| (((a >> s) & 0xFF).wrapping_add((b >> s) & 0xFF)) / 2;
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}
