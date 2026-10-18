//! The packet bridge of [`Ui`] (jvm-host): which clientbound play packets the UI keeps
//! (tier-2 state mirrored into [`GameState`], tier-3 packets re-encoded into Java), the join
//! ([`Ui::joined`]), and the outgoing packets vanilla code sent, decoded back into
//! [`ServerboundPlay`]. See the module documentation of [`crate::ui`].

use super::*;

/// Clientbound play packets whose state lives in tier 3 (forwarded to Java whole).
pub(super) const FORWARDED: &[&str] = &[
    "minecraft:system_chat",
    "minecraft:player_chat",
    "minecraft:disguised_chat",
    "minecraft:delete_chat",
    "minecraft:custom_chat_completions",
    "minecraft:set_title_text",
    "minecraft:set_subtitle_text",
    "minecraft:set_titles_animation",
    "minecraft:set_action_bar_text",
    "minecraft:clear_titles",
    "minecraft:boss_event",
    "minecraft:tab_list",
    "minecraft:player_info_update",
    "minecraft:player_info_remove",
    "minecraft:set_objective",
    "minecraft:set_score",
    "minecraft:reset_score",
    "minecraft:set_display_objective",
    "minecraft:set_player_team",
    "minecraft:commands",
    "minecraft:command_suggestions",
    "minecraft:update_advancements",
    "minecraft:select_advancements_tab",
    "minecraft:award_stats",
    "minecraft:game_rule_values",
    "minecraft:open_screen",
    "minecraft:update_recipes",
    "minecraft:container_set_content",
    "minecraft:container_set_slot",
    "minecraft:container_set_data",
    "minecraft:container_close",
    "minecraft:set_cursor_item",
    "minecraft:set_player_inventory",
    "minecraft:merchant_offers",
    "minecraft:update_tags",
    // the screens that edit block entities (signs, command blocks): the entity store and the
    // editors live in Java (see haste.gui.BlockEntities)
    "minecraft:block_entity_data",
    "minecraft:open_sign_editor",
    "minecraft:open_book",
    // the store drops the chunk's entities with the chunk (vanilla handleForgetLevelChunk)
    "minecraft:forget_level_chunk",
    // the recipe book (real vanilla Java over the forwarded packets): the book's displays and
    // settings, the recipe container, and the ghost recipe the overlay shows on a right-click
    "minecraft:recipe_book_add",
    "minecraft:recipe_book_remove",
    "minecraft:recipe_book_settings",
    "minecraft:update_recipes",
    "minecraft:place_ghost_recipe",
];

/// Packets about one entity: forwarded only when they are about the local player.
pub(super) const FORWARDED_LOCAL: &[&str] = &[
    "minecraft:set_entity_data",
    "minecraft:update_mob_effect",
    "minecraft:remove_mob_effect",
    "minecraft:update_attributes",
    "minecraft:player_combat_kill",
];

/// Whether [`Ui::handle`] uses this packet (tier-2 state or a forwarded tier-3 packet). The
/// HUD JVM boots for seconds while play packets already arrive; `NetDriver` keeps these packets
/// until the `Ui` exists so a fresh join's tab list, boss bars and scoreboard are not lost.
pub fn ui_handles(p: &ClientboundPlay) -> bool {
    use ClientboundPlay as P;
    matches!(
        p,
        P::SetHealth(_)
            | P::SetExperience(_)
            | P::PlayerAbilities(_)
            | P::GameEvent(_)
            | P::PlayerCombatKill(_)
            | P::Respawn(_)
            | P::SetHeldSlot(_)
            | P::EntityEvent(_)
            | P::SetTime(_)
    ) || p
        .name()
        .is_some_and(|n| FORWARDED.contains(&n) || FORWARDED_LOCAL.contains(&n))
}

/// The entity a single-entity packet is about.
fn entity_of(p: &ClientboundPlay) -> Option<i32> {
    use ClientboundPlay as P;
    Some(match p {
        P::SetEntityData(x) => x.entity_id,
        P::UpdateMobEffect(x) => x.entity_id,
        P::RemoveMobEffect(x) => x.entity_id,
        P::UpdateAttributes(x) => x.entity_id,
        P::PlayerCombatKill(x) => x.player_id,
        _ => return None,
    })
}

fn body<P: Packet>(p: &P) -> Vec<u8> {
    let mut w = Writer::new();
    p.encode(&mut w);
    w.into_inner()
}

fn encode_body(p: &ClientboundPlay) -> Vec<u8> {
    let mut w = Writer::new();
    p.encode_body(&mut w);
    w.into_inner()
}

/// Decodes a serverbound packet vanilla code encoded (VarInt id + body).
fn decode_serverbound(
    dispatch: &Dispatch<ServerboundPlay>,
    tables: &ProtocolTables,
    p: &[u8],
) -> Result<ServerboundPlay, String> {
    let mut i = 0;
    let id = read_var_int(p, &mut i).ok_or("truncated packet id")?;
    let body = Vec::from(&p[i..]).into();
    match dispatch.decode(id, &body, Some(tables)) {
        Decoded::Ok(sp) => Ok(sp),
        Decoded::NoDecoder => Ok(ServerboundPlay::Unknown { id, body }),
        Decoded::Failed(name, e) => Err(format!("{name}: {e}")),
    }
}

pub(super) fn read_var_int(b: &[u8], i: &mut usize) -> Option<i32> {
    let mut v: u32 = 0;
    for shift in (0..35).step_by(7) {
        let x = *b.get(*i)?;
        *i += 1;
        v |= u32::from(x & 0x7f) << shift;
        if x & 0x80 == 0 {
            return Some(v as i32);
        }
    }
    None
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Ui {
    /// Joined a server (after `Connection::connect`): registries, tags and feature flags from the
    /// configuration phase, the local player from `login`.
    ///
    /// `game_mode` is the mode the client has actually seen (the session's, which follows every
    /// packet since the `login`). The HUD's JVM takes seconds to start, and packets that arrive
    /// before it (a server that switches a joining player's mode, a respawn with another mode)
    /// never reach [`Ui::handle`]; without the session's mode the `login` packet's would win and
    /// the HUD would show the mode of the world-loading screen (no hearts in survival, or hearts
    /// in creative). Where vanilla stores the local mode is `MultiPlayerGameMode.localMode`
    /// (`ClientPacketListener.handleGameEvent` -> `MultiPlayerGameMode.setLocalMode`); here the
    /// equivalent store is [`haste_jvm::GameState::game_mode`], which every game mode change
    /// writes: this call and [`Ui::handle`]'s `game_event` and respawn arms alike.
    pub fn joined(
        &self,
        conn: &Connection,
        login: &Login,
        game_mode: GameMode,
    ) -> anyhow::Result<()> {
        let tables = conn.tables().clone();
        *self.shared.serverbound.lock() = Some(Dispatch::new(
            &tables.packets,
            State::Play,
            Direction::Serverbound,
            ServerboundPlay::DECODERS,
        ));
        *self.shared.tables.lock() = Some(tables);
        let regs = conn.registries();
        for name in regs.names() {
            let entries = regs.entries(name).unwrap_or_default().to_vec();
            let p = RegistryData {
                registry: name.clone(),
                entries,
            };
            self.gui().registry_data(&body(&p))?;
        }
        if !regs.tags.is_empty() {
            let p = UpdateTags {
                registries: regs.tags.clone(),
            };
            self.gui().tags(&body(&p), false)?;
        }
        self.gui().finish_configuration()?;
        let features: Vec<String> = regs.features.iter().map(|f| f.to_string()).collect();
        self.gui().set_features(&features)?;
        // the pause menu's server links (b = built-in label id, c = custom label flattened to text)
        let links: Vec<String> = regs
            .server_links
            .iter()
            .map(|l| match &l.label {
                ServerLinkLabel::BuiltIn(id) => {
                    format!("b\t{id}\t{}", l.url)
                }
                ServerLinkLabel::Custom(c) => {
                    format!("c\t{}\t{}", c.to_plain(&|_| None), l.url)
                }
            })
            .collect();
        self.gui().set_server_links(&links)?;
        let entity = login.entity_id.0;
        self.shared.local_entity.store(entity, Ordering::Relaxed);
        self.gui()
            .set_player(entity, &login.profile.name, login.profile.uuid.as_u128())?;
        self.gui()
            .set_dimension_type(login.packet.spawn.dimension_type);
        self.gui().with_state(|s| {
            s.in_world = true;
            s.game_mode = game_mode;
            s.vitals.hardcore = login.hardcore;
            s.immediate_respawn = !login.packet.enable_respawn_screen;
            s.disconnect_requested = false;
        });
        // Vanilla keeps the connecting/loading screen up until the level exists
        // (`ClientPacketListener.handleLogin` -> `startWaitingForNewLevel`, the loading screen
        // closing when the level is ready) — never `setScreen(null)` before that, or the
        // no-level fallback opens the title screen over the join. The level exists from here.
        if self.gui().is_open() {
            self.gui().close_screen();
        }
        Ok(())
    }

    /// One clientbound play packet (call for every packet, in order, on the game thread).
    /// `block_state` reads the world's block state at a position (the caller holds the world's
    /// lock while draining packets; the callback must not call back into the JVM).
    pub fn handle(&self, p: &ClientboundPlay, block_state: &dyn Fn(BlockPos) -> BlockStateId) {
        use ClientboundPlay as P;
        let local = self.shared.local_entity.load(Ordering::Relaxed);
        // The block entities the screens edit are built from the real state (vanilla's chunks
        // build them when the block state arrives, `LevelChunk.setBlockState`): record the
        // positions this packet names a block entity at, before Java builds one.
        if let Some(pos) = match p {
            P::BlockEntityData(b) => Some(b.pos),
            P::OpenSignEditor(o) => Some(o.pos),
            _ => None,
        } {
            let id = block_state(pos).0 as i32;
            self.gui().set_block_state(pos, id);
        }
        match p {
            P::SetHealth(h) => self.state(|s| {
                s.vitals.health = h.health;
                s.vitals.food = h.food;
                s.vitals.saturation = h.saturation;
            }),
            P::SetExperience(x) => self.state(|s| {
                s.vitals.xp_progress = x.bar;
                s.vitals.xp_level = x.level;
                s.vitals.xp_total = x.total;
            }),
            P::PlayerAbilities(a) => self.state(|s| {
                let f = a.flags;
                s.vitals.invulnerable = f & 1 != 0;
                s.vitals.flying = f & 2 != 0;
                s.vitals.may_fly = f & 4 != 0;
                s.vitals.instabuild = f & 8 != 0;
                s.vitals.fly_speed = a.flying_speed;
                s.vitals.walk_speed = a.fov_modifier;
            }),
            P::GameEvent(g) if g.event == 3 => {
                if let Some(m) = GameMode::from_id(g.value as i32) {
                    self.state(|s| s.game_mode = m);
                }
            }
            // 11: respawn screen on (0) or immediate respawn (1)
            P::GameEvent(g) if g.event == 11 => {
                self.state(|s| s.immediate_respawn = g.value == 1.0)
            }
            P::PlayerCombatKill(k)
                if k.player_id == local && self.state(|s| s.immediate_respawn) =>
            {
                // no death screen: respawn at once, as the vanilla client does
                self.shared
                    .outbox
                    .lock()
                    .push(ServerboundPlay::ClientCommand(sb::ClientCommand {
                        action: 0,
                    }));
            }
            P::Respawn(r) => {
                // a respawned player is a new entity at full health until the server says otherwise
                self.state(|s| s.vitals.health = 20.0);
                if let Some(m) = GameMode::from_id(r.spawn.game_mode) {
                    self.state(|s| s.game_mode = m);
                }
                self.gui().set_dimension_type(r.spawn.dimension_type);
                // the death screen (or any screen) closes when the player is back in the world
                if self.gui().is_open() {
                    self.gui().close_screen();
                }
            }
            P::SetHeldSlot(h) => {
                // Vanilla's handleSetHeldSlot only sets the inventory slot: the next
                // `gameMode.tick`'s `ensureHasSentCarriedItem` echoes `set_carried_item` back
                // (the player's own `ensure_carried_item` in `tick_inner`, and this side's
                // `sent_slot` follows with it). Marking it sent here would swallow the echo.
                self.state(|s| s.selected_slot = h.slot);
            }
            P::EntityEvent(e) if e.entity_id == local => {
                // 24..=28: operator permission level 0..=4
                if (24..=28).contains(&e.status) {
                    self.state(|s| s.permission_level = (e.status - 24) as u8);
                }
            }
            P::SetTime(t) => self.state(|s| {
                s.game_time = t.world_age;
                if let Some(c) = t.clocks.first() {
                    s.day_time = c.time;
                }
            }),
            // Vanilla drops a chunk's block entities with the chunk (the server forgets it):
            // the Java-side store does the same through the forwarded packet; the states
            // recorded for its positions go with them, so the map does not grow without bound.
            P::ForgetLevelChunk(f) => {
                let (cx, cz) = (f.pos.x, f.pos.z);
                self.gui().forget_recorded_block_states(cx, cz);
            }
            _ => {}
        }
        let Some(name) = p.name() else { return };
        let local_only = FORWARDED_LOCAL.contains(&name);
        if !FORWARDED.contains(&name) && !local_only {
            return;
        }
        if local_only && entity_of(p) != Some(local) {
            return;
        }
        if name == "minecraft:update_tags" {
            if let Err(e) = self.gui().tags(&encode_body(p), true) {
                tracing::warn!("update_tags into Java: {e:#}");
            }
            return;
        }
        let Some(id) = self.packet_id(name) else {
            return;
        };
        let mut w = Writer::new();
        w.var_int(id);
        p.encode_body(&mut w);
        if let Err(e) = self.gui().packet(&w.into_inner()) {
            tracing::warn!("{name} into Java: {e:#}");
        }
    }

    fn packet_id(&self, name: &str) -> Option<i32> {
        self.shared
            .tables
            .lock()
            .as_ref()?
            .packets
            .id(State::Play, Direction::Clientbound, name)
    }

    /// Everything the UI sends this tick, in order: packets vanilla code sent, chat and commands
    /// typed in the chat screen, the selected hotbar slot when it changed, and the respawn a
    /// dead player's immediate respawn queued (`PlayerCombatKill`).
    pub fn drain_outgoing(&self, out: &mut Vec<ServerboundPlay>) {
        out.append(&mut self.shared.outbox.lock());
        let (packets, chat, slot) =
            self.state(|s| (s.take_outgoing(), s.take_sent(), s.selected_slot));
        self.decode_java_sends(packets, out);
        for m in chat {
            out.push(if m.command {
                ServerboundPlay::ChatCommand(sb::ChatCommand { command: m.text })
            } else {
                ServerboundPlay::Chat(sb::Chat {
                    message: m.text,
                    timestamp: now_millis(),
                    salt: self.next_salt(),
                    signature: None,
                    message_count: 0,
                    acknowledged: [0; 3],
                    checksum: 0,
                })
            });
        }
        if self.shared.sent_slot.swap(slot, Ordering::Relaxed) != slot {
            out.push(ServerboundPlay::SetCarriedItem(sb::SetCarriedItem {
                slot: slot as i16,
            }));
        }
    }

    /// Decodes packets vanilla Java code sent (VarInt id + body) into [`ServerboundPlay`] in
    /// order; shared by the per-tick drain and the input-event send.
    fn decode_java_sends(&self, packets: Vec<Vec<u8>>, out: &mut Vec<ServerboundPlay>) {
        if packets.is_empty() {
            return;
        }
        let tables = self.shared.tables.lock().clone();
        let dispatch = self.shared.serverbound.lock();
        if let (Some(tables), Some(dispatch)) = (tables, dispatch.as_ref()) {
            for p in packets {
                match decode_serverbound(dispatch, &tables, &p) {
                    Ok(sp) => out.push(sp),
                    Err(e) => tracing::warn!("packet from Java not sent: {e}"),
                }
            }
        }
    }

    /// Sets where an input event's screen packets go at once ([`Ui::send_screen_packets_now`]):
    /// the game thread's queue-and-wake closure, once the connection exists.
    pub fn set_immediate_sender(&self, sender: ImmediateSender) {
        *self.shared.send_now.lock() = Some(sender);
    }

    /// Sends what the open screen just sent from an input event at once, not at the next tick:
    /// vanilla's `MouseHandler.onPress` -> `AbstractContainerScreen.slotClicked` ->
    /// `MultiPlayerGameMode.handleContainerInput` -> `Connection.send` all run inside the
    /// event, and the crafting result the server answers with shows a full tick and more
    /// earlier. Called right after forwarding a mouse or key event to a screen; the sender
    /// (the game thread) is woken even when Java sent nothing, because the click may have
    /// lost the race with the running tick's own drain and already sit in the driver's
    /// queue, which the wake path flushes too.
    pub fn send_screen_packets_now(&self) {
        let sender = self.shared.send_now.lock().clone();
        let Some(sender) = sender else { return };
        let mut out = Vec::new();
        self.decode_java_sends(self.state(|s| s.take_outgoing()), &mut out);
        sender(&out);
    }

    fn next_salt(&self) -> i64 {
        // SplitMix64 over a counter: unpredictable enough for an unsigned message's salt
        let mut z = self
            .shared
            .salt
            .fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
            .wrapping_add(now_millis() as u64);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) as i64
    }
}
