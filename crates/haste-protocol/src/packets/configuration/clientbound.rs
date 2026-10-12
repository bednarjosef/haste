//! Clientbound configuration packets. Several are shared with the play state
//! (same layout); play re-exports them.
use bytes::Bytes;
use haste_core::{Component, Identifier};
use uuid::Uuid;

use crate::packets::{packet, packet_enum};
use crate::wire::{ByteArray, DecodeResult, List, RawNbt, Reader, Rest, VarInt, Wire, Writer};

packet_enum! {
    /// Clientbound configuration packets.
    ClientboundConfiguration {
        CookieRequest, CustomPayload, Disconnect, FinishConfiguration, KeepAlive, Ping, ResetChat,
        RegistryData, ResourcePackPop, ResourcePackPush, PostEffects, StoreCookie, Transfer,
        UpdateEnabledFeatures, UpdateTags, SelectKnownPacks, CustomReportDetails, ServerLinks,
        ClearDialog, ShowDialog, CodeOfConduct,
    }
}

packet! {
    /// Asks for a stored cookie.
    CookieRequest = "minecraft:cookie_request" {
        /// Cookie key.
        key: Identifier,
    }
}

packet! {
    /// Plugin message.
    CustomPayload = "minecraft:custom_payload" {
        /// Channel, e.g. `minecraft:brand`.
        channel: Identifier,
        /// Channel-specific data.
        data: Bytes as Rest,
    }
}

packet! {
    /// Disconnect with a reason.
    Disconnect = "minecraft:disconnect" {
        /// Why.
        reason: Component,
    }
}

packet! {
    /// Configuration done; the client acknowledges and enters play.
    FinishConfiguration = "minecraft:finish_configuration" {}
}

packet! {
    /// Keep-alive; must be echoed within 15 s.
    KeepAlive = "minecraft:keep_alive" {
        /// Id to echo.
        id: i64,
    }
}

packet! {
    /// Ping; answered with `pong`.
    Ping = "minecraft:ping" {
        /// Id to echo.
        id: i32,
    }
}

packet! {
    /// Clears the chat history.
    ResetChat = "minecraft:reset_chat" {}
}

/// One entry of a `registry_data` packet.
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryEntry {
    /// Entry name, e.g. `minecraft:overworld`.
    pub id: Identifier,
    /// Entry data as raw network NBT; `None` when it comes from a known pack.
    pub data: Option<Bytes>,
}
impl Wire for RegistryEntry {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            id: r.identifier()?,
            data: r.option(|r| r.nbt_raw())?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.id);
        w.option(&self.data, |w, d| w.bytes(d));
    }
}

packet! {
    /// The contents of one synchronized registry. Entry order defines the
    /// numeric ids.
    RegistryData = "minecraft:registry_data" {
        /// Registry, e.g. `minecraft:dimension_type`.
        registry: Identifier,
        /// Entries in id order.
        entries: Vec<RegistryEntry>,
    }
}

packet! {
    /// Removes a resource pack (or all).
    ResourcePackPop = "minecraft:resource_pack_pop" {
        /// The pack, or `None` for all.
        id: Option<Uuid>,
    }
}

packet! {
    /// Offers a resource pack.
    ResourcePackPush = "minecraft:resource_pack_push" {
        /// Pack id.
        id: Uuid,
        /// Download URL.
        url: String,
        /// SHA-1 hex.
        hash: String,
        /// Whether declining disconnects.
        forced: bool,
        /// Prompt text.
        prompt: Option<Component>,
    }
}

packet! {
    /// Active post-processing effects.
    PostEffects = "minecraft:post_effects" {
        /// Effect ids.
        effects: Vec<Identifier>,
    }
}

packet! {
    /// Stores a cookie on the client.
    StoreCookie = "minecraft:store_cookie" {
        /// Cookie key.
        key: Identifier,
        /// Payload (at most 5 KiB).
        payload: Bytes as ByteArray,
    }
}

packet! {
    /// Asks the client to connect to another server.
    Transfer = "minecraft:transfer" {
        /// Host.
        host: String,
        /// Port.
        port: i32 as VarInt,
    }
}

packet! {
    /// Enabled feature flags.
    UpdateEnabledFeatures = "minecraft:update_enabled_features" {
        /// Flags, e.g. `minecraft:vanilla`.
        features: Vec<Identifier>,
    }
}

/// One tag of a registry in `update_tags`.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    /// Tag name without `#`.
    pub name: Identifier,
    /// Registry ids in the tag.
    pub entries: Vec<i32>,
}
impl Wire for Tag {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            name: r.identifier()?,
            entries: r.read_as::<_, List<VarInt>>()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.name);
        w.write_as::<_, List<VarInt>>(&self.entries);
    }
}

/// The tags of one registry in `update_tags`.
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryTags {
    /// Registry, e.g. `minecraft:block`.
    pub registry: Identifier,
    /// Its tags.
    pub tags: Vec<Tag>,
}
impl Wire for RegistryTags {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            registry: r.identifier()?,
            tags: r.read()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.identifier(&self.registry);
        w.write(&self.tags);
    }
}

packet! {
    /// Registry tags.
    UpdateTags = "minecraft:update_tags" {
        /// Per registry.
        registries: Vec<RegistryTags>,
    }
}

/// A data pack identity in known-pack negotiation.
#[derive(Clone, Debug, PartialEq)]
pub struct KnownPack {
    /// Namespace, e.g. `minecraft`.
    pub namespace: String,
    /// Id, e.g. `core`.
    pub id: String,
    /// Version, e.g. `26.3`.
    pub version: String,
}
impl Wire for KnownPack {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            namespace: r.string()?,
            id: r.string()?,
            version: r.string()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.namespace);
        w.string(&self.id);
        w.string(&self.version);
    }
}

packet! {
    /// The server's data packs; the client answers with those it knows.
    SelectKnownPacks = "minecraft:select_known_packs" {
        /// Packs on the server.
        packs: Vec<KnownPack>,
    }
}

/// One entry of `custom_report_details`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReportDetail {
    /// Title.
    pub title: String,
    /// Description.
    pub description: String,
}
impl Wire for ReportDetail {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            title: r.string_max(128)?,
            description: r.string_max(4096)?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.string(&self.title);
        w.string(&self.description);
    }
}

packet! {
    /// Key-value lines for crash/disconnect reports.
    CustomReportDetails = "minecraft:custom_report_details" {
        /// Details.
        details: Vec<ReportDetail>,
    }
}

/// Label of a server link: built-in (enum id) or custom text.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerLinkLabel {
    /// Built-in label id (0 bug report .. 9 announcements).
    BuiltIn(i32),
    /// Custom text.
    Custom(Component),
}

/// One server link.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerLink {
    /// Label.
    pub label: ServerLinkLabel,
    /// URL.
    pub url: String,
}
impl Wire for ServerLink {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let label = if r.bool()? {
            ServerLinkLabel::BuiltIn(r.var_int()?)
        } else {
            ServerLinkLabel::Custom(r.component()?)
        };
        Ok(Self {
            label,
            url: r.string()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        match &self.label {
            ServerLinkLabel::BuiltIn(i) => {
                w.bool(true);
                w.var_int(*i);
            }
            ServerLinkLabel::Custom(c) => {
                w.bool(false);
                w.component(c);
            }
        }
        w.string(&self.url);
    }
}

packet! {
    /// Links shown in the pause menu.
    ServerLinks = "minecraft:server_links" {
        /// Links.
        links: Vec<ServerLink>,
    }
}

packet! {
    /// Closes the current dialog.
    ClearDialog = "minecraft:clear_dialog" {}
}

packet! {
    /// Shows a dialog (inline NBT definition, kept raw).
    ShowDialog = "minecraft:show_dialog" {
        /// Dialog definition as network NBT.
        dialog: Bytes as RawNbt,
    }
}

packet! {
    /// The server's code of conduct; configuration waits for
    /// `accept_code_of_conduct`.
    CodeOfConduct = "minecraft:code_of_conduct" {
        /// Text.
        text: String,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    fn nbt_bytes() -> Bytes {
        let mut c = crate::nbt::NbtCompound::new();
        c.put_int("min_y", -64);
        let mut w = Writer::new();
        w.nbt_compound(Some(&c));
        w.into_inner().into()
    }

    #[test]
    fn round_trips() {
        round_trip(&CookieRequest {
            key: Identifier::minecraft("c"),
        });
        round_trip(&CustomPayload {
            channel: Identifier::minecraft("brand"),
            data: Bytes::from_static(b"\x07vanilla"),
        });
        round_trip(&Disconnect {
            reason: Component::text("x"),
        });
        round_trip(&FinishConfiguration {});
        round_trip(&KeepAlive { id: -9 });
        round_trip(&Ping { id: 4 });
        round_trip(&ResetChat {});
        round_trip(&RegistryData {
            registry: Identifier::minecraft("dimension_type"),
            entries: vec![
                RegistryEntry {
                    id: Identifier::minecraft("overworld"),
                    data: Some(nbt_bytes()),
                },
                RegistryEntry {
                    id: Identifier::minecraft("the_nether"),
                    data: None,
                },
            ],
        });
        round_trip(&ResourcePackPop {
            id: Some(Uuid::from_u128(1)),
        });
        round_trip(&ResourcePackPush {
            id: Uuid::from_u128(2),
            url: "http://x".into(),
            hash: "ab".into(),
            forced: false,
            prompt: Some(Component::text("p")),
        });
        round_trip(&PostEffects {
            effects: vec![Identifier::minecraft("blur")],
        });
        round_trip(&StoreCookie {
            key: Identifier::minecraft("k"),
            payload: Bytes::from_static(b"v"),
        });
        round_trip(&Transfer {
            host: "example.org".into(),
            port: 25565,
        });
        round_trip(&UpdateEnabledFeatures {
            features: vec![Identifier::minecraft("vanilla")],
        });
        round_trip(&UpdateTags {
            registries: vec![RegistryTags {
                registry: Identifier::minecraft("block"),
                tags: vec![Tag {
                    name: Identifier::minecraft("logs"),
                    entries: vec![1, 2, 300],
                }],
            }],
        });
        round_trip(&SelectKnownPacks {
            packs: vec![KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: "26.3".into(),
            }],
        });
        round_trip(&CustomReportDetails {
            details: vec![ReportDetail {
                title: "a".into(),
                description: "b".into(),
            }],
        });
        round_trip(&ServerLinks {
            links: vec![
                ServerLink {
                    label: ServerLinkLabel::BuiltIn(0),
                    url: "u".into(),
                },
                ServerLink {
                    label: ServerLinkLabel::Custom(Component::text("c")),
                    url: "v".into(),
                },
            ],
        });
        round_trip(&ClearDialog {});
        round_trip(&ShowDialog {
            dialog: nbt_bytes(),
        });
        round_trip(&CodeOfConduct {
            text: "be nice".into(),
        });
    }
}
