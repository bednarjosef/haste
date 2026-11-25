use super::*;

packet! {
    /// Selected hotbar slot.
    SetCarriedItem = "minecraft:set_carried_item" {
        /// Slot 0..8.
        slot: i16,
    }
}
packet! {
    /// Selects an item inside a bundle.
    BundleItemSelected = "minecraft:bundle_item_selected" {
        /// Inventory slot of the bundle.
        slot: i32 as VarInt,
        /// Slot inside the bundle.
        slot_in_bundle: i32 as VarInt,
    }
}
packet! {
    /// Container button (enchanting, lectern, stonecutter, loom).
    ContainerButtonClick = "minecraft:container_button_click" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Button id.
        button: i32 as VarInt,
    }
}
/// `Hashed Slot`: an item whose component values are CRC32C hashes.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HashedSlot {
    /// `(item id, count, added (type, hash), removed types)`, `None` for empty.
    pub item: Option<HashedItem>,
}

/// The non-empty part of a [`HashedSlot`].
#[derive(Clone, Debug, PartialEq)]
pub struct HashedItem {
    /// Index into `minecraft:item`.
    pub item: i32,
    /// Count.
    pub count: i32,
    /// Added components: type id and CRC32C hash.
    pub added: Vec<(i32, i32)>,
    /// Removed component type ids.
    pub removed: Vec<i32>,
}
impl Wire for HashedSlot {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        if !r.bool()? {
            return Ok(Self { item: None });
        }
        Ok(Self {
            item: Some(HashedItem {
                item: r.var_int()?,
                count: r.var_int()?,
                added: r.list(256, |r| Ok((r.var_int()?, r.i32()?)))?,
                removed: r.read_as::<_, List<VarInt>>()?,
            }),
        })
    }
    fn write(&self, w: &mut Writer) {
        match &self.item {
            None => w.bool(false),
            Some(i) => {
                w.bool(true);
                w.var_int(i.item);
                w.var_int(i.count);
                w.list(&i.added, |w, (t, h)| {
                    w.var_int(*t);
                    w.i32(*h);
                });
                w.write_as::<_, List<VarInt>>(&i.removed);
            }
        }
    }
}
/// A changed slot in `container_click`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangedSlot {
    /// Slot index.
    pub slot: i16,
    /// New content in the client's view.
    pub item: HashedSlot,
}
impl Wire for ChangedSlot {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            slot: r.i16()?,
            item: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.i16(self.slot);
        w.write(&self.item);
    }
}
/// A click in a container.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerClick {
    /// Window id.
    pub window_id: i32,
    /// Last state id received.
    pub state_id: i32,
    /// Clicked slot (-999 outside).
    pub slot: i16,
    /// Button.
    pub button: i8,
    /// Mode.
    pub mode: i32,
    /// Changed slots (at most 128).
    pub changed: Vec<ChangedSlot>,
    /// Cursor item.
    pub carried: HashedSlot,
}
impl Packet for ContainerClick {
    const NAME: &'static str = "minecraft:container_click";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            window_id: r.var_int()?,
            state_id: r.var_int()?,
            slot: r.i16()?,
            button: r.i8()?,
            mode: r.var_int()?,
            changed: r.list(128, ChangedSlot::read)?,
            carried: r.read()?,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.window_id);
        w.var_int(self.state_id);
        w.i16(self.slot);
        w.i8(self.button);
        w.var_int(self.mode);
        w.write(&self.changed);
        w.write(&self.carried);
    }
}
packet! {
    /// Closes a container.
    ContainerClose = "minecraft:container_close" {
        /// Window id.
        window_id: i32 as VarInt,
    }
}
packet! {
    /// Toggles a crafter slot.
    ContainerSlotStateChanged = "minecraft:container_slot_state_changed" {
        /// Slot.
        slot: i32 as VarInt,
        /// Window id.
        window_id: i32 as VarInt,
        /// Enabled.
        enabled: bool,
    }
}
packet! {
    /// Edits or signs a book.
    EditBook = "minecraft:edit_book" {
        /// Hotbar slot.
        slot: i32 as VarInt,
        /// Pages.
        pages: Vec<String>,
        /// Title when signing.
        title: Option<String>,
    }
}
packet! {
    /// Recipe book click.
    PlaceRecipe = "minecraft:place_recipe" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Recipe display id.
        recipe: i32 as VarInt,
        /// Shift held.
        make_all: bool,
    }
}
packet! {
    /// Recipe book settings.
    RecipeBookChangeSettings = "minecraft:recipe_book_change_settings" {
        /// 0 crafting, 1 furnace, 2 blast furnace, 3 smoker.
        book: i32 as VarInt,
        /// Open.
        open: bool,
        /// Filter active.
        filter: bool,
    }
}
packet! {
    /// Marks a recipe as seen.
    RecipeBookSeenRecipe = "minecraft:recipe_book_seen_recipe" {
        /// Recipe display id.
        recipe: i32 as VarInt,
    }
}
packet! {
    /// Anvil rename text.
    RenameItem = "minecraft:rename_item" {
        /// New name.
        name: String,
    }
}
/// Advancement screen tab changes.
#[derive(Clone, Debug, PartialEq)]
pub struct SeenAdvancements {
    /// 0 opened tab, 1 closed screen.
    pub action: i32,
    /// Tab (only for action 0).
    pub tab: Option<Identifier>,
}
impl Packet for SeenAdvancements {
    const NAME: &'static str = "minecraft:seen_advancements";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let action = r.var_int()?;
        let tab = if action == 0 {
            Some(r.identifier()?)
        } else {
            None
        };
        Ok(Self { action, tab })
    }
    fn encode(&self, w: &mut Writer) {
        w.var_int(self.action);
        if let Some(t) = &self.tab {
            w.identifier(t);
        }
    }
}
packet! {
    /// Selects a villager trade.
    SelectTrade = "minecraft:select_trade" {
        /// Trade index.
        slot: i32 as VarInt,
    }
}
packet! {
    /// Beacon effects.
    SetBeacon = "minecraft:set_beacon" {
        /// Primary effect.
        primary: Option<i32> as Opt<VarInt>,
        /// Secondary effect.
        secondary: Option<i32> as Opt<VarInt>,
    }
}
/// An item stack in the "untrusted" encoding where every added component is
/// length-prefixed (as Pumpkin reads `set_creative_mode_slot`; the wiki says
/// plain `Slot`): uncertain, see NOTES.md.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct UntrustedItemStack {
    /// Count (0 = empty, nothing else present).
    pub count: i32,
    /// Index into `minecraft:item`.
    pub item: i32,
    /// Added components: type id and raw data.
    pub added: Vec<(i32, Bytes)>,
    /// Removed component type ids.
    pub removed: Vec<i32>,
}
impl Wire for UntrustedItemStack {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let count = r.var_int()?;
        if count <= 0 {
            return Ok(Self::default());
        }
        let item = r.var_int()?;
        let adds = r.len_prefix(256)?;
        let removes = r.len_prefix(256)?;
        let mut added = Vec::with_capacity(adds.min(r.remaining()));
        for _ in 0..adds {
            added.push((
                r.var_int()?,
                r.byte_array(crate::transport::MAX_PACKET_DATA_SIZE)?,
            ));
        }
        let mut removed = Vec::with_capacity(removes.min(r.remaining()));
        for _ in 0..removes {
            removed.push(r.var_int()?);
        }
        Ok(Self {
            count,
            item,
            added,
            removed,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.count);
        if self.count <= 0 {
            return;
        }
        w.var_int(self.item);
        w.len_prefix(self.added.len());
        w.len_prefix(self.removed.len());
        for (t, d) in &self.added {
            w.var_int(*t);
            w.byte_array(d);
        }
        for t in &self.removed {
            w.var_int(*t);
        }
    }
}
packet! {
    /// Creative inventory slot set.
    SetCreativeModeSlot = "minecraft:set_creative_mode_slot" {
        /// Slot.
        slot: i16,
        /// Item.
        item: UntrustedItemStack,
    }
}
