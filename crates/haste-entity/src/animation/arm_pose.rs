//! The humanoid arm poses: `HumanoidModel.ArmPose` (with vanilla's two-handed and
//! off-hand-affecting flags), the pose choice per hand (`AvatarRenderer.getArmPose` for
//! players, `HumanoidMobRenderer.getArmPose` for humanoid mobs) and the arms' rotations
//! each pose sets (`HumanoidModel.poseRightArm`/`poseLeftArm` and their `AnimationUtils`
//! helpers).
//!
//! The item side of the choice (`ItemStack.getUseAnimation`, "is this a crossbow", "is
//! this a spear") is not registry data — vanilla reads it off the item classes — so the
//! client resolves it once per item from the game data into [`ItemUses`] and hands it to
//! the entity store; an empty table leaves every item at the ITEM pose.

use std::f32::consts::{FRAC_PI_2, PI};

use haste_core::ids::RegistryIndex;
use haste_core::math::Vec3;

use crate::store::Entity;

/// `HumanoidModel.ArmPose`: the pose one arm takes, with the enum's constructor flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmPose {
    Empty,
    Item,
    Block,
    BowAndArrow,
    ThrowTrident,
    CrossbowCharge,
    CrossbowHold,
    Spyglass,
    TootHorn,
    Brush,
    Spear,
}

impl ArmPose {
    /// The enum's `twoHanded` flag.
    fn two_handed(self) -> bool {
        matches!(
            self,
            ArmPose::BowAndArrow
                | ArmPose::ThrowTrident
                | ArmPose::CrossbowCharge
                | ArmPose::CrossbowHold
        )
    }

    /// The enum's `affectsOffhandPose` flag.
    fn affects_offhand_pose(self) -> bool {
        self.two_handed()
    }
}

/// `ItemUseAnimation`, by vanilla's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseAnimation {
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Trident,
    Crossbow,
    Spyglass,
    TootHorn,
    Brush,
    Spear,
}

impl UseAnimation {
    /// The arm pose `AvatarRenderer.getArmPose`'s use branch picks for this animation;
    /// `None` falls through to the spear/ITEM rule (eat and drink have no pose of their
    /// own in 26.3).
    fn pose(self) -> Option<ArmPose> {
        Some(match self {
            UseAnimation::Block => ArmPose::Block,
            UseAnimation::Bow => ArmPose::BowAndArrow,
            UseAnimation::Trident => ArmPose::ThrowTrident,
            UseAnimation::Crossbow => ArmPose::CrossbowCharge,
            UseAnimation::Spyglass => ArmPose::Spyglass,
            UseAnimation::TootHorn => ArmPose::TootHorn,
            UseAnimation::Brush => ArmPose::Brush,
            UseAnimation::Spear => ArmPose::Spear,
            _ => return None,
        })
    }
}

/// One item's pose inputs, resolved by the client from the game data (see [`ItemUses`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemUse {
    /// `ItemStack.getUseAnimation` (`None` for items that are not used over time).
    pub animation: UseAnimation,
    /// The item is `minecraft:crossbow` (`CrossbowItem.isCharged` decides `CROSSBOW_HOLD`).
    pub crossbow: bool,
    /// The item is in `ItemTags.SPEARS` (`HumanoidMobRenderer.usesSpearPose`).
    pub spear: bool,
}

impl Default for ItemUse {
    fn default() -> Self {
        ItemUse {
            animation: UseAnimation::None,
            crossbow: false,
            spear: false,
        }
    }
}

/// The per-item pose inputs by `minecraft:item` registry index, built once by the client
/// ([`crate::store::EntityStore::set_item_uses`]). Unknown indices and an absent table
/// answer [`ItemUse::default`], so an item Haste has never seen still gets the ITEM pose.
#[derive(Default)]
pub struct ItemUses {
    items: Vec<ItemUse>,
    /// The `minecraft:charged_projectiles` data-component-type id, for reading a stack's
    /// charge off its component patch (`CrossbowItem.isCharged`).
    charged_projectiles_type: Option<i32>,
}

impl ItemUses {
    /// The table from its parts (the client's builder).
    pub fn new(items: Vec<ItemUse>, charged_projectiles_type: Option<i32>) -> ItemUses {
        ItemUses {
            items,
            charged_projectiles_type,
        }
    }

    /// This item's pose inputs, `ItemUse::default` when unknown.
    pub fn of(&self, item: Option<RegistryIndex>) -> ItemUse {
        item.and_then(|i| self.items.get(i.0 as usize))
            .copied()
            .unwrap_or_default()
    }
}

/// The process-wide table (like `model_dump::library`: built once from the game data the
/// client loads at startup, read per entity per frame without a lock). A reconnect's
/// `set_item_uses` is ignored — the same game data builds the same table.
static ITEM_USES: std::sync::OnceLock<ItemUses> = std::sync::OnceLock::new();

/// Installs the client-built table ([`ItemUses`]); before it arrives (and in tests) the
/// empty default holds every item at the ITEM pose.
pub fn set_item_uses(uses: ItemUses) {
    let _ = ITEM_USES.set(uses);
}

/// The installed table.
pub(crate) fn item_uses() -> &'static ItemUses {
    ITEM_USES.get_or_init(ItemUses::default)
}

/// `AvatarRenderer.getArmPose`/`HumanoidMobRenderer.getArmPose`: the (right, left) arm
/// poses from the held items, the use state and the synced main arm. Players get the full
/// rule set; humanoid mobs only ever hold a spear up (`usesSpearPose`), their held items
/// hang at the EMPTY pose.
pub fn poses(e: &Entity, uses: &ItemUses) -> (ArmPose, ArmPose) {
    let main_right = main_arm_is_right(e);
    // The main hand is wire slot 0, the off hand 1, whichever arms they map onto.
    let main_pose = if e.is_player_type {
        hand_pose(e, uses, 0)
    } else {
        mob_hand_pose(e, uses, 0)
    };
    let off_pose = if e.is_player_type {
        hand_pose(e, uses, 1)
    } else {
        mob_hand_pose(e, uses, 1)
    };
    // A two-handed main-hand pose takes the off hand over (`getArmPose`'s outer rule).
    if main_pose.two_handed() {
        let off_hand = if hand_item(&e.equipment, 1).is_some() {
            ArmPose::Item
        } else {
            ArmPose::Empty
        };
        return if main_right {
            (main_pose, off_hand)
        } else {
            (off_hand, main_pose)
        };
    }
    if main_right {
        (main_pose, off_pose)
    } else {
        (off_pose, main_pose)
    }
}

/// `Avatar.getMainArm` (the synced `DATA_PLAYER_MAIN_HAND` byte at metadata index 15:
/// 0 right, 1 left; right when the server never sent it).
fn main_arm_is_right(e: &Entity) -> bool {
    use haste_core::entity::{MetadataEntry, MetadataValue};
    match e.metadata.get(crate::avatar::DATA_PLAYER_MAIN_HAND) {
        Some(MetadataEntry {
            value: MetadataValue::Byte(b),
            ..
        }) => *b == 0,
        _ => true,
    }
}

/// `AvatarRenderer.getArmPose(avatar, itemInHand, hand)` for one hand.
fn hand_pose(e: &Entity, uses: &ItemUses, hand: u8) -> ArmPose {
    let Some(stack) = hand_item(&e.equipment, hand) else {
        return ArmPose::Empty;
    };
    let item = uses.of(stack.item);
    // A charged crossbow at rest is held up, but not mid-swing.
    if !e.swing.is_swinging() && item.crossbow && is_charged(uses, stack) {
        return ArmPose::CrossbowHold;
    }
    // The use poses while this hand's use runs (`getUsedItemHand` is the off-hand flag).
    if e.metadata.living.using_item && e.metadata.living.use_offhand == (hand == 1) {
        if let Some(pose) = item.animation.pose() {
            return pose;
        }
    }
    if item.spear {
        ArmPose::Spear
    } else {
        ArmPose::Item
    }
}

/// `HumanoidMobRenderer.getArmPose`: a mob holds a spear up, everything else hangs.
fn mob_hand_pose(e: &Entity, uses: &ItemUses, hand: u8) -> ArmPose {
    match hand_item(&e.equipment, hand) {
        Some(stack) if uses.of(stack.item).spear => ArmPose::Spear,
        _ => ArmPose::Empty,
    }
}

/// `getItemInHand` (a wire slot's stack, `None` when the slot is empty or holds air).
fn hand_item(
    equipment: &[(u8, haste_core::entity::ItemStackData)],
    slot: u8,
) -> Option<&haste_core::entity::ItemStackData> {
    equipment
        .iter()
        .find(|(s, _)| *s == slot)
        .and_then(|(_, stack)| (!stack.is_empty()).then_some(stack))
}

/// `CrossbowItem.isCharged`: the stack's component patch adds `charged_projectiles`. Like
/// the armour dye read, only a patch whose first added component it is can be recognised
/// (the bytes after an unknown component cannot be skipped); such a crossbow falls back
/// to the ITEM pose.
fn is_charged(uses: &ItemUses, stack: &haste_core::entity::ItemStackData) -> bool {
    let Some(charged) = uses.charged_projectiles_type else {
        return false;
    };
    let mut r = PatchReader::new(&stack.components_raw);
    let Some(adds) = r.var_int() else {
        return false;
    };
    let _removes = r.var_int();
    adds > 0 && r.var_int() == Some(charged)
}

/// A minimal varint reader over a stack's raw component-patch bytes (the same decoding
/// `haste-data`'s component readers use).
struct PatchReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> PatchReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        PatchReader { bytes, at: 0 }
    }

    fn var_int(&mut self) -> Option<i32> {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            value |= ((byte & 0x7F) as u32) << shift;
            if byte & 0x80 == 0 {
                return Some(value as i32);
            }
            shift += 7;
            if shift >= 32 {
                return None;
            }
        }
    }
}

/// `HumanoidModel.poseRightArm`/`poseLeftArm` applied to the additive arm rotations
/// (`ra`/`la` are the walk-swing + passenger rotations so far, in radians; `head` is the
/// head's yaw relative to the body and its pitch, radians). The two-handed poses write
/// both arms, and the order follows `setupAnim`'s `prioritizeArm`: the use hand's arm
/// first (the off-hand arm otherwise, unless its pose is two-handed), and a pose that
/// affects the off hand suppresses the other arm's own pose.
pub(super) fn apply(
    e: &Entity,
    poses: (ArmPose, ArmPose),
    ra: &mut Vec3,
    la: &mut Vec3,
    head: (f32, f32),
) {
    let (right, left) = poses;
    let main_right = main_arm_is_right(e);
    let prioritize_right = if e.metadata.living.using_item {
        // The use hand's arm (`useItemHand.asArm(mainArm)`).
        if e.metadata.living.use_offhand {
            !main_right
        } else {
            main_right
        }
    } else if left.two_handed() {
        main_right
    } else {
        !main_right
    };
    if prioritize_right {
        pose_arm(right, true, ra, la, head, e);
        if !right.affects_offhand_pose() {
            pose_arm(left, false, ra, la, head, e);
        }
    } else {
        pose_arm(left, false, ra, la, head, e);
        if !left.affects_offhand_pose() {
            pose_arm(right, true, ra, la, head, e);
        }
    }
}

/// One arm's pose (`HumanoidModel.poseRightArm` for `right`, `poseLeftArm` mirrored).
fn pose_arm(
    pose: ArmPose,
    right: bool,
    ra: &mut Vec3,
    la: &mut Vec3,
    head: (f32, f32),
    e: &Entity,
) {
    let (arm, other) = if right { (ra, la) } else { (la, ra) };
    let invert = if right { 1.0 } else { -1.0 };
    let (hy, hx) = head;
    match pose {
        ArmPose::Empty => {
            arm.y = 0.0;
        }
        ArmPose::Item => {
            arm.x = arm.x * 0.5 - PI / 10.0;
            arm.y = 0.0;
        }
        ArmPose::Block => {
            // `HumanoidModel.poseBlockingArm`.
            arm.x = arm.x * 0.5 - 0.9424779 + hx.clamp(-PI * 4.0 / 9.0, 0.43633232);
            arm.y = invert * (PI / 6.0) + hy.clamp(-PI / 6.0, PI / 6.0);
        }
        ArmPose::BowAndArrow => {
            arm.y = -0.1 * invert + hy;
            other.y = 0.1 * invert + hy + 0.4 * invert;
            arm.x = -FRAC_PI_2 + hx;
            other.x = -FRAC_PI_2 + hx;
        }
        ArmPose::ThrowTrident => {
            arm.x = arm.x * 0.5 - PI;
            arm.y = 0.0;
        }
        ArmPose::CrossbowHold => {
            // `AnimationUtils.animateCrossbowHold`.
            arm.y = -0.3 * invert + hy;
            other.y = 0.6 * invert + hy;
            arm.x = -FRAC_PI_2 + hx + 0.1;
            other.x = -1.5 + hx;
        }
        ArmPose::CrossbowCharge => {
            // `AnimationUtils.animateCrossbowCharge` at zero progress (the pull animation
            // over the use's ticks is not modelled yet).
            arm.y = -0.8 * invert;
            arm.x = -0.97079635;
            other.x = arm.x;
            other.y = 0.4 * invert;
        }
        ArmPose::Spyglass => {
            arm.x = (hx
                - 1.9198622
                - if e.metadata.shared.crouching {
                    PI / 12.0
                } else {
                    0.0
                })
            .clamp(-2.4, 3.3);
            arm.y = hy - PI / 12.0 * invert;
        }
        ArmPose::TootHorn => {
            arm.x = hx.clamp(-1.2, 1.2) - 1.4835298;
            arm.y = hy - PI / 6.0 * invert;
        }
        ArmPose::Brush => {
            arm.x = arm.x * 0.5 - PI / 5.0;
            arm.y = 0.0;
        }
        ArmPose::Spear => {
            // `SpearAnimations.thirdPersonHandUse`'s static part (the kinetic-weapon sway
            // while using is not modelled yet).
            arm.y = -0.1 * invert + hy;
            arm.x = -FRAC_PI_2 + hx + 0.8;
            if e.pose == haste_core::entity::Pose::FallFlying || e.swim_amount > 0.0 {
                arm.x -= 0.9599311;
            }
            arm.y = arm.y.to_degrees().clamp(-60.0, 60.0).to_radians();
            arm.x = arm.x.to_degrees().clamp(-120.0, 30.0).to_radians();
        }
    }
}
