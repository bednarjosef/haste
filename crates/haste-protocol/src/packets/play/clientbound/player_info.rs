use super::*;

packet! {
    /// Removes players from the tab list.
    PlayerInfoRemove = "minecraft:player_info_remove" {
        /// Player UUIDs.
        uuids: Vec<Uuid>,
    }
}

/// Chat session data in `player_info_update` (action 0x02).
#[derive(Clone, Debug, PartialEq)]
pub struct ChatSession {
    /// Session id.
    pub session_id: Uuid,
    /// Key expiry, ms since epoch.
    pub expires_at: i64,
    /// Public key, DER.
    pub public_key: Bytes,
    /// Mojang's signature of the key.
    pub key_signature: Bytes,
}
impl Wire for ChatSession {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            session_id: r.uuid()?,
            expires_at: r.i64()?,
            public_key: r.byte_array(512)?,
            key_signature: r.byte_array(4096)?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.uuid(&self.session_id);
        w.i64(self.expires_at);
        w.byte_array(&self.public_key);
        w.byte_array(&self.key_signature);
    }
}

/// `player_info_update` action bits.
pub mod player_info_actions {
    #![allow(missing_docs)]
    pub const ADD_PLAYER: u8 = 0x01;
    pub const INITIALIZE_CHAT: u8 = 0x02;
    pub const UPDATE_GAME_MODE: u8 = 0x04;
    pub const UPDATE_LISTED: u8 = 0x08;
    pub const UPDATE_LATENCY: u8 = 0x10;
    pub const UPDATE_DISPLAY_NAME: u8 = 0x20;
    pub const UPDATE_LIST_PRIORITY: u8 = 0x40;
    pub const UPDATE_HAT: u8 = 0x80;
}

/// One player's entry in `player_info_update`; fields are `Some` exactly
/// when the packet's action mask has the corresponding bit.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlayerInfoEntry {
    /// Player UUID.
    pub uuid: Uuid,
    /// Add player: name and profile properties.
    pub add: Option<(String, Vec<ProfileProperty>)>,
    /// Initialize chat.
    pub chat_session: Option<Option<ChatSession>>,
    /// Game mode id.
    pub game_mode: Option<i32>,
    /// Listed on the tab list.
    pub listed: Option<bool>,
    /// Latency in ms.
    pub latency: Option<i32>,
    /// Display name.
    pub display_name: Option<Option<Component>>,
    /// Tab list priority.
    pub list_priority: Option<i32>,
    /// Hat layer visible.
    pub show_hat: Option<bool>,
}

/// Adds or updates tab list entries.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfoUpdate {
    /// Action bit mask ([`player_info_actions`]).
    pub actions: u8,
    /// Entries.
    pub entries: Vec<PlayerInfoEntry>,
}
impl PlayerInfoUpdate {
    /// As the shared `PlayerInfo` (entries without `ADD_PLAYER` get an empty name).
    pub fn infos(&self) -> Vec<haste_core::entity::PlayerInfo> {
        self.entries
            .iter()
            .map(|e| haste_core::entity::PlayerInfo {
                uuid: e.uuid,
                name: e.add.as_ref().map(|a| a.0.clone()).unwrap_or_default(),
                display_name: e.display_name.clone().flatten(),
                latency: e.latency.unwrap_or(0),
                game_mode: e.game_mode.and_then(haste_core::game::GameMode::from_id),
                listed: e.listed.unwrap_or(false),
                skin_texture_url: None,
            })
            .collect()
    }
}
impl Packet for PlayerInfoUpdate {
    const NAME: &'static str = "minecraft:player_info_update";
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        use player_info_actions::*;
        let actions = r.u8()?;
        let entries = r.list(4096, |r| {
            let mut e = PlayerInfoEntry {
                uuid: r.uuid()?,
                ..Default::default()
            };
            if actions & ADD_PLAYER != 0 {
                e.add = Some((r.string_max(16)?, r.list(16, ProfileProperty::read)?));
            }
            if actions & INITIALIZE_CHAT != 0 {
                e.chat_session = Some(r.read()?);
            }
            if actions & UPDATE_GAME_MODE != 0 {
                e.game_mode = Some(r.var_int()?);
            }
            if actions & UPDATE_LISTED != 0 {
                e.listed = Some(r.bool()?);
            }
            if actions & UPDATE_LATENCY != 0 {
                e.latency = Some(r.var_int()?);
            }
            if actions & UPDATE_DISPLAY_NAME != 0 {
                e.display_name = Some(r.read()?);
            }
            if actions & UPDATE_LIST_PRIORITY != 0 {
                e.list_priority = Some(r.var_int()?);
            }
            if actions & UPDATE_HAT != 0 {
                e.show_hat = Some(r.bool()?);
            }
            Ok(e)
        })?;
        Ok(Self { actions, entries })
    }
    fn encode(&self, w: &mut Writer) {
        w.u8(self.actions);
        w.list(&self.entries, |w, e| {
            w.uuid(&e.uuid);
            if let Some((name, props)) = &e.add {
                w.string(name);
                w.write(props);
            }
            if let Some(c) = &e.chat_session {
                w.write(c);
            }
            if let Some(g) = e.game_mode {
                w.var_int(g);
            }
            if let Some(l) = e.listed {
                w.bool(l);
            }
            if let Some(l) = e.latency {
                w.var_int(l);
            }
            if let Some(d) = &e.display_name {
                w.write(d);
            }
            if let Some(p) = e.list_priority {
                w.var_int(p);
            }
            if let Some(h) = e.show_hat {
                w.bool(h);
            }
        });
    }
}
