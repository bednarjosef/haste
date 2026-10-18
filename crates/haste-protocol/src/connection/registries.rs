//! What the server sent about the player and the world during login and
//! configuration: the play-state `login` result and the synced registries.

use haste_core::game::{DimensionInfo, GameMode};
use haste_core::{EntityId, Identifier};

use crate::nbt::NbtCompound;
use crate::packets::configuration::clientbound::{
    KnownPack, RegistryEntry, RegistryTags, ReportDetail, ServerLink,
};
use crate::packets::play;
use crate::wire::{GameProfile, Reader};

/// What the play-state `login` told us, plus resolved dimension info.
#[derive(Clone, Debug)]
pub struct Login {
    /// The player's entity id.
    pub entity_id: EntityId,
    /// Dimension (min y, height, skylight from the synced dimension type).
    pub dimension: DimensionInfo,
    /// Game mode.
    pub game_mode: GameMode,
    /// Server view distance.
    pub view_distance: i32,
    /// Server simulation distance.
    pub simulation_distance: i32,
    /// Hardcore.
    pub hardcore: bool,
    /// The profile from `login_finished`.
    pub profile: GameProfile,
    /// The full packet.
    pub packet: play::clientbound::Login,
}

/// Registries, tags and feature flags from the configuration state. Entry
/// data is kept as raw network NBT; entry order defines numeric ids.
#[derive(Clone, Debug, Default)]
pub struct SyncedRegistries {
    registries: Vec<(Identifier, Vec<RegistryEntry>)>,
    /// Tags per registry (`update_tags`), latest packet wins per registry.
    pub tags: Vec<RegistryTags>,
    /// Enabled feature flags.
    pub features: Vec<Identifier>,
    /// Packs the server announced in `select_known_packs`.
    pub server_packs: Vec<KnownPack>,
    /// Pause-menu links from `server_links` (configuration state; later
    /// updates arrive as play packets).
    pub server_links: Vec<ServerLink>,
    /// Crash report lines from `custom_report_details`.
    pub report_details: Vec<ReportDetail>,
}

impl SyncedRegistries {
    /// Stores one `registry_data` packet (replacing an earlier one for the
    /// same registry).
    pub fn insert(&mut self, registry: Identifier, entries: Vec<RegistryEntry>) {
        if let Some(r) = self.registries.iter_mut().find(|(id, _)| *id == registry) {
            r.1 = entries;
        } else {
            self.registries.push((registry, entries));
        }
    }
    /// Entries of a registry in id order.
    pub fn entries(&self, registry: &Identifier) -> Option<&[RegistryEntry]> {
        self.registries
            .iter()
            .find(|(id, _)| id == registry)
            .map(|(_, e)| e.as_slice())
    }
    /// Registry names received.
    pub fn names(&self) -> impl Iterator<Item = &Identifier> {
        self.registries.iter().map(|(id, _)| id)
    }
    /// Numeric id of an entry.
    pub fn index_of(&self, registry: &Identifier, entry: &Identifier) -> Option<usize> {
        self.entries(registry)?.iter().position(|e| &e.id == entry)
    }
    /// Parsed NBT of an entry (`None` if absent or not a compound).
    pub fn entry_nbt(&self, registry: &Identifier, index: usize) -> Option<NbtCompound> {
        let data = self.entries(registry)?.get(index)?.data.as_ref()?;
        Reader::new(data).nbt_compound().ok().flatten()
    }
    /// Dimension info for a `dimension_type` index and dimension name, from
    /// the synced `minecraft:dimension_type` NBT (`min_y`, `height`,
    /// `has_skylight`).
    pub fn dimension_info(
        &self,
        dimension_type: i32,
        dimension: &Identifier,
    ) -> Option<DimensionInfo> {
        let reg = Identifier::minecraft("dimension_type");
        let idx = usize::try_from(dimension_type).ok()?;
        let entry = self.entries(&reg)?.get(idx)?;
        let nbt = self.entry_nbt(&reg, idx)?;
        Some(DimensionInfo {
            name: dimension.clone(),
            dimension_type: entry.id.clone(),
            min_y: nbt.get_int("min_y")?,
            height: nbt.get_int("height")?,
            has_skylight: nbt
                .get_bool("has_skylight")
                .or_else(|| nbt.get_byte("has_skylight").map(|b| b != 0))?,
        })
    }
    pub(super) fn apply_tags(&mut self, tags: Vec<RegistryTags>) {
        for t in tags {
            if let Some(old) = self.tags.iter_mut().find(|o| o.registry == t.registry) {
                *old = t;
            } else {
                self.tags.push(t);
            }
        }
    }
}
