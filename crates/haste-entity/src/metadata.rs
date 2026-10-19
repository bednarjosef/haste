//! Entity metadata ("synched entity data"): raw entries kept per entity, the shared indices
//! interpreted.
//!
//! Shared indices are the base `Entity` class's slots (per minecraft.wiki, "Entity format" /
//! metadata tables): index 0 is the shared flags byte, index 6 the `Pose` (modern versions;
//! older layouts differ). Anything a specific entity class adds is stored raw and interpreted
//! later by whoever needs it.
use haste_core::entity::{MetadataEntry, MetadataValue, Pose};
use haste_core::Component;

/// The shared flags byte (metadata index 0).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SharedFlags {
    pub on_fire: bool,
    pub crouching: bool,
    pub riding: bool,
    pub invisible: bool,
    pub glowing: bool,
    pub elytra_flying: bool,
}

impl SharedFlags {
    fn from_byte(b: i8) -> SharedFlags {
        let b = b as u8;
        SharedFlags {
            on_fire: b & 0x01 != 0,
            crouching: b & 0x02 != 0,
            riding: b & 0x04 != 0,
            // Vanilla `Entity.FLAG_INVISIBLE = 5` (bit 0x20; bit 0x10 is FLAG_SWIMMING,
            // `Entity.getSharedFlag`).
            invisible: b & 0x20 != 0,
            glowing: b & 0x40 != 0,
            elytra_flying: b & 0x80 != 0,
        }
    }
}

/// `Mob.DATA_MOB_FLAGS_ID` (index 15: `Entity`'s 8 slots 0..7, `LivingEntity`'s 7 slots 8..14,
/// then `Mob`'s own first `defineId`; `Mob.java:101`). Only the bits the client reads are kept:
/// `MOB_FLAG_LEFTHANDED = 2` and `MOB_FLAG_AGGRESSIVE = 4` (`Mob.isLeftHanded`/`isAggressive`,
/// `Mob.java:1326-1332`).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct MobFlags {
    pub aggressive: bool,
    pub left_handed: bool,
}

impl MobFlags {
    fn from_byte(b: i8) -> MobFlags {
        let b = b as u8;
        MobFlags {
            aggressive: b & 0x04 != 0,
            left_handed: b & 0x02 != 0,
        }
    }
}

/// `LivingEntity.DATA_LIVING_ENTITY_FLAGS` (index 8, the first of `LivingEntity`'s slots
/// after `Entity`'s eight; a byte). Only the bits the client's use tracking reads:
/// `setLivingEntityFlag(1, ...)` starts a use, `2` marks the off hand as the used one.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct LivingFlags {
    /// `isUsingItem` (bit 1).
    pub using_item: bool,
    /// `getUsedItemHand` answers the off hand for bit 2.
    pub use_offhand: bool,
}

impl LivingFlags {
    fn from_byte(b: i8) -> LivingFlags {
        let b = b as u8;
        LivingFlags {
            using_item: b & 0x01 != 0,
            use_offhand: b & 0x02 != 0,
        }
    }
}

/// Per-entity metadata: interpreted shared fields plus every raw entry, indexed.
pub struct Metadata {
    pub shared: SharedFlags,
    pub pose: Option<Pose>,
    /// The `Mob` flags byte (index 15): aggressive / left-handed.
    pub mob: MobFlags,
    /// The `LivingEntity` flags byte (index 8): an item use in progress.
    pub living: LivingFlags,
    /// `Entity.DATA_CUSTOM_NAME` (index 2): the entity's custom name, for the upside-down check.
    pub custom_name: Option<Component>,
    /// The custom name flattened to plain text, computed when the metadata changes (name tags
    /// and the upside-down check read it every frame without allocating).
    pub custom_name_plain: Option<String>,
    /// `Creeper.DATA_SWELL_DIR` (index 16, an int: `AgeableMob` defines none for a creeper, so
    /// `Mob`'s 15 is followed by `Creeper`'s first accessor). The client advances its own swell
    /// count from this (`Creeper.tick`).
    pub creeper_swell_dir: i32,
    /// `Creeper.DATA_IS_IGNITED` (index 18, a bool).
    pub creeper_ignited: bool,
    /// `AbstractVillager.DATA_UNHAPPY_COUNTER` (index 18, an int: `AgeableMob`'s two accessors
    /// sit at 16/17, then `AbstractVillager`'s). `> 0` means the villager shakes its head.
    pub villager_unhappy: bool,
    /// `PrimedTnt.DATA_FUSE_ID` (index 8, an int: `PrimedTnt` extends `Entity`, not
    /// `LivingEntity`, so its first slot follows the eight shared ones). The default is
    /// vanilla's `DEFAULT_FUSE_TIME`; the client ticks it down itself (`PrimedTnt.tick`
    /// runs client-side too) between the server's syncs.
    pub tnt_fuse: i32,
    /// `PrimedTnt.DATA_BLOCK_STATE_ID` (index 9): the block the TNT renders as; `None` when
    /// the server sent none (the TNT block's default state).
    pub tnt_block_state: Option<haste_core::BlockStateId>,
    /// Minecart state, interpreted in `crate::minecart` (only minecart entities read it).
    /// `VehicleEntity.DATA_ID_HURT` (index 8, an int: `Entity`'s eight slots come first),
    /// ticked down client-side by `AbstractMinecart.tick`.
    pub minecart_hurt_time: i32,
    /// `VehicleEntity.DATA_ID_HURTDIR` (index 9, an int; 1 or -1).
    pub minecart_hurt_dir: i32,
    /// `VehicleEntity.DATA_ID_DAMAGE` (index 10, a float), ticked down client-side.
    pub minecart_damage: f32,
    /// `AbstractMinecart.DATA_ID_CUSTOM_DISPLAY_BLOCK` (index 11, an optional block state):
    /// the block a minecart shows instead of its type's default, `None` when unset.
    pub minecart_custom_display: Option<haste_core::BlockStateId>,
    /// `MinecartFurnace.DATA_ID_FUEL` (index 13, a bool): a lit furnace cart.
    pub minecart_fuel: bool,
    /// The TNT cart's fuse (`MinecartTNT.fuse`, started by entity event 70 at 80 ticks and
    /// ticked down client-side; `None` when not primed, vanilla's -1).
    pub minecart_fuse: Option<i32>,
    /// All entries (including the shared ones), kept so later consumers can read the rest.
    entries: std::vec::Vec<MetadataEntry>,
}

impl Metadata {
    pub fn apply(&mut self, entries: &[MetadataEntry]) {
        for e in entries {
            if let Some(slot) = self.entries.iter_mut().find(|x| x.index == e.index) {
                *slot = e.clone();
            } else {
                self.entries.push(e.clone());
            }
        }
        for e in entries {
            match (e.index, &e.value) {
                (0, MetadataValue::Byte(b)) => self.shared = SharedFlags::from_byte(*b),
                (2, MetadataValue::OptionalComponent(c)) => {
                    self.custom_name = c.clone();
                    self.custom_name_plain = c.as_ref().map(|c| c.to_plain(&|_| None));
                }
                (2, MetadataValue::Component(c)) => {
                    self.custom_name = Some(c.clone());
                    self.custom_name_plain = Some(c.to_plain(&|_| None));
                }
                (6, MetadataValue::Pose(p)) => self.pose = Some(*p),
                // Index 8 is `LivingEntity`'s flags byte; `ItemEntity`'s stack sits on the
                // same index as an ItemStack, so the wire type tells them apart. `PrimedTnt`
                // (not a living entity) puts its fuse int on the same slot, and a minecart
                // its hurt time (`VehicleEntity.DATA_ID_HURT`) — the entity's type decides
                // which field is read.
                (8, MetadataValue::Byte(b)) => self.living = LivingFlags::from_byte(*b),
                (8, MetadataValue::VarInt(v)) => {
                    self.tnt_fuse = *v;
                    self.minecart_hurt_time = *v;
                }
                // Index 9: `PrimedTnt.DATA_BLOCK_STATE_ID` or `VehicleEntity`'s hurt
                // direction, by wire type.
                (9, MetadataValue::BlockState(s)) => self.tnt_block_state = Some(*s),
                (9, MetadataValue::VarInt(v)) => self.minecart_hurt_dir = *v,
                (15, MetadataValue::Byte(b)) => self.mob = MobFlags::from_byte(*b),
                // Index 16 is `Creeper.DATA_SWELL_DIR` (int); `AgeableMob`'s baby flag at 16 is a
                // bool, so the wire type keeps the two apart.
                (16, MetadataValue::VarInt(v)) => self.creeper_swell_dir = *v,
                // Index 18 is `Creeper.DATA_IS_IGNITED` (bool) or `AbstractVillager`'s unhappy
                // counter (int).
                (18, MetadataValue::Boolean(b)) => self.creeper_ignited = *b,
                (18, MetadataValue::VarInt(v)) => self.villager_unhappy = *v > 0,
                // `VehicleEntity`'s damage slot (10) and `AbstractMinecart`'s (11) and
                // `MinecartFurnace`'s (13): interpreted in `crate::minecart`.
                (10, MetadataValue::Float(v)) => self.minecart_damage = *v,
                (11, MetadataValue::OptionalBlockState(s)) => self.minecart_custom_display = *s,
                (13, MetadataValue::Boolean(b)) => self.minecart_fuel = *b,
                _ => {}
            }
        }
    }

    /// The stored entry for an index, if any.
    pub fn get(&self, index: u8) -> Option<&MetadataEntry> {
        self.entries.iter().find(|e| e.index == index)
    }

    /// `Entity.DATA_CUSTOM_NAME` (index 2): the custom name in plain text, if set. Computed
    /// when the metadata changes; per-frame readers take it without allocating.
    pub fn custom_name_plain(&self) -> Option<&str> {
        match self.get(2).map(|e| &e.value) {
            Some(MetadataValue::OptionalComponent(Some(_))) | Some(MetadataValue::Component(_)) => {
                self.custom_name_plain.as_deref()
            }
            _ => None,
        }
    }

    /// `Entity.DATA_CUSTOM_NAME_VISIBLE` (index 3): whether the custom name always shows
    /// (`Entity.shouldShowName`).
    pub fn custom_name_visible(&self) -> bool {
        matches!(
            self.get(3).map(|e| &e.value),
            Some(MetadataValue::Boolean(true))
        )
    }

    pub fn iter(&self) -> impl Iterator<Item = &MetadataEntry> {
        self.entries.iter()
    }
}

impl Default for Metadata {
    /// Everything starts cleared except the TNT fuse, whose synched default is vanilla's
    /// `PrimedTnt.DEFAULT_FUSE_TIME` (80): a TNT that spawns without entity data counts
    /// down from there.
    fn default() -> Metadata {
        Metadata {
            shared: SharedFlags::default(),
            pose: None,
            mob: MobFlags::default(),
            living: LivingFlags::default(),
            custom_name: None,
            custom_name_plain: None,
            creeper_swell_dir: 0,
            creeper_ignited: false,
            villager_unhappy: false,
            tnt_fuse: DEFAULT_TNT_FUSE,
            tnt_block_state: None,
            minecart_hurt_time: 0,
            minecart_hurt_dir: 0,
            minecart_damage: 0.0,
            minecart_custom_display: None,
            minecart_fuel: false,
            minecart_fuse: None,
            entries: Vec::new(),
        }
    }
}

/// `PrimedTnt.DEFAULT_FUSE_TIME`.
pub const DEFAULT_TNT_FUSE: i32 = 80;
