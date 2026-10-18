//! The crosshair target (player workstream): `Minecraft.pick` -> `LocalPlayer.pick` with
//! `ProjectileUtil.getEntityHitResult`'s entity search, and the per-type pickability rules
//! (`Entity.isPickable`, `Entity.getPickRadius`) the search filters with.

use glam::DVec3;
use haste_core::view::{BlockView, CollisionShapes};
use haste_core::{Aabb, BlockPos, Direction, EntityId, Identifier, RegistryIndex};
use haste_entity::store::HEALTH_METADATA_INDEX;
use haste_physics::raycast::{self, BlockHit};
use haste_protocol::haste_core::entity::MetadataValue;

use super::super::LocalPlayer;
use crate::net::EntitySink;

/// What the crosshair is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Block(BlockHit),
    Entity {
        id: EntityId,
        /// Where the crosshair hits it (`EntityHitResult.location`).
        location: DVec3,
        /// The entity's feet position the same tick (`MultiPlayerGameMode.interact` sends the
        /// hit location relative to it: `location - entity.position()`).
        pos: DVec3,
        /// The entity's `minecraft:entity_type` registry index (unknown — a mod type, or no
        /// type data — is the sentinel `u32::MAX`), for the use's fall-through rules.
        kind: RegistryIndex,
    },
}

impl LocalPlayer {
    /// Block under the crosshair and the face hit.
    pub fn target(&self) -> Option<(BlockPos, Direction)> {
        match self.int.target {
            Some(Target::Block(h)) => Some((h.pos, h.face)),
            _ => None,
        }
    }

    /// Entity under the crosshair.
    pub fn target_entity(&self) -> Option<EntityId> {
        match self.int.target {
            Some(Target::Entity { id, .. }) => Some(id),
            _ => None,
        }
    }

    /// Whether the crosshair picks a living, alive entity in reach (vanilla `Minecraft
    /// .crosshairPickEntity`, the HUD attack indicator's full-icon condition): the pick is
    /// already filtered to `entity_interaction_range`, so "in reach" holds by construction.
    /// Rust tracks no is-living registry: like the mount HUD's vehicle check, the health
    /// metadata's presence marks a living entity (every living entity's pairing data carries
    /// `LivingEntity.DATA_HEALTH_ID`), and its value being positive is "alive".
    pub fn crosshair_pick_living(&self, entities: &dyn EntitySink) -> bool {
        let Some(id) = self.target_entity() else {
            return false;
        };
        matches!(
            entities.metadata_value(id, HEALTH_METADATA_INDEX),
            Some(MetadataValue::Float(h)) if *h > 0.0
        )
    }

    /// The crosshair target for a look direction: `Minecraft.pick` ->
    /// `LocalPlayer.raycastHitResult` -> `LocalPlayer.pick`, with
    /// `ProjectileUtil.getEntityHitResult`'s `EntitySelector.CAN_BE_PICKED` filter.
    ///
    /// Block outlines are searched out to the larger of the two interaction ranges and then
    /// filtered back to `block_interaction_range` (`filterHitResult`); the entity segment reaches
    /// the block's distance when a block is hit, else the larger range, and an entity only wins
    /// when it is strictly closer than the block. The winner is filtered to
    /// `entity_interaction_range`, so an entity just out of reach makes the pick a miss rather
    /// than showing the block behind it.
    pub(super) fn pick(
        &self,
        look: (f32, f32),
        world: &dyn BlockView,
        shapes: &dyn CollisionShapes,
        data: &haste_data::GameData,
        entities: &[(EntityId, Aabb)],
        entity_types: &[RegistryIndex],
    ) -> Option<Target> {
        let eye = self.eye();
        let dir = raycast::look_vector(look.0, look.1);
        let a = self.int.attrs;
        let max_dist = a.block_reach.max(a.entity_reach);
        // LocalPlayer.pick: the block raycast reaches the larger range; `filterHitResult`
        // afterwards drops a block beyond `block_interaction_range`.
        let block = raycast::raycast_blocks(eye, dir, max_dist, world, shapes)
            .filter(|b| b.distance <= a.block_reach);
        let block_dist = block.as_ref().map_or(f64::INFINITY, |b| b.distance);
        // ProjectileUtil.getEntityHitResult: the nearest pickable box the segment touches. The
        // segment reaches the block's distance when a block was hit, else the larger range
        // (LocalPlayer.pick clamps maxDistance to the block distance); the winner is filtered
        // back to `entity_interaction_range` (filterHitResult).
        let search_dist = block_dist.min(max_dist);
        let delta = dir * search_dist;
        let mut best: Option<(usize, f64)> = None;
        for (i, (id, b)) in entities.iter().enumerate() {
            if *id == self.entity_id {
                continue; // getEntities(except = the camera entity)
            }
            let name = self.entity_type_name(data, entity_types, i);
            // EntitySelector.CAN_BE_PICKED = Entity::isPickable per type; unknown (mod)
            // types are pickable, like vanilla's default.
            if !name.is_none_or(is_pickable) {
                continue;
            }
            // Entity.getPickRadius: Projectile searches with 1.0 for pickable projectiles.
            let radius = name.and_then(entity_pick_radius).unwrap_or(0.0);
            let bb = b.expand(DVec3::splat(radius));
            // ProjectileUtil: an entity whose box holds the eye is picked at distance 0
            // (canBePickedFromInside).
            if bb.min.x <= eye.x
                && eye.x <= bb.max.x
                && bb.min.y <= eye.y
                && eye.y <= bb.max.y
                && bb.min.z <= eye.z
                && eye.z <= bb.max.z
            {
                if best.is_none_or(|(_, bd)| 0.0 < bd) {
                    best = Some((i, 0.0));
                }
                continue;
            }
            if let Some((t, _, _)) = raycast::clip_box(&bb, eye, delta) {
                let d = search_dist * t;
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
        }
        if let Some((i, d)) = best {
            if d < block_dist {
                // filterHitResult: an entity beyond entity_interaction_range makes the pick a
                // miss (the block behind it is not shown).
                if d <= a.entity_reach {
                    let (id, b) = entities[i];
                    return Some(Target::Entity {
                        id,
                        location: eye + dir * d,
                        // `EntityDimensions.makeBoundingBox`: feet at `pos`, centred
                        // horizontally (the raw box, not the pick-radius-inflated one).
                        pos: DVec3::new(
                            (b.min.x + b.max.x) / 2.0,
                            b.min.y,
                            (b.min.z + b.max.z) / 2.0,
                        ),
                        // Unknown (mod) types are the sentinel the caller filled in.
                        kind: *entity_types.get(i).unwrap_or(&RegistryIndex(u32::MAX)),
                    });
                }
                return None;
            }
        }
        block.map(Target::Block)
    }

    /// `minecraft:entity_type` name of `entities[i]`, when the caller has type data.
    fn entity_type_name<'d>(
        &self,
        data: &'d haste_data::GameData,
        entity_types: &[RegistryIndex],
        i: usize,
    ) -> Option<&'d Identifier> {
        let kind = *entity_types.get(i)?;
        data.registries.name("minecraft:entity_type", kind)
    }
}

/// Vanilla's per-class `Entity.isPickable` (what `EntitySelector.CAN_BE_PICKED` tests in
/// `ProjectileUtil.getEntityHitResult`), as a rule over `minecraft:entity_type` names. The type
/// names are the registry ids from `EntityTypeIds` (ItemEntity's type is `item`, FishingHook's is
/// `fishing_bobber`, the thrown potions are `splash_potion`/`lingering_potion`).
/// Classes returning false: ItemEntity, ExperienceOrb, Marker, AreaEffectCloud, LightningBolt,
/// EyeOfEnder, EvokerFangs and OminousItemSpawner (Entity's default `isPickable`), the Display
/// entities (block/item/text_display, also Entity's default), EnderDragon (explicit false),
/// AbstractArrow's arrow, spectral_arrow and trident (`!isInGround` aside), and the Projectile
/// types outside the redirectable_projectile tag (small_fireball, dragon_fireball, wither_skull,
/// egg, snowball, ender_pearl, splash_potion, lingering_potion, experience_bottle,
/// firework_rocket, llama_spit, fishing_bobber — the tag holds fireball, wind_charge and
/// breeze_wind_charge). Everything else — LivingEntity's `!isRemoved`, Player's `!isSpectator`,
/// the boats, minecarts, item frames, paintings, leash knot, end crystal, TNT, falling blocks,
/// the interaction entity, shulker bullets — is pickable, as is any unknown (mod) type. Arrows
/// stuck in the ground (`AbstractArrow.isInGround`) stay pickable here: the box alone cannot tell.
pub fn is_pickable(t: &Identifier) -> bool {
    if t.namespace() != "minecraft" {
        return true;
    }
    let p = t.path();
    !matches!(
        p,
        "item"
            | "experience_orb"
            | "marker"
            | "area_effect_cloud"
            | "lightning_bolt"
            | "eye_of_ender"
            | "evoker_fangs"
            | "ominous_item_spawner"
            | "block_display"
            | "item_display"
            | "text_display"
            | "ender_dragon"
            | "trident"
            | "small_fireball"
            | "dragon_fireball"
            | "wither_skull"
            | "egg"
            | "snowball"
            | "ender_pearl"
            | "splash_potion"
            | "lingering_potion"
            | "experience_bottle"
            | "firework_rocket"
            | "llama_spit"
            | "fishing_bobber"
    ) && p != "arrow"
        && !p.ends_with("_arrow")
}

/// `Entity.getPickRadius` (Projectile: 1.0 when the type is pickable, else 0.0; every other
/// class 0.0) — the box inflation `ProjectileUtil.getEntityHitResult` searches with.
fn entity_pick_radius(t: &Identifier) -> Option<f64> {
    if t.namespace() == "minecraft"
        && matches!(
            t.path(),
            "fireball" | "wind_charge" | "breeze_wind_charge" | "shulker_bullet"
        )
    {
        Some(1.0)
    } else {
        None
    }
}
