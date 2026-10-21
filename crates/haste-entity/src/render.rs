//! The renderer's entity instances: the interpolated draw list `EntityStore::render_instances`
//! builds each frame (vanilla's per-renderer render state extraction). Extracted from
//! `store.rs` to keep that file within the size limit; a pure move.

use haste_core::math::Vec3;
use haste_core::model::{BodyTransform, EntityInstance};

use crate::store::{Entity, EntityStore, HURT_TICKS};

/// The `EntityInstance::item` entry for a tracked entity: the stack of a dropped item entity
/// (count included; the renderer draws 1-5 copies from it), aged by the partial tick.
fn item_of(e: &Entity, partial_tick: f32) -> Option<haste_core::model::DroppedItem> {
    let stack = e.item.as_ref()?;
    let item = stack.item?;
    Some(haste_core::model::DroppedItem {
        item,
        count: stack.count,
        age: e.age as f32 + partial_tick,
    })
}

/// Hurt flash strength for an entity's remaining hurt ticks, eased out (quadratic, like
/// vanilla's `hurtTime`-driven fade: strong on impact, gone by tick 10). A dead entity keeps
/// the flash until the server removes it (`LivingEntityRenderer.extractRenderState`:
/// `hasRedOverlay = hurtTime > 0 || deathTime > 0`, and `deathTime` never resets).
pub fn hurt_strength(e: &Entity) -> f32 {
    if e.dead {
        return 1.0;
    }
    let left = e.hurt_ticks.min(HURT_TICKS) as f32 / HURT_TICKS as f32;
    left * left
}

impl EntityStore {
    /// Interpolated draw list for the renderer (renderer). Players get the humanoid model (classic
    /// layout; skins arrive with renderer's texture work), known mob types the model the recorder
    /// dumped for their vanilla layer, everything else a generic box from its current dimensions;
    /// a dropped item entity draws as the item itself. Each model's `setupAnim` (walk swing, head
    /// aim, attack, pose) is applied by part name; models with no "head" part (the box) get no
    /// overrides.
    pub fn render_instances(&self, partial_tick: f32, out: &mut Vec<EntityInstance>) {
        for slot in self.slots.iter().flatten() {
            // Hanging entities (item frames, paintings) are drawn specially, and the client
            // moved them to the bounding-box centre vanilla's `recalculateBoundingBox` computes
            // (the spawn packet carries the block position, not the render origin). Minecarts
            // and thrown projectiles draw through their own renderers as well
            // (`AbstractMinecartRenderer`, `ThrownItemRenderer`, `DragonFireballRenderer`),
            // but keep the entity's own interpolated position. Arrows too (`ArrowRenderer`),
            // keeping the entity's own position.
            // `DisplayRenderer`: a display draws only once its render state exists, with the
            // shadow its interpolated state casts.
            let display = match slot.display {
                Some(_) => match crate::display::decoration(slot, partial_tick) {
                    Some(d) => Some(d),
                    None => continue,
                },
                None => None,
            };
            let minecart_variant = crate::minecart::variant_for_kind(slot.kind);
            let minecart = minecart_variant
                .map(|v| crate::minecart::decoration(&slot.metadata, v, partial_tick));
            let thrown = crate::thrown_item::variant_for_kind(slot.kind);
            let dragon_fireball = crate::dragon_fireball::is_dragon_fireball(slot.kind);
            let non_hanging = minecart_variant.is_some()
                || thrown.is_some()
                || dragon_fireball
                || crate::arrow::is_arrow(slot.kind);
            let decoration = minecart
                .map(haste_core::model::Decoration::Minecart)
                .or_else(|| {
                    let d = display.as_ref()?.0.clone();
                    Some(haste_core::model::Decoration::Display(d))
                })
                .or_else(|| {
                    thrown
                        .and_then(|t| crate::thrown_item::decoration(&t, &slot.metadata))
                        .map(haste_core::model::Decoration::ThrownItem)
                })
                .or_else(|| {
                    dragon_fireball.then_some(haste_core::model::Decoration::DragonFireball)
                })
                .or_else(|| crate::arrow::decoration(slot, partial_tick))
                .or_else(|| {
                    crate::decoration::build(
                        slot.kind,
                        slot.data,
                        &slot.metadata,
                        &self.painting_variants,
                        slot.id.0,
                        slot.tick_count as f32 + partial_tick,
                        partial_tick,
                        self.default_tnt_state,
                    )
                });
            // The fence knot sits at the box centre of its block (`crate::leash::knot_center`).
            let knot = crate::leash::is_knot(slot.kind);
            let pos = if knot {
                crate::leash::knot_center(slot.prev_pos.lerp(slot.pos, partial_tick as f64))
            } else if non_hanging {
                slot.prev_pos.lerp(slot.pos, partial_tick as f64)
            } else {
                match &decoration {
                    // Hanging entities (item frames, paintings) moved to the wall centre
                    // vanilla's `recalculateBoundingBox` computes; the bolt and the orb keep
                    // the entity's own position; falling blocks and TNT interpolate like
                    // every other entity (vanilla renders them from `Entity.getPosition` at
                    // the partial tick).
                    Some(haste_core::model::Decoration::ItemFrame(_))
                    | Some(haste_core::model::Decoration::Painting(_))
                    | Some(haste_core::model::Decoration::Lightning(_))
                    | Some(haste_core::model::Decoration::ExperienceOrb(_)) => {
                        crate::decoration::render_center(
                            slot.kind,
                            slot.data,
                            slot.pos,
                            &slot.metadata,
                            &self.painting_variants,
                        )
                    }
                    _ => slot.prev_pos.lerp(slot.pos, partial_tick as f64),
                }
            };
            let (model, has_head, overlays) = crate::models::model_for_entity(
                slot,
                minecart_variant.is_some(),
                &self.mob_variants,
                self.world_age,
            );
            let is_passenger = self.riding.vehicle_of(slot.id).is_some();
            // `AbstractBoatModel.setupAnim` for boats (the paddles); the armour stand's pose
            // from its synced data (vanilla `ArmorStandArmorModel.setupAnim` overwrites what
            // `HumanoidModel.setupAnim` set, walk animation and head aim included); the
            // generic `setupAnim` for everything else. A player-typed entity is never posed
            // as a boat (its kind may be anything the server paired it with).
            let is_boat = slot.is_boat && !slot.is_player_type;
            let is_armor_stand = !slot.is_player_type
                && crate::registry::entity_type_name(slot.kind) == Some("minecraft:armor_stand");
            let pose = if is_boat {
                crate::boat::setup_anim(slot, &model, partial_tick)
            } else if is_armor_stand {
                crate::armor_stand::pose(slot, slot.yaw_at(partial_tick))
            } else {
                crate::animation::setup_anim(slot, &model, has_head, partial_tick, is_passenger)
            };
            let texture = model.texture.path();
            // `AbstractBoatRenderer.submit`'s rotations for boats; the armour stand's
            // setupRotations is only the hit wobble (`ArmorStandRenderer.setupRotations` does
            // not call super, so no death tilt or sleeping rotations either); the generic
            // `setupRotations` for everything else.
            let body = if is_boat {
                crate::boat::body_transform(slot, partial_tick)
            } else if knot {
                BodyTransform::default()
            } else if is_armor_stand {
                crate::armor_stand::body_transform(slot, partial_tick)
            } else {
                crate::animation::body_transform(slot, partial_tick, texture.contains("iron_golem"))
            };
            // `CreeperRenderer.scale`'s swell wobble; every other model is uniformly scaled.
            let body_scale = if texture.contains("creeper") {
                crate::animation::creeper_scale(slot, partial_tick)
            } else {
                Vec3::ONE
            };
            // Living models turn with their body yaw; a non-living box follows its yaw
            // (`Entity.getYRot(partialTicks)`).
            // `LeashKnotRenderer.submit` turns the knot by no yaw at all.
            let yaw = if has_head {
                slot.body_yaw_at(partial_tick)
            } else if knot {
                0.0
            } else {
                slot.yaw_at(partial_tick)
            };
            let type_name = crate::registry::entity_type_name(slot.kind);
            let is_baby = !slot.is_player_type && crate::registry::is_baby(&slot.metadata);
            // `LivingEntityRenderer.getShadowRadius`/`MobRenderer.getShadowRadius`: the
            // renderer's base radius times the living/age scale (0.5 for babies). The player's
            // `AvatarRenderer` uses 0.5.
            let base_shadow = type_name
                .map(crate::registry::shadow_radius_for_name)
                .unwrap_or(0.0);
            let shadow_scale = if is_baby { 0.5 } else { 1.0 };
            let shadow_radius = if slot.is_player_type {
                0.5
            } else {
                base_shadow * shadow_scale
            };
            let shadow_strength = type_name
                .map(crate::registry::shadow_strength_for_name)
                .unwrap_or(1.0);
            let (shadow_radius, shadow_strength) =
                display.map_or((shadow_radius, shadow_strength), |(_, r, s)| (r, s));
            let invisible = slot.metadata.shared.invisible;
            // `EntityRenderer.extractNameTags`/`shouldShowName`: players always show their name
            // (`Player.shouldShowName`), mobs only a visible custom name (`LivingEntity`), both
            // hidden while invisible (`isVisibleToPlayer`).
            let nametag = if invisible {
                None
            } else if slot.is_player_type {
                slot.nametag.clone()
            } else if slot.metadata.custom_name_visible() {
                // The plain custom name is computed when the metadata changes (it allocates),
                // never per frame.
                slot.metadata
                    .custom_name_plain()
                    .map(str::to_owned)
                    .or_else(|| slot.nametag.clone())
            } else {
                None
            };
            let name_tag_y = crate::registry::name_tag_offset_for_name(
                type_name.unwrap_or(""),
                slot.dimensions.1,
            );
            // Equipment layers (held items, armour) are only drawn on humanoid models
            // (vanilla adds `ItemInHandLayer`/`HumanoidArmorLayer` per renderer type).
            let humanoid = has_head
                && crate::models::model_has_part(&model.root, haste_core::parts::RIGHT_ARM);
            let (hands, equipment, head_item) = if humanoid {
                crate::local_player::equipment_of(slot, &self.item_components)
            } else {
                ([None, None], Vec::new(), None)
            };
            let mut inst = EntityInstance {
                id: slot.id,
                model,
                pos,
                yaw,
                pitch: slot.pitch_at(partial_tick),
                head_yaw: slot.head_yaw_at(partial_tick),
                // Slime/magma cube size (`AbstractCubeMob.ID_SIZE`); 1.0 for everything else.
                scale: crate::variants::scale_for(slot.kind, &slot.metadata),
                pose,
                body,
                body_scale,
                skin: None,
                nametag,
                below_name: None,
                shadow_radius,
                shadow_strength,
                invisible,
                // `Entity.displayFireAnimation`: `isOnFire()` (the shared flag, never for a
                // fire-immune type) and not a spectator (only players can be one; their game
                // mode is not tracked for remote players yet).
                display_fire: slot.metadata.shared.on_fire && !self.dims.fire_immune(slot.kind),
                width: slot.dimensions.0,
                height: slot.dimensions.1,
                is_discrete: slot.metadata.shared.crouching,
                name_tag_y,
                hurt: hurt_strength(slot),
                // Full sky until the client samples the world where it builds the frame's
                // instances (`EntityInstance::sample_light`).
                light: (15, 0),
                // Overlay layers (sheep wool, horse markings, collars, the fish pattern):
                // drawn with the same transform and pose after the base model.
                overlays,
                item: item_of(slot, partial_tick),
                decoration,
                hands,
                equipment,
                head_item,
            };
            if slot.is_player_type {
                crate::avatar::dress_tracked(
                    &self.player_skin(slot.uuid),
                    slot,
                    &mut inst,
                    partial_tick,
                );
            }
            out.push(inst);
        }
    }
}
