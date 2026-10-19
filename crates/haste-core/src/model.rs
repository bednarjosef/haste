//! Cube-list entity models, the shape of vanilla's `ModelPart` trees, expressed as data so
//! `haste-entity` can define/animate models and `haste-render` can draw any of them generically.
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::ids::RegistryIndex;
use crate::Identifier;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CubeModel {
    pub texture: Identifier,
    pub tex_width: u32,
    pub tex_height: u32,
    pub root: ModelPart,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelPart {
    pub name: String,
    /// The interned `name` (`parts::intern` at model build time): the frame path matches pose
    /// overrides by this id, never by string. Skipped by serde (nothing serializes models
    /// today); see `parts::PartId` for the deserialization caveat.
    #[serde(skip)]
    pub id: crate::parts::PartId,
    /// Pivot in model units (1/16 block), vanilla's `PartPose` offset.
    pub pivot: Vec3,
    /// Rotation in radians, applied around the pivot (x, y, z order like vanilla).
    pub rotation: Vec3,
    pub visible: bool,
    pub cubes: Vec<Cube>,
    pub children: Vec<ModelPart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cube {
    /// Origin and size in model units, relative to the part pivot.
    pub origin: Vec3,
    pub size: Vec3,
    /// Uniform inflation (vanilla `CubeDeformation`).
    pub inflate: f32,
    /// Top-left texture offset in pixels (vanilla box UV layout).
    pub uv: [u32; 2],
    pub mirror: bool,
}

/// Per-frame pose overrides, keyed by interned part id, so animation state stays in
/// `haste-entity`. Fixed-size, like `BodyTransform`: no per-frame allocation, and a model's
/// setupAnim never pushes more than the capacities (the spider's eight legs plus the head is
/// the largest rotation set; the player's six hideable layers the largest hidden set; the
/// crouching humanoid's head, body, arms and legs the largest offset set).
#[derive(Clone, Debug, Default)]
pub struct PoseOverrides {
    rotations: [Option<(crate::parts::PartId, Vec3)>; POSE_ROTATIONS],
    hidden: [Option<crate::parts::PartId>; POSE_HIDDEN],
    offsets: [Option<(crate::parts::PartId, Vec3)>; POSE_OFFSETS],
}

/// Enough rotation slots for every model's setupAnim (the spider's 8 legs + head, plus
/// headroom for a model that adds a body or wing pair on top).
const POSE_ROTATIONS: usize = 12;
/// Enough hidden slots for the player's six hideable layers, plus headroom.
const POSE_HIDDEN: usize = 8;
/// Enough pivot-offset slots for the crouching humanoid's six parts, plus headroom.
const POSE_OFFSETS: usize = 8;

impl PoseOverrides {
    /// Adds (or replaces) a part's rotation override.
    pub fn set_rotation(&mut self, id: crate::parts::PartId, rot: Vec3) {
        for slot in self.rotations.iter_mut() {
            match slot {
                Some((i, r)) if *i == id => {
                    *r = rot;
                    return;
                }
                Some(_) => continue,
                None => {
                    *slot = Some((id, rot));
                    return;
                }
            }
        }
        debug_assert!(false, "rotation slots exhausted");
    }

    /// The rotation override for `id`, if set.
    pub fn rotation_of(&self, id: crate::parts::PartId) -> Option<Vec3> {
        self.rotations
            .iter()
            .flatten()
            .find(|(i, _)| *i == id)
            .map(|(_, r)| *r)
    }

    /// Adds (or replaces) a part's pivot offset in model units (y down), added to the
    /// part's pivot like vanilla setupAnim's `part.y += ...` on the reset pose.
    pub fn set_offset(&mut self, id: crate::parts::PartId, offset: Vec3) {
        for slot in self.offsets.iter_mut() {
            match slot {
                Some((i, o)) if *i == id => {
                    *o = offset;
                    return;
                }
                Some(_) => continue,
                None => {
                    *slot = Some((id, offset));
                    return;
                }
            }
        }
        debug_assert!(false, "offset slots exhausted");
    }

    /// The pivot offset for `id`, if set.
    pub fn offset_of(&self, id: crate::parts::PartId) -> Option<Vec3> {
        self.offsets
            .iter()
            .flatten()
            .find(|(i, _)| *i == id)
            .map(|(_, o)| *o)
    }

    /// Hides a part for this frame.
    pub fn hide(&mut self, id: crate::parts::PartId) {
        for slot in self.hidden.iter_mut() {
            match slot {
                Some(i) if *i == id => return,
                Some(_) => continue,
                None => {
                    *slot = Some(id);
                    return;
                }
            }
        }
        debug_assert!(false, "hidden slots exhausted");
    }

    /// Whether `id` is hidden this frame.
    pub fn is_hidden(&self, id: crate::parts::PartId) -> bool {
        self.hidden.iter().flatten().any(|i| *i == id)
    }

    /// The set rotations, in push order.
    pub fn rotations(&self) -> impl Iterator<Item = (crate::parts::PartId, Vec3)> + '_ {
        self.rotations.iter().flatten().map(|(i, r)| (*i, *r))
    }

    /// The hidden part ids, in push order.
    pub fn hidden(&self) -> impl Iterator<Item = crate::parts::PartId> + '_ {
        self.hidden.iter().flatten().copied()
    }
}

/// One step of the whole-model transform `LivingEntityRenderer.setupRotations` builds (plus
/// `AvatarRenderer`'s flying/swimming pitch), in the order vanilla applies it (`poseStack` calls
/// compose left to right, so the first op is outermost).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyOp {
    RotateX(f32),
    RotateY(f32),
    RotateZ(f32),
    /// A translation in blocks, in the pre-scale (world-oriented) space.
    Translate(Vec3),
}

/// The whole-model transform applied between the body yaw and the model scale. Fixed-size and
/// `Copy`: no per-frame allocation (`setupRotations` never emits more than four steps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyTransform {
    pub ops: [BodyOp; 4],
    pub len: u8,
    /// `LivingEntityRenderer.setupRotations`: a sleeping entity skips the `180 - bodyRot` yaw and
    /// uses its own rotation sequence instead.
    pub suppress_yaw: bool,
    /// A translation applied before the yaw (sleeping's bed offset), in blocks.
    pub pre_translate: Vec3,
}

impl Default for BodyTransform {
    fn default() -> Self {
        BodyTransform {
            ops: [BodyOp::RotateY(0.0); 4],
            len: 0,
            suppress_yaw: false,
            pre_translate: Vec3::ZERO,
        }
    }
}

impl BodyTransform {
    pub fn push(&mut self, op: BodyOp) {
        if (self.len as usize) < self.ops.len() {
            self.ops[self.len as usize] = op;
            self.len += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &BodyOp> {
        self.ops[..self.len as usize].iter()
    }
}

/// One drawable entity instance handed to the renderer each frame (already interpolated).
#[derive(Clone, Debug)]
pub struct EntityInstance {
    pub id: crate::EntityId,
    pub model: std::sync::Arc<CubeModel>,
    pub pos: glam::DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub scale: f32,
    pub pose: PoseOverrides,
    /// Whole-model rotations (`LivingEntityRenderer.setupRotations`: death tilt, sleeping,
    /// upside-down, player flying/swimming pitch), applied by the renderer between the body yaw
    /// and the model scale.
    pub body: BodyTransform,
    /// Per-axis subclass scale (`EntityRenderer.scale`, e.g. `CreeperRenderer.scale`'s swell
    /// wobble), multiplied into the model scale. `Vec3::ONE` for most entities.
    pub body_scale: Vec3,
    pub skin: Option<Identifier>,
    /// The name tag's text when it should show (`EntityRenderer.extractNameTags`: within 64
    /// blocks and `shouldShowName`), `None` otherwise. For players this is the player name.
    pub nametag: Option<String>,
    /// The scoreboard's below-name line (`Entity.belowNameDisplay`), drawn under the name tag.
    pub below_name: Option<String>,
    /// `EntityRenderer.shadowRadius` for this entity's renderer, already scaled by the age
    /// (`MobRenderer.getShadowRadius` * `ageScale`) or living scale; 0 draws no shadow.
    pub shadow_radius: f32,
    /// `EntityRenderer.shadowStrength` (1.0, or 0.75 for items/experience orbs).
    pub shadow_strength: f32,
    /// `Entity.isInvisible()`: no shadow (`extractShadow` skips it) and no name tag.
    pub invisible: bool,
    /// `Entity.displayFireAnimation()`: on fire (shared flag 0x01), not fire immune, not a
    /// spectator. Draws the flames (`FlameFeatureRenderer`).
    pub display_fire: bool,
    /// `Entity.getBbWidth()` / `getBbHeight()`: the bounding box the flames are sized to.
    pub width: f32,
    pub height: f32,
    /// `Entity.isDiscrete()` (crouching): the name tag hides past 32 blocks and drops the
    /// see-through pass (`LivingEntityRenderer.shouldShowName`, `submitNameTag`).
    pub is_discrete: bool,
    /// The name tag's anchor above the entity's feet: `EntityAttachment.NAME_TAG`'s fallback
    /// `AT_HEIGHT` (`Entity.getBbHeight`, 2.05 for the sniffer).
    pub name_tag_y: f32,
    /// Hurt flash strength in 0..1 (1 just after the hit, easing to 0 over the 10 hurt
    /// ticks; death holds it at 1 for the 20-tick death animation). The renderer mixes
    /// red into the entity's colour for values above 0.
    pub hurt: f32,
    /// Sky and block light at the entity's position, `(sky, block)`, sampled once where the
    /// instance is built for the frame ([`Self::sample_light`]; vanilla
    /// `EntityRenderer.getLightColorCoordinates`). `(15, 0)` (full sky) until sampled, so
    /// instances drawn without sampling render at full sky light.
    pub light: (u8, u8),
    /// Overlay layers drawn on top of the base model with the same transform and pose
    /// (vanilla's `RenderLayer`s: sheep wool and undercoat, horse markings, cat/wolf
    /// collars, the tropical fish pattern). Fixed-size: sheep needs two, everything else
    /// at most one; empty for almost every mob, so no per-frame allocation.
    pub overlays: [Option<EntityOverlay>; 2],
    /// A dropped item entity (`minecraft:item`) drawn as the item itself (spinning and
    /// bobbing); the model above is ignored for these.
    pub item: Option<DroppedItem>,
    /// Set for hanging decorations (item frames, paintings): drawn specially instead of from
    /// `model` (vanilla's `ItemFrameRenderer` / `PaintingRenderer`). `None` for every other
    /// entity, which the renderer draws from its cube model.
    pub decoration: Option<Decoration>,
    /// The item held in each hand, by item registry index: `[right (main), left (off)]`.
    /// `None` is an empty hand (vanilla `ItemInHandLayer` draws nothing for it).
    pub hands: [Option<RegistryIndex>; 2],
    /// Armour and other equipment layers the renderer draws, already resolved to the equipment
    /// asset each item names (vanilla `HumanoidArmorLayer`). Empty for entities with none.
    pub equipment: Vec<EquipmentPiece>,
    /// The head-slot item when it is not armour (vanilla `CustomHeadLayer`: pumpkins, mob and
    /// player heads). `None` when the head is bare or wears a helmet (drawn by `equipment`).
    pub head_item: Option<RegistryIndex>,
}

impl EntityInstance {
    /// Samples this instance's [`Self::light`] from `world`: vanilla
    /// `EntityRenderer.getLightColorCoordinates` probes the block containing the position half
    /// a block up (the entity's middle), sky and block light separately. Called by the client
    /// in the pass that builds the instances for the frame, so the renderer never re-samples.
    pub fn sample_light(&mut self, world: &dyn crate::view::BlockView) {
        let p = crate::BlockPos::new(
            self.pos.x.floor() as i32,
            (self.pos.y + 0.5).floor() as i32,
            self.pos.z.floor() as i32,
        );
        self.light = (world.sky_light(p), world.block_light(p));
    }
}

/// An equipment slot, matching vanilla `EquipmentSlot`'s wire order (the ordinals the
/// `set_equipment` packet uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentSlot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}

impl EquipmentSlot {
    /// The slot for a `set_equipment` wire ordinal, or `None` for an id we do not know.
    pub fn from_wire(id: u8) -> Option<EquipmentSlot> {
        use EquipmentSlot::*;
        Some(match id {
            0 => MainHand,
            1 => OffHand,
            2 => Feet,
            3 => Legs,
            4 => Chest,
            5 => Head,
            6 => Body,
            7 => Saddle,
            _ => return None,
        })
    }

    /// Whether the slot is humanoid armour (vanilla `EquipmentSlot.Type.HUMANOID_ARMOR`).
    pub fn is_armor(self) -> bool {
        matches!(
            self,
            EquipmentSlot::Feet | EquipmentSlot::Legs | EquipmentSlot::Chest | EquipmentSlot::Head
        )
    }
}

/// One worn equipment piece, resolved by `haste-entity` from the entity's equipment and the
/// item's default `equippable` component.
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentPiece {
    pub slot: EquipmentSlot,
    /// The item's registry index (`None` for an empty slot).
    pub item: Option<RegistryIndex>,
    /// The `equippable.asset_id` (`minecraft:iron` -> `equipment/iron.json`), if the item has one.
    pub asset: Option<Identifier>,
    /// The stack's `dyed_color` component (ARGB) when dyed (leather armour); `None` otherwise.
    pub dye: Option<u32>,
}

/// What a dropped item entity carries: which item to draw (index into `minecraft:item`,
/// resolved against the renderer's item names) and the stack size (stacks draw 1-5
/// copies). `age` is the item entity's age in ticks plus the partial tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroppedItem {
    pub item: RegistryIndex,
    pub count: i32,
    pub age: f32,
}

/// One overlay layer: the model (usually a copy of the base shape with its own texture,
/// sometimes the base model itself with one, like vanilla's wolf collar and horse
/// markings) and the tint vanilla multiplies into its vertex colours
/// (`coloredCutoutModelCopyLayerRender`'s colour, `DyeColor.getTextureDiffuseColor` for
/// dyed layers; white when the layer is untinted).
#[derive(Clone, Debug)]
pub struct EntityOverlay {
    pub model: std::sync::Arc<CubeModel>,
    /// Linear RGB multiplier, 0..1 per channel.
    pub tint: [f32; 3],
}

/// A decoration the renderer draws itself rather than from a cube model.
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    ItemFrame(ItemFrameDecoration),
    Painting(PaintingDecoration),
    Lightning(LightningDecoration),
    ExperienceOrb(ExperienceOrbDecoration),
    Minecart(MinecartDecoration),
    ThrownItem(ThrownItemDecoration),
    DragonFireball,
}

/// A minecart's display block (`AbstractMinecart.getDisplayBlockState`), resolved to the block
/// and its properties once, when the synched data or the fuel flag changes; the renderer bakes
/// its quads once per state id.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDisplay {
    /// The block state id (the renderer's quad-cache key, and air's check).
    pub state: crate::BlockStateId,
    /// The state's block.
    pub block: Identifier,
    /// The state's properties, for the blockstate file's variant choice.
    pub props: Vec<(String, String)>,
}

/// The state `AbstractMinecartRenderer.extractRenderState` collects for a minecart
/// (`MinecartRenderState`), old-behavior rendering only (the new-style track packet is a
/// separate task).
#[derive(Clone, Debug, PartialEq)]
pub struct MinecartDecoration {
    /// The display block, `None` for air (nothing is drawn in the cart).
    pub display: Option<std::sync::Arc<BlockDisplay>>,
    /// `AbstractMinecart.getDisplayOffset` (the custom offset when the synched data set one).
    pub display_offset: i32,
    /// `getHurtTime() - partialTicks` (0 when unhurt).
    pub hurt_time: f32,
    /// `getHurtDir()` (1 or -1).
    pub hurt_dir: f32,
    /// `max(getDamage() - partialTicks, 0)`.
    pub damage_time: f32,
    /// The TNT cart's `getFuse() - partialTicks + 1`, or -1 when not primed.
    pub fuse: f32,
    /// `OldMinecartBehavior.getPos`/`getPosOffs`: the rail-snapped position plus the points
    /// 0.3 blocks front and back the cart's rotation is derived from, filled per frame from
    /// the world; `None` off the rails.
    pub rail: Option<(glam::DVec3, glam::DVec3, glam::DVec3)>,
}

/// The state `ExperienceOrbRenderer.submit` collects for an experience orb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExperienceOrbDecoration {
    /// The orb's `Value` metadata, which picks the icon tier (`ExperienceOrb.getIcon`).
    pub value: i32,
    /// The orb's age in ticks (plus the partial tick), driving the colour cycle.
    pub age: f32,
}

/// The state `ThrownItemRenderer` draws: which item (from `ItemSupplier.getItem`, the
/// synched stack or the type's default), at which scale, and whether the renderer lights it
/// full-bright (`ThrownItemRenderer`'s constructor arguments and
/// `ThrownItemRenderState`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrownItemDecoration {
    pub item: RegistryIndex,
    /// The renderer's `scale` constructor argument (1.0 for most, 3.0 for the large fireball,
    /// 0.75 for the small one).
    pub scale: f32,
    /// `ThrownItemRenderer.getBlockLightLevel` returns 15 for the eye of ender and the
    /// fireballs.
    pub full_bright: bool,
}

/// The state `ItemFrameRenderer.extractRenderState` collects for an item frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemFrameDecoration {
    /// The direction the frame faces (`ItemFrame.getDirection`, from the spawn packet's data).
    pub facing: crate::Direction,
    /// `ItemFrame.getRotation` 0..7; the item turns `rotation * 45` degrees.
    pub rotation: u8,
    /// Glow item frames light the frame at least block-light 5 and the item full-bright.
    pub glow: bool,
    /// The framed item (`DATA_ITEM`), by item registry index.
    pub item: Option<crate::RegistryIndex>,
}

/// The state `PaintingRenderer.extractRenderState` collects for a painting.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintingDecoration {
    /// The direction the painting faces (`Painting.getDirection`).
    pub facing: crate::Direction,
    /// The variant's size in blocks (`PaintingVariant.width`/`height`).
    pub width: u8,
    pub height: u8,
    /// The front sprite's texture id, `None` for a variant we do not know (draw the back only).
    pub front: Option<Identifier>,
}

/// The state `LightningBoltRenderer.extractRenderState` collects for a lightning bolt: the
/// seed its geometry is generated from. Vanilla's seed is the entity's own random (not
/// synced, re-rolled per flash), so a seed derived from the entity id stands in — the bolt's
/// shape is procedural either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightningDecoration {
    pub seed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world that answers one sky/block light pair and records where it was asked.
    struct LightWorld {
        sky: u8,
        block: u8,
        asked: std::sync::Mutex<Vec<crate::BlockPos>>,
    }

    impl crate::view::BlockView for LightWorld {
        fn block_state(&self, _pos: crate::BlockPos) -> crate::BlockStateId {
            crate::BlockStateId::AIR
        }
        fn is_chunk_loaded(&self, _pos: crate::ChunkPos) -> bool {
            true
        }
        fn sky_light(&self, pos: crate::BlockPos) -> u8 {
            self.asked.lock().unwrap().push(pos);
            self.sky
        }
        fn block_light(&self, _pos: crate::BlockPos) -> u8 {
            self.block
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn height(&self) -> i32 {
            384
        }
    }

    fn instance(pos: glam::DVec3) -> EntityInstance {
        let model = CubeModel {
            texture: Identifier::minecraft("stone"),
            tex_width: 64,
            tex_height: 64,
            root: ModelPart {
                name: "root".into(),
                id: crate::parts::ROOT,
                pivot: Vec3::ZERO,
                rotation: Vec3::ZERO,
                visible: true,
                cubes: Vec::new(),
                children: Vec::new(),
            },
        };
        EntityInstance {
            id: crate::EntityId(1),
            model: std::sync::Arc::new(model),
            pos,
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            scale: 1.0,
            pose: PoseOverrides::default(),
            body: BodyTransform::default(),
            body_scale: Vec3::ONE,
            skin: None,
            nametag: None,
            below_name: None,
            shadow_radius: 0.5,
            shadow_strength: 1.0,
            invisible: false,
            display_fire: false,
            width: 0.6,
            height: 1.8,
            is_discrete: false,
            name_tag_y: 1.8,
            hurt: 0.0,
            light: (15, 0),
            overlays: Default::default(),
            item: None,
            decoration: None,
            hands: [None, None],
            equipment: Vec::new(),
            head_item: None,
        }
    }

    #[test]
    fn sample_light_carries_the_world_s_light_half_a_block_up() {
        let world = LightWorld {
            sky: 11,
            block: 7,
            asked: std::sync::Mutex::new(Vec::new()),
        };
        let mut e = instance(glam::DVec3::new(3.2, 64.6, -7.8));
        e.sample_light(&world);
        assert_eq!(e.light, (11, 7));
        // `EntityRenderer.getLightColorCoordinates` probes the block containing the position
        // half a block up: y = 64.6 probes y = 65, not the entity's feet block.
        assert_eq!(
            world.asked.lock().unwrap().clone(),
            vec![crate::BlockPos::new(3, 65, -8)]
        );
    }

    #[test]
    fn sample_light_below_half_stays_in_the_feet_block() {
        let world = LightWorld {
            sky: 11,
            block: 7,
            asked: std::sync::Mutex::new(Vec::new()),
        };
        let mut e = instance(glam::DVec3::new(3.2, 64.4, -7.8));
        e.sample_light(&world);
        assert_eq!(
            world.asked.lock().unwrap().clone(),
            vec![crate::BlockPos::new(3, 64, -8)]
        );
    }

    #[test]
    fn unsampled_instances_carry_full_sky_light() {
        // Instances drawn without sampling render at full sky light (the old map's default).
        assert_eq!(instance(glam::DVec3::ZERO).light, (15, 0));
    }
}
