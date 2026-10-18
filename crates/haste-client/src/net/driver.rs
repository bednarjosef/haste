// The network driver: owns the `Connection` and the `Session`, drains every queued packet
// into the world, the entity store, the session and the HUD (in order), and sends this tick's
// serverbound packets. World and entities are borrowed per tick, never owned.

use anyhow::Context;
use haste_core::input::InputState;
use haste_core::Identifier;
use haste_protocol::{
    ClientboundPlay, ConnectOptions, Connection, Login, ServerboundPlay, StatsSnapshot,
};

use super::chat_session;
use super::{EntitySink, Session, WorldSink};

/// Cap on the packets kept for a still-starting HUD (a server spamming the UI channel must not
/// grow the buffer without bound; the real backlog during JVM boot is a handful of packets).
const UI_BACKLOG_LIMIT: usize = 8192;

/// Owns the connection and the session state; world and entities are borrowed per tick.
pub struct NetDriver {
    conn: Connection,
    login: Login,
    /// Session state.
    pub session: Session,
    out: Vec<ServerboundPlay>,
    /// Packets the UI would use, seen while the HUD JVM was still starting (replayed by the
    /// game thread once the `Ui` exists; see `crate::ui::ui_handles`).
    ui_backlog: Vec<ClientboundPlay>,
    /// Signed chat: the session, the last-seen window, signing outgoing `chat`.
    pub chat: chat_session::ClientChat,
}

impl NetDriver {
    /// Connects (handshake, login, configuration) and returns in the play state.
    pub async fn connect(opts: ConnectOptions) -> anyhow::Result<Self> {
        let (conn, login) = Connection::connect(opts).await.context("connecting")?;
        // The static `minecraft:particle_type` registry by protocol id, from the same
        // tables the packet decoder resolves particle names with.
        let particle_names = std::sync::Arc::new(
            (0..)
                .map_while(|i| conn.tables().particle_name(i).map(|s| s.to_string()))
                .map(|s| Identifier::parse(&s).expect("registry names are valid identifiers"))
                .collect::<Vec<_>>(),
        );
        let mut session = Session::new(
            login.entity_id,
            login.game_mode,
            login.dimension.clone(),
            particle_names,
        );
        session.view_distance = login.view_distance;
        let chat = chat_session::ClientChat::new(conn.profile().uuid, None);
        Ok(Self {
            conn,
            login,
            session,
            chat,
            out: Vec::new(),
            ui_backlog: Vec::new(),
        })
    }

    /// The connection (registries, stats, close reason).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// What `login` said (the dimension may since have changed: `session.dimension`).
    pub fn login(&self) -> &Login {
        &self.login
    }

    /// UI-bound packets seen before the HUD JVM was up, in order; the game thread replays them
    /// into the new `Ui` once it exists so a fresh join's tab list, boss bars and scoreboard
    /// survive the boot.
    pub fn take_ui_backlog(&mut self) -> Vec<ClientboundPlay> {
        std::mem::take(&mut self.ui_backlog)
    }

    /// Connection counters.
    pub fn stats(&self) -> StatsSnapshot {
        self.conn.stats()
    }

    /// Whether the connection has closed.
    pub fn is_closed(&self) -> bool {
        self.conn.is_closed()
    }

    /// Closes the connection.
    pub fn disconnect(self) {
        self.conn.disconnect();
    }

    /// Drains every queued packet into the session, world, entities and the UI (the HUD: every
    /// packet in order, after the client's own handling), and sends the replies. Packets queued
    /// since the last drain are applied now (vanilla runs the packet queue every frame, before
    /// the tick, `Minecraft.runTick` -> `packetProcessor.processQueuedPackets`).
    pub fn receive<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        world: &mut W,
        entities: &mut E,
        ui: Option<&crate::ui::Ui>,
    ) -> anyhow::Result<()> {
        while let Some(a) = self.conn.try_recv_arrived() {
            let waited_ms = a.at.elapsed().as_secs_f64() * 1000.0;
            if tracing::enabled!(tracing::Level::DEBUG) {
                if let Some(name) = a.packet.name() {
                    if name == "minecraft:container_set_slot" {
                        tracing::debug!(wait_ms = waited_ms, "container_set_slot applied");
                    }
                }
            }
            let regs = self.conn.registries();
            // The HUD sees every packet after the client's own handling, in order; cloning only
            // when there is a UI to show it (the session takes the original). While the HUD JVM
            // is still starting the UI-bound packets are kept and replayed once it is up.
            let keep = ui.is_some() || crate::ui::ui_handles(&a.packet);
            let for_ui = keep.then(|| a.packet.clone());
            self.chat.handle(&a.packet, &mut self.out);
            self.session
                .apply(a.packet, world, entities, regs, &mut self.out);
            // The mount message at the packet's own moment (vanilla shows it while handling
            // `set_passengers`, not on the next tick).
            if self.session.take_mounted_message() {
                if let Some(ui) = ui {
                    ui.show_mount_message();
                }
            }
            match (ui, for_ui) {
                (Some(ui), Some(p)) => ui.handle(&p, &|pos| world.block_state_at(pos)),
                (None, Some(p)) if self.ui_backlog.len() < UI_BACKLOG_LIMIT => {
                    self.ui_backlog.push(p)
                }
                _ => {}
            }
        }
        self.flush()
    }

    /// Ends the tick: entity interpolation step, the local player's tick with `input` and this
    /// tick's serverbound packets.
    pub fn end_tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &mut W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        entities.tick();
        self.session.end_tick(input, world, entities, &mut self.out);
        self.flush()
    }

    /// Queues a serverbound packet; it goes out with this tick's packets.
    pub fn queue(&mut self, p: ServerboundPlay) {
        self.out.push(p);
    }

    /// One 20 Hz tick: [`Self::receive`] then [`Self::end_tick`], without a HUD.
    pub fn tick<W: WorldSink + ?Sized, E: EntitySink + ?Sized>(
        &mut self,
        input: &InputState,
        world: &mut W,
        entities: &mut E,
    ) -> anyhow::Result<()> {
        self.receive(world, entities, None)?;
        self.end_tick(input, world, entities)
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        for mut p in self.out.drain(..) {
            self.chat.pack(&mut p);
            self.conn
                .send(p)
                .map_err(|e| anyhow::anyhow!("send: {e}"))?;
        }
        Ok(())
    }
}
