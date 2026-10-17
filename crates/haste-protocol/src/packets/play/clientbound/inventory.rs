use super::*;

packet! {
    /// Replaces a container's contents.
    ContainerSetContent = "minecraft:container_set_content" {
        /// Window id (0 = player inventory).
        window_id: i32 as VarInt,
        /// State id.
        state_id: i32 as VarInt,
        /// Slots.
        slots: Vec<ItemStackData>,
        /// Item on the cursor.
        carried: ItemStackData,
    }
}
packet! {
    /// Sets one container slot.
    ContainerSetSlot = "minecraft:container_set_slot" {
        /// Window id.
        window_id: i32 as VarInt,
        /// State id.
        state_id: i32 as VarInt,
        /// Slot index.
        slot: i16,
        /// Item.
        item: ItemStackData,
    }
}
packet! {
    /// Recipe book additions (body kept raw).
    RecipeBookAdd = "minecraft:recipe_book_add" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Recipe book removals (body kept raw).
    RecipeBookRemove = "minecraft:recipe_book_remove" {
        /// Raw body.
        body: Bytes as Rest,
    }
}

packet! {
    /// Recipe book settings (body kept raw).
    RecipeBookSettings = "minecraft:recipe_book_settings" {
        /// Raw body.
        body: Bytes as Rest,
    }
}
packet! {
    /// Recipe property sets and stonecutter recipes (body kept raw).
    UpdateRecipes = "minecraft:update_recipes" {
        /// Raw body.
        body: Bytes as Rest,
    }
}
packet! {
    /// Forcibly closes a container.
    ContainerClose = "minecraft:container_close" {
        /// Window id.
        window_id: i32 as VarInt,
    }
}

packet! {
    /// A container property (furnace progress, enchantment seeds, ...).
    ContainerSetData = "minecraft:container_set_data" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Property index.
        property: i16,
        /// Value.
        value: i16,
    }
}
packet! {
    /// Item cooldown.
    Cooldown = "minecraft:cooldown" {
        /// Item id or `use_cooldown` group.
        group: Identifier,
        /// Ticks, 0 clears.
        ticks: i32 as VarInt,
    }
}
/// A trade cost item (`Trade Item`): item, count and an exact component
/// list kept raw (Prefixed Array of type + data).
#[derive(Clone, Debug, PartialEq)]
pub struct TradeItem {
    /// Index into `minecraft:item`.
    pub item: i32,
    /// Count.
    pub count: i32,
    /// Raw component list including its length prefix.
    pub components_raw: Bytes,
}
impl Wire for TradeItem {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let item = r.var_int()?;
        let count = r.var_int()?;
        let start = r.peek_rest();
        crate::wire::slot::skip_exact_components(r)?;
        let used = start.len() - r.remaining();
        Ok(Self {
            item,
            count,
            components_raw: r.to_bytes(&start[..used]),
        })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.item);
        w.var_int(self.count);
        if self.components_raw.is_empty() {
            w.var_int(0);
        } else {
            w.bytes(&self.components_raw);
        }
    }
}

/// One villager trade.
#[derive(Clone, Debug, PartialEq)]
pub struct MerchantOffer {
    /// First input.
    pub input1: TradeItem,
    /// Output.
    pub output: ItemStackData,
    /// Second input.
    pub input2: Option<TradeItem>,
    /// Disabled.
    pub disabled: bool,
    /// Uses so far.
    pub uses: i32,
    /// Max uses.
    pub max_uses: i32,
    /// Villager XP per trade.
    pub xp: i32,
    /// Special price.
    pub special_price: i32,
    /// Price multiplier.
    pub price_multiplier: f32,
    /// Demand.
    pub demand: i32,
}
impl Wire for MerchantOffer {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            input1: r.read()?,
            output: r.read()?,
            input2: r.read()?,
            disabled: r.bool()?,
            uses: r.i32()?,
            max_uses: r.i32()?,
            xp: r.i32()?,
            special_price: r.i32()?,
            price_multiplier: r.f32()?,
            demand: r.i32()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.input1);
        w.write(&self.output);
        w.write(&self.input2);
        w.bool(self.disabled);
        w.i32(self.uses);
        w.i32(self.max_uses);
        w.i32(self.xp);
        w.i32(self.special_price);
        w.f32(self.price_multiplier);
        w.i32(self.demand);
    }
}

packet! {
    /// Villager trades.
    MerchantOffers = "minecraft:merchant_offers" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Trades.
        offers: Vec<MerchantOffer>,
        /// Villager level.
        level: i32 as VarInt,
        /// Villager experience.
        experience: i32 as VarInt,
        /// Regular villager (not wandering trader).
        regular_villager: bool,
        /// Can restock.
        can_restock: bool,
    }
}
packet! {
    /// Opens a written book.
    OpenBook = "minecraft:open_book" {
        /// 0 main hand, 1 off hand.
        hand: i32 as VarInt,
    }
}
packet! {
    /// Opens a container screen.
    OpenScreen = "minecraft:open_screen" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Index into `minecraft:menu`.
        kind: i32 as VarInt,
        /// Title.
        title: Component,
    }
}
packet! {
    /// Opens the sign editor.
    OpenSignEditor = "minecraft:open_sign_editor" {
        /// Sign.
        pos: BlockPos,
        /// 0 back, 1 front.
        text_slot: i32 as VarInt,
    }
}
packet! {
    /// Ghost recipe in a crafting grid. The `Recipe Display` is kept raw.
    PlaceGhostRecipe = "minecraft:place_ghost_recipe" {
        /// Window id.
        window_id: i32 as VarInt,
        /// Recipe display, raw.
        display: Bytes as Rest,
    }
}
packet! {
    /// Switches the advancements tab.
    SelectAdvancementsTab = "minecraft:select_advancements_tab" {
        /// Tab root advancement.
        tab: Option<Identifier>,
    }
}
packet! {
    /// Item on the cursor.
    SetCursorItem = "minecraft:set_cursor_item" {
        /// Item.
        item: ItemStackData,
    }
}
packet! {
    /// Sets a player inventory slot directly.
    SetPlayerInventory = "minecraft:set_player_inventory" {
        /// Inventory slot index.
        slot: i32 as VarInt,
        /// Item.
        item: ItemStackData,
    }
}
/// Advancement display data.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvancementDisplay {
    /// Title.
    pub title: Component,
    /// Description.
    pub description: Component,
    /// Icon (an item stack template on the wire).
    pub icon: ItemStackData,
    /// 0 task, 1 challenge, 2 goal.
    pub frame: i32,
    /// 0x01 background, 0x02 show toast, 0x04 hidden.
    pub flags: i32,
    /// Background texture (present when flags & 1).
    pub background: Option<Identifier>,
}
impl Wire for AdvancementDisplay {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let title = r.component()?;
        let description = r.component()?;
        let icon = r.read_as::<_, crate::wire::slot::ItemStackTemplate>()?;
        let frame = r.var_int()?;
        let flags = r.i32()?;
        let background = if flags & 1 != 0 {
            Some(r.identifier()?)
        } else {
            None
        };
        Ok(Self {
            title,
            description,
            icon,
            frame,
            flags,
            background,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.component(&self.title);
        w.component(&self.description);
        w.write_as::<_, crate::wire::slot::ItemStackTemplate>(&self.icon);
        w.var_int(self.frame);
        w.i32(self.flags);
        if let Some(b) = &self.background {
            w.identifier(b);
        }
    }
}

/// One advancement.
///
/// 26.3 layout as Pumpkin writes it: the tree position (x, y) follows the
/// telemetry flag for every advancement instead of being inside the display
/// (the wiki still shows it in the display): uncertain, see NOTES.md.
#[derive(Clone, Debug, PartialEq)]
pub struct Advancement {
    /// Parent.
    pub parent: Option<Identifier>,
    /// Display.
    pub display: Option<AdvancementDisplay>,
    /// Requirements (AND of ORs of criteria).
    pub requirements: Vec<Vec<String>>,
    /// Sends telemetry.
    pub telemetry: bool,
    /// Tree x.
    pub x: f32,
    /// Tree y.
    pub y: f32,
}
impl Wire for Advancement {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            parent: r.read()?,
            display: r.read()?,
            requirements: r.read()?,
            telemetry: r.bool()?,
            x: r.f32()?,
            y: r.f32()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.write(&self.parent);
        w.write(&self.display);
        w.write(&self.requirements);
        w.bool(self.telemetry);
        w.f32(self.x);
        w.f32(self.y);
    }
}

/// Progress of one criterion.
///
/// The criterion is the plain map key from the advancement's JSON
/// (`AdvancementProgress.STREAM_CODEC` pairs a string with a criterion), so it is carried as
/// the exact string the server wrote. Names made it an `Identifier`, which wrote a
/// `minecraft:` namespace back into the packet: the UI forwards this packet into Java in its
/// wire form, and vanilla's `ClientAdvancements` then drops the progress because the
/// criterion names no longer match the advancement's criteria.
#[derive(Clone, Debug, PartialEq)]
pub struct CriterionProgress {
    /// Criterion name (plain, not namespaced).
    pub criterion: String,
    /// When achieved (ms since epoch).
    pub achieved_at: Option<i64>,
}
impl Wire for CriterionProgress {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            criterion: r.string()?,
            achieved_at: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.criterion);
        w.write(&self.achieved_at);
    }
}

/// Advancement entry in `update_advancements`.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvancementEntry {
    /// Id.
    pub id: Identifier,
    /// Advancement.
    pub advancement: Advancement,
}
impl Wire for AdvancementEntry {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.identifier()?,
            advancement: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.id);
        w.write(&self.advancement);
    }
}

/// Progress entry in `update_advancements`.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvancementProgress {
    /// Advancement id.
    pub id: Identifier,
    /// Criteria.
    pub criteria: Vec<CriterionProgress>,
}
impl Wire for AdvancementProgress {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.identifier()?,
            criteria: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.id);
        w.write(&self.criteria);
    }
}

packet! {
    /// Advancements.
    UpdateAdvancements = "minecraft:update_advancements" {
        /// Reset first.
        reset: bool,
        /// Added advancements.
        added: Vec<AdvancementEntry>,
        /// Removed advancement ids.
        removed: Vec<Identifier>,
        /// Progress.
        progress: Vec<AdvancementProgress>,
        /// Show the advancements toast/screen.
        show_advancements: bool,
    }
}
