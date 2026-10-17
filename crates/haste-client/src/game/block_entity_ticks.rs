// The game thread's block entity tickers for the object-model block entities (vanilla's
// `BlockEntityTicker`s run by `Level.tickBlockEntities` on the client): the bell shake, the
// decorated pot wobble, the enchanting table's book, the conduit's spin and shape, and the
// moving piston's progress. The state lives per block entity in the shared animation map the
// renderer interpolates.

use glam::DVec3;
use haste_protocol::haste_core::{BlockPos, Direction, Identifier};
use haste_world::World;

/// The page-flip random (vanilla `EnchantingTableBlockEntity`'s unseeded `RandomSource`, whose
/// values only decide when the book's pages flip): a fixed seed keeps Haste's runs
/// reproducible.
pub(super) static MODEL_RNG: parking_lot::Mutex<u64> =
    parking_lot::Mutex::new(0x2545_f491_4f6c_dd1d);

/// `RandomSource.nextInt(bound)`: vanilla's 26.3 `XoroshiroRandomSource.nextInt` — the unsigned
/// 32-bit draw multiplied by the bound, with the rejection loop that removes the modulo bias.
fn model_next_int(rng: &mut u64, bound: u32) -> u32 {
    const MASK: u64 = 0xFFFF_FFFF;
    let mut bits = (*rng >> 32) & MASK;
    let mut mul = bits * bound as u64;
    let mut frac = mul & MASK;
    if frac < bound as u64 {
        let start = (u32::MAX - bound + 1).wrapping_rem(bound) as u64;
        while frac < start {
            *rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            bits = (*rng >> 32) & MASK;
            mul = bits * bound as u64;
            frac = mul & MASK;
        }
    }
    (mul >> 32) as u32
}

/// One tick of vanilla's block entity tickers for the object-model block entities: the bell
/// shake (`BellBlockEntity.tick`), the decorated pot wobble (`DecoratedPotBlockEntity.triggerEvent`
/// with the game clock), the enchanting table's book (`EnchantingTableBlockEntity.bookAnimationTick`)
/// and the conduit's spin and active flag (`ConduitBlockEntity.clientTick`). Entries whose block
/// entity is gone are dropped, like the skulls above.
pub(super) struct TickModelAnims<'a> {
    pub(super) anims: &'a mut std::collections::HashMap<BlockPos, haste_render::BlockEntityAnim>,
    pub(super) world: &'a World,
    pub(super) blocks: &'a haste_data::BlockStates,
    pub(super) names: &'a [Identifier],
    pub(super) tags: Option<&'a haste_physics::mining::BlockTags>,
    pub(super) world_age: i64,
    pub(super) feet: DVec3,
    pub(super) rng: &'a mut u64,
    pub(super) bell_events: &'a [(BlockPos, Option<Direction>)],
    pub(super) pot_events: &'a [(BlockPos, i64, u32)],
}

pub(super) fn tick_model_anims(t: TickModelAnims<'_>) {
    use haste_render::BlockEntityAnim as A;
    t.anims.retain(|pos, anim| {
        let Some(be) = t.world.block_entity(*pos) else {
            return false;
        };
        let is_kind = t
            .names
            .get(be.kind as usize)
            .is_some_and(|n| n.namespace() == "minecraft" && kind_matches(n.path(), anim));
        if !is_kind {
            return false;
        }
        match anim {
            A::EnchantTable {
                time,
                flip,
                open,
                rot,
                t_rot,
                flip_t,
                flip_a,
            } => {
                // `bookAnimationTick`: a player within 3 blocks opens the book and turns it
                // toward them (the client level only knows the local player).
                let near = {
                    let dx = t.feet.x - (pos.x as f64 + 0.5);
                    let dy = t.feet.y - (pos.y as f64 + 0.5);
                    let dz = t.feet.z - (pos.z as f64 + 0.5);
                    dx * dx + dy * dy + dz * dz <= 9.0
                };
                flip.start = flip.end;
                open.start = open.end;
                rot.start = rot.end;
                if near {
                    *t_rot = (t.feet.z - (pos.z as f64 + 0.5))
                        .atan2(t.feet.x - (pos.x as f64 + 0.5)) as f32;
                    open.end += 0.1;
                    if open.end < 0.5 || model_next_int(t.rng, 40) == 0 {
                        let old = *flip_t;
                        loop {
                            let a = model_next_int(t.rng, 4) as i32;
                            let b = model_next_int(t.rng, 4) as i32;
                            *flip_t += (a - b) as f32;
                            if old != *flip_t {
                                break;
                            }
                        }
                    }
                } else {
                    *t_rot += 0.02;
                    open.end -= 0.1;
                }
                wrap_pi(&mut rot.end);
                wrap_pi(t_rot);
                let mut rot_dir = *t_rot - rot.end;
                wrap_pi(&mut rot_dir);
                rot.end += rot_dir * 0.4;
                open.end = open.end.clamp(0.0, 1.0);
                *time += 1.0;
                let diff = ((*flip_t - flip.end) * 0.4).clamp(-0.2, 0.2);
                *flip_a += (diff - *flip_a) * 0.9;
                flip.end += *flip_a;
            }
            A::Bell { ticks, click } => {
                ticks.start = ticks.end;
                if ticks.end >= 50.0 {
                    // Vanilla stops the shake and zeroes the counter; the click direction goes
                    // with it (`shakeDirection` is null when not shaking).
                    ticks.end = 0.0;
                    *click = None;
                } else {
                    ticks.end += 1.0;
                }
            }
            A::Pot { wobble, duration } => {
                // The wobble runs from the event's game tick for the style's duration.
                let (start, event_duration) = t
                    .pot_events
                    .iter()
                    .find(|(p, _, _)| p == pos)
                    .map(|(_, s, d)| (*s, *d))
                    .unwrap_or((0, *duration));
                let elapsed = (t.world_age - start) as f32;
                wobble.start = elapsed - 1.0;
                wobble.end = elapsed;
                *duration = event_duration;
                if elapsed > event_duration as f32 {
                    return false;
                }
            }
            A::Conduit {
                tick_count,
                rotation,
                active,
                hunting,
            } => {
                *tick_count += 1.0;
                // Vanilla re-reads the shape every 40 ticks of the world's game time.
                if t.world_age % 40 == 0 {
                    let shape = conduit_shape(t.world, t.blocks, t.tags, *pos);
                    *active = shape.active;
                    *hunting = shape.hunting;
                }
                if *active {
                    *rotation += 1.0;
                }
            }
            A::Piston { progress } => {
                // Vanilla `PistonMovingBlockEntity.tick`: 0.5 per tick to 1.0; the
                // block entity is removed after (the entry drops when the server's
                // block update arrives).
                progress.start = progress.end;
                progress.end = (progress.end + 0.5).min(1.0);
            }
        }
        true
    });
    // New entries: the bells and pots the events named, and every conduit or enchanting table
    // the world has that is not ticking yet (a pot or bell without an event has no animation).
    for (pos, click) in t.bell_events {
        t.anims.entry(*pos).or_insert(A::Bell {
            ticks: haste_render::TickPair {
                start: 0.0,
                end: 0.0,
            },
            click: *click,
        });
    }
    for (pos, start, duration) in t.pot_events {
        let elapsed = (t.world_age - *start) as f32;
        t.anims.entry(*pos).or_insert(A::Pot {
            wobble: haste_render::TickPair {
                start: elapsed - 1.0,
                end: elapsed,
            },
            duration: *duration,
        });
    }
    'outer: for chunk in t.world.loaded_chunks() {
        for (pos, be) in t.world.block_entities_in(chunk) {
            let Some(name) = t.names.get(be.kind as usize) else {
                continue;
            };
            if name.namespace() != "minecraft" {
                continue;
            }
            let anim = match name.path() {
                "conduit" => {
                    let shape = conduit_shape(t.world, t.blocks, t.tags, pos);
                    A::Conduit {
                        tick_count: 0.0,
                        rotation: 0.0,
                        active: shape.active,
                        hunting: shape.hunting,
                    }
                }
                "enchanting_table" => A::EnchantTable {
                    time: 0.0,
                    flip: haste_render::TickPair::fixed(0.0),
                    open: haste_render::TickPair::fixed(0.0),
                    rot: haste_render::TickPair::fixed(0.0),
                    t_rot: 0.0,
                    flip_t: 0.0,
                    flip_a: 0.0,
                },
                // The moving piston starts at its NBT progress (the server sends it
                // with the block entity).
                "piston" => A::Piston {
                    progress: haste_render::TickPair::fixed(piston_nbt_progress(&be.nbt)),
                },
                _ => continue,
            };
            t.anims.entry(pos).or_insert(anim);
            if t.anims.len() > 4096 {
                break 'outer;
            }
        }
    }
}

/// Whether a block entity type name still matches the animation state it started with.
fn kind_matches(path: &str, anim: &haste_render::BlockEntityAnim) -> bool {
    use haste_render::BlockEntityAnim as A;
    match anim {
        A::Bell { .. } => path == "bell",
        A::Pot { .. } => path == "decorated_pot",
        A::Conduit { .. } => path == "conduit",
        A::EnchantTable { .. } => path == "enchanting_table",
        A::Piston { .. } => path == "piston",
    }
}

/// The moving piston's NBT progress (vanilla `PistonMovingBlockEntity.loadAdditional`).
fn piston_nbt_progress(nbt: &[u8]) -> f32 {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper)
        .ok()
        .and_then(|doc| doc.root_tag.get_float("progress"))
        .unwrap_or(0.0)
}

/// Vanilla's wrap-to-[-PI, PI) loops, as an in-place fix.
fn wrap_pi(v: &mut f32) {
    while *v >= std::f32::consts::PI {
        *v -= std::f32::consts::TAU;
    }
    while *v < -std::f32::consts::PI {
        *v += std::f32::consts::TAU;
    }
}

/// The conduit's shape, refreshed every 40 ticks (vanilla
/// `ConduitBlockEntity.updateShape`/`updateHunting`).
#[derive(Clone, Copy, Default)]
struct ConduitShape {
    active: bool,
    hunting: bool,
}

fn conduit_shape(
    world: &World,
    blocks: &haste_data::BlockStates,
    tags: Option<&haste_physics::mining::BlockTags>,
    pos: BlockPos,
) -> ConduitShape {
    // The 3x3x3 around the conduit must be water.
    for ox in -1..=1i32 {
        for oy in -1..=1i32 {
            for oz in -1..=1i32 {
                let p = pos.add(ox, oy, oz);
                let block = blocks.block_of(world.block_state(p));
                if block.namespace() != "minecraft" || block.path() != "water" {
                    return ConduitShape::default();
                }
            }
        }
    }
    // The ring's `conduit_effect_block` tag entries (16 activate the conduit, 42 make it hunt).
    let tag = Identifier::minecraft("conduit_effect_block");
    let mut effect_blocks = 0;
    for ox in -2..=2i32 {
        for oy in -2..=2i32 {
            for oz in -2..=2i32 {
                let (ax, ay, az) = (ox.abs(), oy.abs(), oz.abs());
                let on_ring = (ax > 1 || ay > 1 || az > 1)
                    && (ox == 0 && (ay == 2 || az == 2)
                        || oy == 0 && (ax == 2 || az == 2)
                        || oz == 0 && (ax == 2 || ay == 2));
                if !on_ring {
                    continue;
                }
                let p = pos.add(ox, oy, oz);
                let block = blocks.block_of(world.block_state(p));
                if let (Some(index), Some(tags)) = (blocks.block_index(block), tags) {
                    if tags.contains(&tag, index.0) {
                        effect_blocks += 1;
                    }
                }
            }
        }
    }
    ConduitShape {
        active: effect_blocks >= 16,
        hunting: effect_blocks >= 42,
    }
}
