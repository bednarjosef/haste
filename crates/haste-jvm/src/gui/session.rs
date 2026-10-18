//! Connection session into the Java side: configuration-phase registries and tags, play packets,
//! the local player, dimension and feature state, and the packet-driven screens.

use jni::objects::{JObjectArray, JString};
use jni::{jni_sig, jni_str};

use super::GuiPipeline;
use crate::proxies::{encode_stack, with_state};
use haste_core::entity::ItemStackData;
use haste_core::BlockPos;

impl GuiPipeline {
    /// One `registry_data` packet of the configuration phase (body without the packet id, exactly
    /// as received): collected and built into vanilla registries by [`Self::finish_configuration`].
    pub fn registry_data(&self, body: &[u8]) -> anyhow::Result<()> {
        Ok(self.call_bytes("registryData", body, None)?)
    }

    /// An `update_tags` packet body (without id). `play`: received in the play phase, where the tags
    /// are applied to the current registries at once; in configuration they wait for
    /// [`Self::finish_configuration`].
    pub fn tags(&self, body: &[u8], play: bool) -> anyhow::Result<()> {
        Ok(self.call_bytes("tags", body, Some(play))?)
    }

    /// End of the configuration phase: vanilla's `RegistryDataCollector` builds the synchronised
    /// registries (entries decoded with their codecs, tags bound, item components initialised).
    pub fn finish_configuration(&self) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("finishConfiguration"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Builds the synchronised registries from the vanilla data pack inside the player's client jar
    /// instead of a server's `registry_data` (no tags). For tests and anything that needs registry
    /// data without a connection; a connection's registries replace these.
    pub fn load_jar_registries(&self) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            env.call_method(bridge, jni_str!("loadJarRegistries"), jni_sig!("()V"), &[])?;
            Ok(())
        })?;
        Ok(())
    }

    /// A clientbound play packet whose state lives in tier 3 (chat, boss bars, scoreboard, titles,
    /// tab list, commands, containers, ...): VarInt packet id + body, the exact wire bytes. Java
    /// decodes it with vanilla's codec and handles it.
    pub fn packet(&self, id_and_body: &[u8]) -> anyhow::Result<()> {
        Ok(self.call_bytes("packet", id_and_body, None)?)
    }

    /// A use opened the command block editor at a position (vanilla
    /// `LocalPlayer.openCommandBlock`; the block entity comes from the Java-side store).
    pub fn open_command_block(&self, pos: BlockPos) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("openCommandBlock"),
                jni_sig!("(III)V"),
                &[
                    jni::objects::JValue::Int(pos.x),
                    jni::objects::JValue::Int(pos.y),
                    jni::objects::JValue::Int(pos.z),
                ],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// A use opened the writable book's editor (vanilla `LocalPlayer.openItemGui`); `hand` is
    /// `InteractionHand.values()` numbering (0 main, 1 off).
    pub fn open_item_gui(&self, hand: i32) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("openItemGui"),
                jni_sig!("(I)V"),
                &[jni::objects::JValue::Int(hand)],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Sets a player inventory slot (vanilla `Inventory` numbering) in Java; the mirror in
    /// [`GameState::inventory`] follows.
    pub fn set_inventory_slot(&self, slot: i32, stack: &ItemStackData) -> anyhow::Result<()> {
        let b = encode_stack(stack);
        self.with_bridge(|env, bridge| {
            let arr = env.byte_array_from_slice(&b)?;
            env.call_method(
                bridge,
                jni_str!("setInventorySlot"),
                jni_sig!("(I[B)V"),
                &[jni::objects::JValue::Int(slot), (&arr).into()],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// The local player as the server introduced it: entity id (from `login`; effect, attribute
    /// and entity-data packets for this id update the player proxy), name and profile id.
    pub fn set_player(&self, entity_id: i32, name: &str, uuid: u128) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            let n = JString::from_str(env, name)?;
            env.call_method(
                bridge,
                jni_str!("setPlayer"),
                jni_sig!("(ILjava/lang/String;JJ)V"),
                &[
                    jni::objects::JValue::Int(entity_id),
                    (&n).into(),
                    jni::objects::JValue::Long((uuid >> 64) as i64),
                    jni::objects::JValue::Long(uuid as u64 as i64),
                ],
            )?;
            Ok(())
        })?;
        with_state(|s| s.player_name = name.to_owned());
        Ok(())
    }

    /// The joined world's dimension type: its id in the synchronised `minecraft:dimension_type`
    /// registry (from `login`/`respawn`).
    pub fn set_dimension_type(&self, id: i32) {
        if let Err(e) = self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("setDimensionType"),
                jni_sig!("(I)V"),
                &[jni::objects::JValue::Int(id)],
            )?;
            Ok(())
        }) {
            self.failed("set_dimension_type", e);
        }
    }

    /// The server's enabled feature flags (configuration phase `update_enabled_features`).
    pub fn set_features(&self, ids: &[String]) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            let arr = JObjectArray::<JString>::new(env, ids.len(), JString::null())?;
            for (i, id) in ids.iter().enumerate() {
                let s = JString::from_str(env, id)?;
                arr.set_element(env, i, &s)?;
            }
            env.call_method(
                bridge,
                jni_str!("setFeatures"),
                jni_sig!("([Ljava/lang/String;)V"),
                &[(&arr).into()],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// The pause-menu server links from the configuration phase's `server_links` packet, one
    /// string per link: `"<b|c>\t<label>\t<url>"` (b = built-in label id, c = custom label text).
    pub fn set_server_links(&self, links: &[String]) -> anyhow::Result<()> {
        self.with_bridge(|env, bridge| {
            let arr = JObjectArray::<JString>::new(env, links.len(), JString::null())?;
            for (i, l) in links.iter().enumerate() {
                let s = JString::from_str(env, l)?;
                arr.set_element(env, i, &s)?;
            }
            env.call_method(
                bridge,
                jni_str!("setServerLinks"),
                jni_sig!("([Ljava/lang/String;)V"),
                &[(&arr).into()],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Tab held or released: vanilla's player list overlay is shown while it is down.
    pub fn set_tab_list_visible(&self, visible: bool) {
        if let Err(e) = self.with_bridge(|env, bridge| {
            env.call_method(
                bridge,
                jni_str!("setTabListVisible"),
                jni_sig!("(Z)V"),
                &[jni::objects::JValue::Bool(visible)],
            )?;
            Ok(())
        }) {
            self.failed("set_tab_list_visible", e);
        }
    }

    /// The player started riding (vanilla `ClientPacketListener.handleSetEntityPassengersPacket`):
    /// the "Press <sneak key> to dismount" overlay message, with the bound key's name.
    pub fn show_mount_message(&self) {
        if let Err(e) = self.with_bridge(|env, bridge| {
            env.call_method(bridge, jni_str!("showMountMessage"), jni_sig!("()V"), &[])?;
            Ok(())
        }) {
            self.failed("show_mount_message", e);
        }
    }
}
