//! Arrows (`minecraft:arrow`, `minecraft:spectral_arrow`): vanilla `AbstractArrow`'s client
//! tick and `ArrowRenderer.extractRenderState`.
//!
//! Like dropped items, arrows are `MoveSimulationType.SERVER_AND_CLIENT`: the client keeps
//! simulating the flight (gravity, drag, the block clip) between the server's position
//! corrections, and it derives the render rotation from the movement itself
//! (`AbstractArrow.tick`). The stuck-in-ground shake (`shakeTime`) is set to 7 when the
//! in-ground flag flips to true (`AbstractArrow.onSyncedDataUpdated`) — by a metadata packet
//! or by the client's own ground detection — and counts down a tick at a time.
//!
//! Metadata indices: `Entity`'s base slots are 0-7, then `AbstractArrow.ID_FLAGS` (8, a byte:
//! bit 1 crit, bit 2 no-physics — the client reads the flag, `AbstractArrow.isNoPhysics`),
//! `PIERCE_LEVEL` (9) and `IN_GROUND` (10, a bool); `Arrow.ID_EFFECT_COLOR` (11, an int)
//! carries the potion colour of a tipped arrow.

use haste_core::entity::MetadataValue;
use haste_core::model::{ArrowDecoration, Decoration};
use haste_core::DVec3;

use crate::store::Entity;

/// `AbstractArrow.ID_FLAGS` (a byte): bit 1 `setCritArrow`, bit 2 no-physics
/// (`AbstractArrow.isNoPhysics` reads it client side).
pub const FLAGS_INDEX: u8 = 8;
pub const CRIT_FLAG: u8 = 0x01;
pub const NO_PHYSICS_FLAG: u8 = 0x02;
/// `AbstractArrow.IN_GROUND` (a bool).
pub const IN_GROUND_INDEX: u8 = 10;
/// `Arrow.ID_EFFECT_COLOR` (an int; -1 means no potion).
pub const EFFECT_COLOR_INDEX: u8 = 11;
/// `Entity.DATA_NO_GRAVITY` (a bool, one of the base `Entity` slots).
pub const NO_GRAVITY_INDEX: u8 = 5;

/// `AbstractArrow.onSyncedDataUpdated`: the shake a freshly stuck arrow runs for.
pub const SHAKE_TICKS: u8 = 7;
/// `AbstractArrow.getDefaultGravity`.
pub const GRAVITY: f64 = 0.05;
/// `AbstractArrow.getAirDrag` (`applyInertia` scales the whole velocity by it).
pub const AIR_DRAG: f64 = 0.99;
/// `AbstractArrow.getWaterInertia`.
pub const WATER_INERTIA: f64 = 0.6;
/// `Projectile.lerpRotation`'s blend factor.
pub const ROTATION_LERP: f32 = 0.2;

/// Whether `kind` is an arrow we render (`minecraft:arrow`, `minecraft:spectral_arrow`).
/// Modded arrow types fall through to the generic box, like any unknown entity. The kind
/// ids resolve from the registry once (this runs per entity per tick, so it must not look
/// the name up every time).
pub fn is_arrow(kind: haste_core::RegistryIndex) -> bool {
    use std::sync::OnceLock;
    static KINDS: OnceLock<(usize, Option<usize>, Option<usize>)> = OnceLock::new();
    let (len, arrow, spectral) = *KINDS.get_or_init(|| {
        let names = crate::registry::entity_type_names();
        let find = |name: &str| {
            names.and_then(|n| {
                n.iter()
                    .find(|(_, v)| **v == name)
                    .map(|(k, _)| k.0 as usize)
            })
        };
        (
            names.map_or(0, |n| n.len()),
            find("minecraft:arrow"),
            find("minecraft:spectral_arrow"),
        )
    });
    let i = kind.0 as usize;
    // Without the registry (no cache) nothing is an arrow, like the name lookup would answer.
    i < len && (Some(i) == arrow || Some(i) == spectral)
}

/// The potion colour of a tipped arrow (`Arrow.getColor`), `-1` when it has none.
pub fn effect_color(metadata: &crate::metadata::Metadata) -> i32 {
    match metadata.get(EFFECT_COLOR_INDEX).map(|e| &e.value) {
        Some(MetadataValue::VarInt(v)) => *v,
        _ => -1,
    }
}

/// Whether the arrow is a critical one (`AbstractArrow.isCritArrow`, flag bit 1).
pub fn is_crit(metadata: &crate::metadata::Metadata) -> bool {
    match metadata.get(FLAGS_INDEX).map(|e| &e.value) {
        Some(MetadataValue::Byte(b)) => (*b as u8 & CRIT_FLAG) != 0,
        _ => false,
    }
}

/// Whether the client simulates the arrow's flight (`AbstractArrow.isNoPhysics` reads the
/// synced flag bit 2 on the client).
fn no_physics(metadata: &crate::metadata::Metadata) -> bool {
    match metadata.get(FLAGS_INDEX).map(|e| &e.value) {
        Some(MetadataValue::Byte(b)) => (*b as u8 & NO_PHYSICS_FLAG) != 0,
        _ => false,
    }
}

/// `Entity.isNoGravity` (`Entity.DATA_NO_GRAVITY`, the boolean at index 5).
fn no_gravity(metadata: &crate::metadata::Metadata) -> bool {
    matches!(
        metadata.get(NO_GRAVITY_INDEX).map(|e| &e.value),
        Some(MetadataValue::Boolean(true))
    )
}

/// `ArrowRenderer.extractRenderState`: the render state of an arrow entity (its interpolated
/// rotation, the shake, the tipped texture choice), `None` for entity types that are not
/// arrows.
pub fn decoration(e: &Entity, partial_tick: f32) -> Option<Decoration> {
    if !is_arrow(e.kind) {
        return None;
    }
    let spectral = crate::registry::entity_type_name(e.kind) == Some("minecraft:spectral_arrow");
    Some(Decoration::Arrow(ArrowDecoration {
        x_rot: e.pitch_at(partial_tick),
        y_rot: e.yaw_at(partial_tick),
        shake: f32::from(e.shake_time) - partial_tick,
        // Only `minecraft:arrow` is tippable: the spectral arrow's renderer never reads a
        // colour (`SpectralArrowRenderer`), so a stale slot 11 on one cannot tip it.
        tipped: !spectral && effect_color(&e.metadata) > 0,
        spectral,
    }))
}

/// `AbstractArrow.onSyncedDataUpdated`, for the metadata packets: the in-ground flag flipping
/// to true starts the shake (unless it already runs or the entity has not ticked yet —
/// vanilla's `firstTick`). Any packet also re-states the flag itself, which is what the
/// client would otherwise have found on its own.
pub fn on_metadata(e: &mut Entity, entries: &[haste_core::entity::MetadataEntry]) {
    for entry in entries {
        if entry.index != IN_GROUND_INDEX {
            continue;
        }
        if let MetadataValue::Boolean(ground) = entry.value {
            e.arrow_in_ground = ground;
            if ground && e.shake_time == 0 && e.tick_count >= 1 {
                e.shake_time = SHAKE_TICKS;
            }
        }
    }
}

/// A particle the arrow's tick asks for: vanilla `Arrow.makeParticle` (the tipped arrow's
/// potion colour, `ColorParticleOption` over `minecraft:entity_effect`), the critical
/// arrow's trail (`AbstractArrow.tick`'s four `ParticleTypes.CRIT`) and the spectral arrow's
/// per-tick swirl (`SpectralArrow.tick`'s white `SpellParticleOption` over
/// `minecraft:effect`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrowParticle {
    pub pos: DVec3,
    /// The particle's velocity (the crit trail's, zero for the others).
    pub vel: DVec3,
    pub kind: ArrowParticleKind,
}

/// Which particle an [`ArrowParticle`] is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArrowParticleKind {
    /// `ParticleTypes.CRIT` with the packet's velocity.
    Crit,
    /// `entity_effect` tinted with a potion colour (ARGB).
    Color(i32),
    /// `effect` with `SpellParticleOption` white (`SpectralArrow.tick`).
    SpectralSwirl,
}

/// The world `AbstractArrow.tick` reads on the client: the block it sticks into, the ray clip
/// of its movement and the water it flies through.
pub trait ArrowWorld {
    /// Whether the block at `block` has a collision shape containing the point `p`
    /// (`AbstractArrow.tick`'s stuck check: `state.getCollisionShape(...).toAabbs()` moved by
    /// the block position, `contains(position)`).
    fn collision_contains(&self, block: haste_core::BlockPos, p: DVec3) -> bool;
    /// `Level.clipIncludingBorder` with `ClipContext.Block.COLLIDER`: where the segment
    /// `from .. to` first meets a block's collision shape (the world border half is not
    /// modelled), `None` when it does not.
    fn clip(&self, from: DVec3, to: DVec3) -> Option<DVec3>;
    /// `Entity.isInWater()`.
    fn in_water(&self, pos: DVec3) -> bool;
}

/// The world when none is available: no blocks and no water, so arrows fly straight and
/// never stick (the same stand-in `item_entity::NoWorld` gives the dropped items).
pub struct NoArrowWorld;

impl ArrowWorld for NoArrowWorld {
    fn collision_contains(&self, _block: haste_core::BlockPos, _p: DVec3) -> bool {
        false
    }
    fn clip(&self, _from: DVec3, _to: DVec3) -> Option<DVec3> {
        None
    }
    fn in_water(&self, _pos: DVec3) -> bool {
        false
    }
}

/// One `AbstractArrow.tick` on the client, in vanilla's order: the stuck check at the
/// position, the shake countdown, the in-ground idle (its time counts, `Arrow.makeParticle`
/// runs every fifth tick) or the flight step (crit particles, the rotation from the
/// movement, the clip, the drag and gravity).
pub(crate) fn tick(e: &mut Entity, world: &dyn ArrowWorld, particles: &mut Vec<ArrowParticle>) {
    let physics = !no_physics(&e.metadata);
    // The shake only starts once the entity has ticked (vanilla's `firstTick` guard, false
    // from the end of the first tick).
    let can_shake = e.tick_count > 1;
    // The block the arrow sits in: `blockPosition()`, the floor of the position.
    let block = haste_core::BlockPos::from_dvec3(e.pos);
    if physics && world.collision_contains(block, e.pos) {
        // Stuck: no movement, in ground, and a fresh stick starts the shake.
        e.velocity = DVec3::ZERO;
        if !e.arrow_in_ground {
            e.arrow_in_ground = true;
            if can_shake && e.shake_time == 0 {
                e.shake_time = SHAKE_TICKS;
            }
        }
    }

    if e.shake_time > 0 {
        e.shake_time -= 1;
    }

    let color = if is_arrow_tippable(e.kind) {
        effect_color(&e.metadata)
    } else {
        -1
    };
    // The in-ground branch itself needs physics enabled (vanilla gates it the same way, so a
    // no-physics arrow stuck in a block keeps flying through the flight branch).
    if e.arrow_in_ground && physics {
        // `Arrow.tick`: a stuck tipped arrow breathes one colour particle every fifth tick
        // (`inGroundTime % 5 == 0`).
        e.in_ground_time = e.in_ground_time.saturating_add(1);
        if color != -1 && e.in_ground_time.is_multiple_of(5) {
            particles.push(ArrowParticle {
                pos: random_in_box(e, 0),
                vel: DVec3::ZERO,
                kind: ArrowParticleKind::Color(color),
            });
        }
        return;
    }

    e.in_ground_time = 0;
    let movement = e.velocity;
    let original = e.pos;
    let in_water = world.in_water(e.pos);
    if in_water {
        // `applyInertia(getWaterInertia())`; the bubble trail is not modelled.
        e.velocity *= WATER_INERTIA;
    }
    // `AbstractArrow.tick`: four crit particles along the step, at quarters of the movement.
    if is_crit(&e.metadata) {
        for i in 0..4 {
            particles.push(ArrowParticle {
                pos: original + movement * f64::from(i) / 4.0,
                vel: DVec3::new(-movement.x, -movement.y + 0.2, -movement.z),
                kind: ArrowParticleKind::Crit,
            });
        }
    }
    // `Arrow.tick`: a flying tipped arrow breathes two colour particles a tick.
    if color != -1 {
        particles.push(ArrowParticle {
            pos: random_in_box(e, 0),
            vel: DVec3::ZERO,
            kind: ArrowParticleKind::Color(color),
        });
        particles.push(ArrowParticle {
            pos: random_in_box(e, 1),
            vel: DVec3::ZERO,
            kind: ArrowParticleKind::Color(color),
        });
    }
    // The rotation from the movement (`AbstractArrow.tick`; `Projectile.lerpRotation` blends
    // a fifth of the way). No-physics arrows (bit 2) negate the horizontal axes.
    let y_rot = if physics {
        movement.x.atan2(movement.z)
    } else {
        (-movement.x).atan2(-movement.z)
    };
    let horizontal = (movement.x * movement.x + movement.z * movement.z).sqrt();
    let x_rot = movement.y.atan2(horizontal);
    e.yaw = crate::store::rot_lerp(ROTATION_LERP, e.yaw, y_rot.to_degrees() as f32);
    e.pitch = crate::store::rot_lerp(ROTATION_LERP, e.pitch, x_rot.to_degrees() as f32);

    if physics {
        // `stepMoveAndHit`: the clip decides how far the movement goes; meeting a block
        // plants the arrow at the hit.
        match world.clip(original, original + movement) {
            Some(hit) => {
                e.pos = hit;
                e.velocity = DVec3::ZERO;
                e.arrow_in_ground = true;
                if can_shake && e.shake_time == 0 {
                    e.shake_time = SHAKE_TICKS;
                }
            }
            None => e.pos = original + movement,
        }
    } else {
        e.pos = original + movement;
    }

    if !in_water {
        e.velocity *= AIR_DRAG;
    }
    // `applyGravity` (a no-gravity arrow's `getGravity()` is 0, so nothing applies).
    if physics && !e.arrow_in_ground && !no_gravity(&e.metadata) {
        e.velocity.y -= GRAVITY;
    }
    // `SpectralArrow.tick` runs after `super.tick()`: a flying spectral arrow breathes one
    // white `effect` particle a tick, at its exact position (a freshly stuck arrow stops).
    if e.spectral && !e.arrow_in_ground {
        particles.push(ArrowParticle {
            pos: e.pos,
            vel: DVec3::ZERO,
            kind: ArrowParticleKind::SpectralSwirl,
        });
    }
}

/// Only `minecraft:arrow` is tippable (`Arrow`); the spectral arrow has no colour.
fn is_arrow_tippable(kind: haste_core::RegistryIndex) -> bool {
    crate::registry::entity_type_name(kind) == Some("minecraft:arrow")
}

/// `Arrow.makeParticle`'s position: `getRandomX(0.5)` / `getRandomY()` / `getRandomZ(0.5)`,
/// a random spot inside the arrow's 0.5 x 0.5 box (a deterministic hash of the entity and the
/// tick stands in for vanilla's shared `RandomSource`).
fn random_in_box(e: &Entity, salt: u32) -> DVec3 {
    random_box_point(
        e.pos,
        e.dimensions,
        e.id.0 as u32,
        e.tick_count as u32,
        salt,
    )
}

/// One random spot in an entity's box, as `Entity.getRandomX(0.5)` / `getRandomY()` /
/// `getRandomZ(0.5)` place one: ± half the width on x and z, anywhere up the height on y.
fn random_box_point(pos: DVec3, dims: (f32, f32), seed: u32, tick: u32, salt: u32) -> DVec3 {
    let rx = random_unit(seed, tick, salt);
    let ry = random_unit(seed, tick, salt + 1);
    let rz = random_unit(seed, tick, salt + 2);
    DVec3::new(
        pos.x + f64::from(dims.0) * f64::from((rx * 2.0 - 1.0) * 0.5),
        pos.y + f64::from(dims.1) * f64::from(ry),
        pos.z + f64::from(dims.0) * f64::from((rz * 2.0 - 1.0) * 0.5),
    )
}

/// A deterministic 0..1 hash (the position jitter of the colour particles; vanilla draws it
/// from the entity's `RandomSource`, seeded per entity).
fn random_unit(a: u32, b: u32, c: u32) -> f32 {
    let mut h = a
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(b.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(c.wrapping_mul(0xC2B2_AE35));
    h ^= h >> 13;
    h = h.wrapping_mul(0x27D4_EB2F);
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// `Arrow.handleEntityEvent` 0 (the potion puff when a stuck tipped arrow reverts): 20
/// colour particles over the arrow's box, queued into `out`; `false` when the entity is not
/// a tipped arrow.
pub fn potion_puff(e: &Entity, out: &mut Vec<ArrowParticle>) -> bool {
    let color = if is_arrow_tippable(e.kind) {
        effect_color(&e.metadata)
    } else {
        -1
    };
    if color == -1 {
        return false;
    }
    for i in 0..20 {
        out.push(ArrowParticle {
            pos: random_box_point(e.pos, e.dimensions, e.id.0 as u32, 0, i),
            vel: DVec3::ZERO,
            kind: ArrowParticleKind::Color(color),
        });
    }
    true
}
