use super::*;

packet! {
    /// The command graph (body kept raw for now).
    Commands = "minecraft:commands" {
        /// Raw body.
        body: Bytes as Rest,
    }
}
packet! {
    /// MOTD and icon.
    ServerData = "minecraft:server_data" {
        /// MOTD.
        motd: Component,
        /// PNG icon.
        icon: Option<Bytes> as Opt<ByteArray>,
    }
}
packet! {
    /// Tab list header and footer.
    TabList = "minecraft:tab_list" {
        /// Header.
        header: Component,
        /// Footer.
        footer: Component,
    }
}
/// The action of a `boss_event`.
#[derive(Clone, Debug, PartialEq)]
pub enum BossEventAction {
    /// 0: add.
    Add {
        /// Title.
        title: Component,
        /// Health 0..1.
        health: f32,
        /// Colour id.
        color: i32,
        /// Division id.
        division: i32,
        /// 0x01 darken sky, 0x02 dragon bar, 0x04 fog.
        flags: u8,
    },
    /// 1: remove.
    Remove,
    /// 2: health.
    UpdateHealth(f32),
    /// 3: title.
    UpdateTitle(Component),
    /// 4: style.
    UpdateStyle {
        /// Colour id.
        color: i32,
        /// Division id.
        division: i32,
    },
    /// 5: flags.
    UpdateFlags(u8),
}

/// Boss bar update.
#[derive(Clone, Debug, PartialEq)]
pub struct BossEvent {
    /// Bar id.
    pub id: Uuid,
    /// What changes.
    pub action: BossEventAction,
}
impl Packet for BossEvent {
    const NAME: &'static str = "minecraft:boss_event";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let id = r.uuid()?;
        let action = match r.var_int()? {
            0 => BossEventAction::Add {
                title: r.component()?,
                health: r.f32()?,
                color: r.var_int()?,
                division: r.var_int()?,
                flags: r.u8()?,
            },
            1 => BossEventAction::Remove,
            2 => BossEventAction::UpdateHealth(r.f32()?),
            3 => BossEventAction::UpdateTitle(r.component()?),
            4 => BossEventAction::UpdateStyle {
                color: r.var_int()?,
                division: r.var_int()?,
            },
            5 => BossEventAction::UpdateFlags(r.u8()?),
            a => return Err(DecodeError::Invalid(format!("boss event action {a}"))),
        };
        Ok(Self { id, action })
    }
    fn encode(&self, w: &mut Writer) {
        w.uuid(&self.id);
        match &self.action {
            BossEventAction::Add {
                title,
                health,
                color,
                division,
                flags,
            } => {
                w.var_int(0);
                w.component(title);
                w.f32(*health);
                w.var_int(*color);
                w.var_int(*division);
                w.u8(*flags);
            }
            BossEventAction::Remove => w.var_int(1),
            BossEventAction::UpdateHealth(h) => {
                w.var_int(2);
                w.f32(*h);
            }
            BossEventAction::UpdateTitle(t) => {
                w.var_int(3);
                w.component(t);
            }
            BossEventAction::UpdateStyle { color, division } => {
                w.var_int(4);
                w.var_int(*color);
                w.var_int(*division);
            }
            BossEventAction::UpdateFlags(f) => {
                w.var_int(5);
                w.u8(*f);
            }
        }
    }
}
packet! {
    /// Clears the title.
    ClearTitles = "minecraft:clear_titles" {
        /// Also reset the title times.
        reset: bool,
    }
}
/// One tab-completion match.
#[derive(Clone, Debug, PartialEq)]
pub struct SuggestionMatch {
    /// Text to insert.
    pub text: String,
    /// Tooltip.
    pub tooltip: Option<Component>,
}
impl Wire for SuggestionMatch {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            text: r.string()?,
            tooltip: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.text);
        w.write(&self.tooltip);
    }
}

packet! {
    /// Command tab-completion answer.
    CommandSuggestions = "minecraft:command_suggestions" {
        /// Transaction id.
        id: i32 as VarInt,
        /// Start of the replaced text.
        start: i32 as VarInt,
        /// Length of the replaced text.
        length: i32 as VarInt,
        /// Matches.
        matches: Vec<SuggestionMatch>,
    }
}
packet! {
    /// Removes a score.
    ResetScore = "minecraft:reset_score" {
        /// Score holder.
        owner: String,
        /// Objective, or all.
        objective: Option<String>,
    }
}
packet! {
    /// Action bar text.
    SetActionBarText = "minecraft:set_action_bar_text" {
        /// Text.
        text: Component,
    }
}
packet! {
    /// Shows a scoreboard objective in a display slot.
    SetDisplayObjective = "minecraft:set_display_objective" {
        /// 0 list, 1 sidebar, 2 below name, 3..18 team sidebars.
        position: i32 as VarInt,
        /// Objective name.
        name: String,
    }
}
/// Score number format.
#[derive(Clone, Debug, PartialEq)]
pub enum NumberFormat {
    /// 0: show nothing.
    Blank,
    /// 1: styled, style compound as raw NBT.
    Styled(Bytes),
    /// 2: fixed text.
    Fixed(Component),
}
impl Wire for NumberFormat {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(match r.var_int()? {
            0 => NumberFormat::Blank,
            1 => NumberFormat::Styled(r.nbt_raw()?),
            2 => NumberFormat::Fixed(r.component()?),
            t => return Err(DecodeError::Invalid(format!("number format {t}"))),
        })
    }
    fn write(&self, w: &mut Writer) {
        match self {
            NumberFormat::Blank => w.var_int(0),
            NumberFormat::Styled(s) => {
                w.var_int(1);
                w.bytes(s);
            }
            NumberFormat::Fixed(c) => {
                w.var_int(2);
                w.component(c);
            }
        }
    }
}

/// Objective display data (modes 0 and 2 of `set_objective`).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveDisplay {
    /// Display text.
    pub value: Component,
    /// 0 integer, 1 hearts.
    pub render_type: i32,
    /// Number format.
    pub number_format: Option<NumberFormat>,
}

/// Creates, removes or updates a scoreboard objective.
#[derive(Clone, Debug, PartialEq)]
pub struct SetObjective {
    /// Objective name.
    pub name: String,
    /// 0 create, 1 remove, 2 update.
    pub mode: i8,
    /// Present for modes 0 and 2.
    pub display: Option<ObjectiveDisplay>,
}
impl Packet for SetObjective {
    const NAME: &'static str = "minecraft:set_objective";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let name = r.string()?;
        let mode = r.i8()?;
        let display = if mode == 0 || mode == 2 {
            Some(ObjectiveDisplay {
                value: r.component()?,
                render_type: r.var_int()?,
                number_format: r.read()?,
            })
        } else {
            None
        };
        Ok(Self {
            name,
            mode,
            display,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.name);
        w.i8(self.mode);
        if let Some(d) = &self.display {
            w.component(&d.value);
            w.var_int(d.render_type);
            w.write(&d.number_format);
        }
    }
}
/// Team display parameters (methods 0 and 2 of `set_player_team`).
///
/// Field order follows the 26.3 wiki; the colour is a Prefixed Optional
/// VarInt as Pumpkin writes it for 26.2+ (the wiki lists a plain VarInt):
/// uncertain, see NOTES.md.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamParameters {
    /// Display name.
    pub display_name: Component,
    /// Prefix.
    pub prefix: Component,
    /// Suffix.
    pub suffix: Component,
    /// 0 always, 1 never, 2 hide for other teams, 3 hide for own team.
    pub name_tag_visibility: i32,
    /// 0 always, 1 never, 2 push other teams, 3 push own team.
    pub collision_rule: i32,
    /// Formatting colour id, if any.
    pub color: Option<i32>,
    /// 0x01 friendly fire, 0x02 see invisible teammates.
    pub flags: i8,
}
impl Wire for TeamParameters {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            display_name: r.component()?,
            prefix: r.component()?,
            suffix: r.component()?,
            name_tag_visibility: r.var_int()?,
            collision_rule: r.var_int()?,
            color: r.option(|r| r.var_int())?,
            flags: r.i8()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.component(&self.display_name);
        w.component(&self.prefix);
        w.component(&self.suffix);
        w.var_int(self.name_tag_visibility);
        w.var_int(self.collision_rule);
        w.option(&self.color, |w, c| w.var_int(*c));
        w.i8(self.flags);
    }
}

/// Team update.
#[derive(Clone, Debug, PartialEq)]
pub struct SetPlayerTeam {
    /// Team name.
    pub name: String,
    /// 0 create, 1 remove, 2 update, 3 add entities, 4 remove entities.
    pub method: i8,
    /// Parameters for methods 0 and 2.
    pub parameters: Option<TeamParameters>,
    /// Entities for methods 0, 3 and 4.
    pub entities: Vec<String>,
}
impl Packet for SetPlayerTeam {
    const NAME: &'static str = "minecraft:set_player_team";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let name = r.string()?;
        let method = r.i8()?;
        let parameters = if method == 0 || method == 2 {
            Some(r.read()?)
        } else {
            None
        };
        let entities = if matches!(method, 0 | 3 | 4) {
            r.read()?
        } else {
            Vec::new()
        };
        Ok(Self {
            name,
            method,
            parameters,
            entities,
        })
    }
    fn encode(&self, w: &mut Writer) {
        w.string(&self.name);
        w.i8(self.method);
        if let Some(p) = &self.parameters {
            w.write(p);
        }
        if matches!(self.method, 0 | 3 | 4) {
            w.write(&self.entities);
        }
    }
}
packet! {
    /// Sets a score.
    SetScore = "minecraft:set_score" {
        /// Score holder.
        owner: String,
        /// Objective.
        objective: String,
        /// Value.
        value: i32 as VarInt,
        /// Display name.
        display_name: Option<Component>,
        /// Number format.
        number_format: Option<NumberFormat>,
    }
}
packet! {
    /// Subtitle.
    SetSubtitleText = "minecraft:set_subtitle_text" {
        /// Text.
        text: Component,
    }
}

packet! {
    /// Title.
    SetTitleText = "minecraft:set_title_text" {
        /// Text.
        text: Component,
    }
}

packet! {
    /// Title fade timings.
    SetTitlesAnimation = "minecraft:set_titles_animation" {
        /// Fade in ticks.
        fade_in: i32,
        /// Stay ticks.
        stay: i32,
        /// Fade out ticks.
        fade_out: i32,
    }
}
packet! {
    /// NBT query answer.
    TagQuery = "minecraft:tag_query" {
        /// Transaction id.
        transaction_id: i32 as VarInt,
        /// NBT (raw; may be TAG_End).
        nbt: Bytes as RawNbt,
    }
}
/// A dialog as `ID or NBT`: a `minecraft:dialog` registry id or an inline
/// definition kept as raw network NBT.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogNbt(pub Bytes);
impl Wire for DialogNbt {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self(r.nbt_raw()?))
    }
    fn write(&self, w: &mut Writer) {
        w.bytes(&self.0)
    }
}

packet! {
    /// Shows a dialog.
    ShowDialog = "minecraft:show_dialog" {
        /// Registry id or inline NBT.
        dialog: IdOr<DialogNbt>,
    }
}
