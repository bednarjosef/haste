//! The local player's fall sounds, ported from vanilla's landing chain:
//! `Entity.checkFallDamage` -> `Block.fallOn` -> `Player.causeFallDamage` ->
//! `LivingEntity.causeFallDamage` (`calculateFallDamage`, `getFallDamageSound`,
//! `playBlockFallSound`). The server excludes the acting player from its broadcasts, so
//! like every own sound the client plays these itself.

use glam::DVec3;
use haste_audio::random::LegacyRandom;
use haste_core::BlockStateId;

use super::{PlayRequest, SoundPos, SoundRef, SoundRequest};

/// `SoundEvents.PLAYER_SMALL_FALL` (`Player.getFallSounds`): the name is the sound
/// event's registry id, resolved against `sounds.json` like every inline event.
const SMALL_FALL: &str = "minecraft:entity.player.small_fall";
/// `SoundEvents.PLAYER_BIG_FALL`.
const BIG_FALL: &str = "minecraft:entity.player.big_fall";
/// `SoundEvents.HONEY_BLOCK_SLIDE` (`HoneyBlock.fallOn` plays it on every landing on honey).
const HONEY_SLIDE: &str = "minecraft:block.honey_block.slide";

/// `Attributes.SAFE_FALL_DISTANCE`'s default (the player's safe fall distance).
const SAFE_FALL_DISTANCE: f64 = 3.0;
/// The damage modifier of hay and honey (`HayBlock.fallOn` / `HoneyBlock.fallOn`).
const HAY_HONEY_MODIFIER: f64 = 0.2;
/// The epsilon vanilla's `calculateFallPower` adds before flooring (`fallDistance + 1.0E-6`).
const FALL_POWER_EPSILON: f64 = 1.0e-6;
/// `LivingEntity.getFallDamageSound`: big above damage 4, small at or below.
const BIG_FALL_MIN_DAMAGE: i32 = 4;
/// `PowderSnowBlock.fallOn`: no fall sound below this distance, small below
/// `POWDER_SNOW_BIG_FALL_DISTANCE`, big above.
const POWDER_SNOW_MIN_DISTANCE: f64 = 4.0;
const POWDER_SNOW_BIG_FALL_DISTANCE: f64 = 7.0;

/// The block the player landed on, as far as fall sounds care. Vanilla picks the block
/// at `getOnPosLegacy` (feet y minus 0.2) in `playBlockFallSound` and the one `fallOn`
/// received; both are the supporting block, which the caller resolves once.
#[derive(Clone, Copy)]
pub struct LandedOn<'a> {
    /// Its sound type, from the oracle dump (`BlockState.getSoundType`).
    pub sound: &'a haste_data::BlockSoundType,
}

/// How the landed-on block changes vanilla's fall-damage chain. Defaults to the plain
/// `Block.fallOn` (modifier 1.0, no distance reduction); named after the vanilla class
/// that overrides it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FallBlockRule {
    /// `Block.fallOn`: `causeFallDamage(fallDistance * (1 - reduction), 1.0)`.
    Default,
    /// `BlockBehaviour.fallDistanceReduction(0.5)` (beds, shelf mushroom): the fall
    /// distance itself is halved before the damage.
    HalvesFallDistance,
    /// `HayBlock.fallOn`: `causeFallDamage(fallDistance, 0.2F)`.
    Hay,
    /// `SlimeBlock.fallOn`: `causeFallDamage(fallDistance, 0.0F)` — no damage, no sound.
    Slime,
    /// `HoneyBlock.fallOn`: slide sound first, then `causeFallDamage(fallDistance, 0.2F)`
    /// and, when it damages, the block fall sound a second time (vanilla plays both).
    Honey,
    /// `PowderSnowBlock.fallOn`: no fall damage; the small/big fall sound plays by fall
    /// distance alone (4+ small, 7+ big).
    PowderSnow,
}

/// Which fall sounds a landing plays, decided by vanilla's chain. Built per landing;
/// `requests` carries the result to the normal `Sounds::apply` path.
pub struct FallSounds {
    /// `ClientLevel.random`, seeding each sound instance like `playLocalSound` does.
    random: LegacyRandom,
}

impl FallBlockRule {
    /// The rule of the block `state` landed on, from the server-synced block tags and
    /// the block-state table (`#minecraft:beds` decides the halving, so modded beds
    /// halve too; the blocks vanilla overrides by class are matched by registry name
    /// like `SyncedClimbables` matches scaffolding). Everything unknown is `Default`.
    pub fn of(
        state: BlockStateId,
        tags: Option<&haste_physics::mining::BlockTags>,
        blocks: Option<&haste_data::BlockStates>,
    ) -> Self {
        let Some(blocks) = blocks else {
            return Self::Default;
        };
        let name = blocks.block_of(state);
        if name.namespace() != "minecraft" {
            return Self::Default;
        }
        let is = |path: &str| name.path() == path;
        if is("hay_block") {
            Self::Hay
        } else if is("slime_block") {
            Self::Slime
        } else if is("honey_block") {
            Self::Honey
        } else if is("powder_snow") {
            Self::PowderSnow
        } else if is("shelf_mushroom")
            || (is_beds(tags, blocks, state) && name.path() != "straw_bed")
        {
            // Beds halve the fall distance (BlockBehaviour.fallDistanceReduction 0.5);
            // vanilla's straw bed is a StrawBedBlock without the reduction.
            Self::HalvesFallDistance
        } else {
            Self::Default
        }
    }
}

/// `#minecraft:beds` membership of `state`'s block, through the synced tags.
fn is_beds(
    tags: Option<&haste_physics::mining::BlockTags>,
    blocks: &haste_data::BlockStates,
    state: BlockStateId,
) -> bool {
    tags.is_some_and(|t| {
        blocks
            .block_registry_index_of_state(state)
            .is_some_and(|i| t.contains(&haste_core::Identifier::minecraft("beds"), i.0))
    })
}

impl FallSounds {
    pub fn new() -> Self {
        Self {
            random: LegacyRandom::new(super::own_block::unseeded()),
        }
    }

    /// A landing this tick (`!was_on_ground && on_ground`, vanilla's `checkFallDamage`
    /// runs inside the move that grounds the player). `fall_distance` is what vanilla
    /// would pass to `fallOn`: the accumulated distance **including this tick's final
    /// fall** (our physics resets it on grounding, so the caller adds the last tick's
    /// downward movement back). `may_fly` is vanilla's `Player.causeFallDamage` early
    /// return: creative-style players never play fall sounds.
    pub fn landed(
        &mut self,
        player_pos: DVec3,
        fall_distance: f64,
        may_fly: bool,
        landed_on: Option<LandedOn<'_>>,
        rule: FallBlockRule,
        requests: &mut Vec<SoundRequest>,
    ) {
        if may_fly {
            return; // Player.causeFallDamage: mayfly players take no fall damage at all.
        }
        let Some(on) = landed_on else {
            return;
        };
        match rule {
            FallBlockRule::Slime => {}
            FallBlockRule::PowderSnow => {
                // PowderSnowBlock.fallOn: no damage, the fall sound by distance alone.
                if fall_distance >= POWDER_SNOW_MIN_DISTANCE {
                    let event = if fall_distance < POWDER_SNOW_BIG_FALL_DISTANCE {
                        SMALL_FALL
                    } else {
                        BIG_FALL
                    };
                    self.play(event, player_pos, 1.0, 1.0, requests);
                }
            }
            FallBlockRule::Honey => {
                // HoneyBlock.fallOn: the slide sound on every landing, then the 0.2 chain.
                self.play(HONEY_SLIDE, player_pos, 1.0, 1.0, requests);
                if Self::damage(fall_distance, HAY_HONEY_MODIFIER) > 0 {
                    self.fall_damage_sounds(
                        player_pos,
                        &on,
                        fall_distance,
                        HAY_HONEY_MODIFIER,
                        requests,
                    );
                    // Honey's fallOn plays the block fall sound itself when the landing
                    // damaged, on top of playBlockFallSound inside the damage chain.
                    self.block_fall(player_pos, &on, requests);
                }
            }
            FallBlockRule::Hay => {
                self.fall_damage_sounds(
                    player_pos,
                    &on,
                    fall_distance,
                    HAY_HONEY_MODIFIER,
                    requests,
                );
            }
            FallBlockRule::HalvesFallDistance => {
                self.fall_damage_sounds(player_pos, &on, fall_distance * 0.5, 1.0, requests);
            }
            FallBlockRule::Default => {
                self.fall_damage_sounds(player_pos, &on, fall_distance, 1.0, requests);
            }
        }
    }

    /// `LivingEntity.causeFallDamage` for a landing on `on` with fall distance
    /// `fall_distance` and the block's damage `modifier`: the small/big fall sound and
    /// the block fall sound when the damage is above zero.
    fn fall_damage_sounds(
        &mut self,
        player_pos: DVec3,
        on: &LandedOn<'_>,
        fall_distance: f64,
        modifier: f64,
        requests: &mut Vec<SoundRequest>,
    ) {
        let damage = Self::damage(fall_distance, modifier);
        if damage > 0 {
            // LivingEntity.getFallDamageSound + Player.getFallSounds.
            let event = if damage > BIG_FALL_MIN_DAMAGE {
                BIG_FALL
            } else {
                SMALL_FALL
            };
            self.play(event, player_pos, 1.0, 1.0, requests);
            self.block_fall(player_pos, on, requests);
        }
    }

    /// `LivingEntity.calculateFallDamage` with the player's base attributes (safe fall
    /// distance 3.0, fall damage multiplier 1.0 — no attribute modifiers yet):
    /// `floor((fallDistance + 1.0E-6 - safeFallDistance) * damageModifier)`.
    fn damage(fall_distance: f64, modifier: f64) -> i32 {
        ((fall_distance + FALL_POWER_EPSILON - SAFE_FALL_DISTANCE) * modifier).floor() as i32
    }

    /// `LivingEntity.playBlockFallSound`: the landed-on block's fall sound at
    /// `volume * 0.5`, `pitch * 0.75` (PLAYER source).
    fn block_fall(
        &mut self,
        player_pos: DVec3,
        on: &LandedOn<'_>,
        requests: &mut Vec<SoundRequest>,
    ) {
        let t = on.sound;
        self.play(
            &t.fall_sound,
            player_pos,
            t.volume * 0.5,
            t.pitch * 0.75,
            requests,
        );
    }

    /// One own-sound request, an inline event the `Sounds::apply` path resolves
    /// (dropped as unknown when `sounds.json` does not know it, logged once).
    fn play(
        &mut self,
        event: &str,
        pos: DVec3,
        volume: f32,
        pitch: f32,
        requests: &mut Vec<SoundRequest>,
    ) {
        if event == "minecraft:intentionally_empty" {
            return; // vanilla's "no sound" event
        }
        requests.push(SoundRequest::Play(PlayRequest {
            sound: SoundRef::Inline(event.to_owned()),
            category: CATEGORY_PLAYERS,
            pos: SoundPos::Fixed(pos),
            volume,
            pitch,
            seed: self.random.next_long(),
        }));
    }
}

impl Default for FallSounds {
    fn default() -> Self {
        Self::new()
    }
}

/// `SoundSource.PLAYERS`, the category of the player's own sounds (`getSoundSource`).
const CATEGORY_PLAYERS: i32 = 7;
