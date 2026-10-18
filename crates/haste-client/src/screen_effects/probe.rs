//! The per-tick world probe the screen effects read: the block at the eye, the portal at
//! the feet, fire and powder snow around the box, the light at the eye.
use super::fire::is_fire_block;
use super::in_block::{is_view_blocking, particle_texture, view_probe_points};
use super::portal::is_portal_block;
use super::powder_snow::is_powder_snow_block;
use haste_protocol::haste_core::Identifier;
/// Where the player is standing, once per tick: the blocks the camera sees through and the
/// portal under our feet.
#[derive(Clone, Debug, Default)]
pub struct WorldProbe {
    /// The texture over the whole screen when the camera is inside a view-blocking block
    /// (the block's particle texture, `LevelExtractor.getViewBlockingState`).
    pub block_overlay: Option<Identifier>,
    /// Whether the block at our feet is a nether portal (`NetherPortalBlock.entityInside`).
    pub in_portal: bool,
    /// Whether our bounding box overlaps a fire block (`BaseFireBlock.entityInside` applies
    /// `FIRE_IGNITE`, which is what sets the local player's fire ticks on the client).
    pub in_fire: bool,
    /// Whether our bounding box overlaps powder snow (`PowderSnowBlock.entityInside` applies
    /// `FREEZE`, which is what sets the local player's freeze ticks on the client).
    pub in_powder_snow: bool,
    /// The brighter of the sky and block light at the eye position
    /// (`Level.getMaxLocalRawBrightness`, overworld: no sky darkening).
    pub eye_light: u8,
}

/// The world the screen effects' per-tick probe reads.
pub struct Probe<'a> {
    pub world: &'a dyn haste_core::view::BlockView,
    pub shapes: &'a dyn haste_core::view::CollisionShapes,
    pub blocks: &'a haste_data::BlockStates,
    /// The block tags the server synced (`update_tags`), for `causes_suffocation`.
    pub tags: Option<&'a haste_physics::mining::BlockTags>,
    /// The interpolated eye position and the feet, for the eye's block and the portal check.
    pub eye: glam::DVec3,
    pub feet: glam::DVec3,
    /// The player's box width, height and scale: the width and scale for the view-blocking
    /// sample points, all three for the fire block overlap.
    pub bb_width: f64,
    pub bb_height: f64,
    pub scale: f64,
    /// The local player is a spectator: vanilla's `Entity.noPhysics` (set from
    /// `Player.tick`'s `isSpectator()`). It closes `LevelExtractor.getViewBlockingState`
    /// (no in-block overlay) and `Entity.isAffectedByBlocks` (no block-inside effects:
    /// portal, fire ignite, powder-snow freeze).
    pub spectator: bool,
}

impl Probe<'_> {
    /// Vanilla's `LevelExtractor.getViewBlockingState` (the block the camera is inside),
    /// `Level.getMaxLocalRawBrightness` at the eye (the water overlay's light) and
    /// `NetherPortalBlock.entityInside` (standing in a portal), in one world read.
    pub fn sample(&self) -> WorldProbe {
        let air = haste_core::BlockStateId::AIR;
        let mut block_overlay = None;
        if !self.spectator {
            for p in view_probe_points(self.eye, self.bb_width, self.scale) {
                let pos = haste_core::BlockPos::new(
                    p.x.floor() as i32,
                    p.y.floor() as i32,
                    p.z.floor() as i32,
                );
                let state = self.world.block_state(pos);
                if state == air {
                    continue;
                }
                if is_view_blocking(state, self.blocks, self.shapes, self.tags) {
                    block_overlay = Some(particle_texture(self.blocks.block_of(state)));
                    break;
                }
            }
        }
        // The block at the feet is a nether portal (`Entity.setAsInsidePortal` fires from the
        // portal block's `entityInside`).
        let in_portal = if self.spectator {
            false
        } else {
            let feet_pos = haste_core::BlockPos::new(
                self.feet.x.floor() as i32,
                self.feet.y.floor() as i32,
                self.feet.z.floor() as i32,
            );
            let feet_state = self.world.block_state(feet_pos);
            is_portal_block(self.blocks.block_of(feet_state))
        };
        // Whether any fire block overlaps our bounding box: the local player's client-side
        // `BaseFireBlock.entityInside` -> `FIRE_IGNITE` is what sets its own fire ticks (the
        // server does not send the owner their own `set_entity_data`). A spectator never
        // gets block-inside effects at all (`Entity.isAffectedByBlocks`: `!noPhysics`).
        let mut in_fire = false;
        let mut in_powder_snow = false;
        if !self.spectator {
            let half = self.bb_width * 0.5;
            let min = self.feet - glam::DVec3::new(half, 0.0, half);
            let max = self.feet + glam::DVec3::new(half, self.bb_height, half);
            let (x0, x1) = (min.x.floor() as i32, (max.x - 1e-7).floor() as i32);
            let (y0, y1) = (min.y.floor() as i32, (max.y - 1e-7).floor() as i32);
            let (z0, z1) = (min.z.floor() as i32, (max.z - 1e-7).floor() as i32);
            for x in x0..=x1 {
                for y in y0..=y1 {
                    for z in z0..=z1 {
                        let pos = haste_core::BlockPos::new(x, y, z);
                        let block = self.blocks.block_of(self.world.block_state(pos));
                        in_fire |= is_fire_block(block, self.blocks, self.tags);
                        in_powder_snow |= is_powder_snow_block(block, self.blocks, self.tags);
                    }
                }
            }
        }
        // The eye position's brightest light channel (`getMaxLocalRawBrightness`: the
        // overworld's sky light is not darkened by the time of day here, so the raw values).
        let eye_pos = haste_core::BlockPos::new(
            self.eye.x.floor() as i32,
            self.eye.y.floor() as i32,
            self.eye.z.floor() as i32,
        );
        let eye_light = self
            .world
            .sky_light(eye_pos)
            .max(self.world.block_light(eye_pos));
        WorldProbe {
            block_overlay,
            in_portal,
            in_fire,
            in_powder_snow,
            eye_light,
        }
    }
}
