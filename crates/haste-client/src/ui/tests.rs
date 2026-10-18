use super::packets::{read_var_int, FORWARDED, FORWARDED_LOCAL};
use super::*;

mod click_timing;

#[test]
fn var_ints() {
    let mut i = 0;
    assert_eq!(read_var_int(&[0xAC, 0x02, 7], &mut i), Some(300));
    assert_eq!(i, 2);
    let mut i = 0;
    assert_eq!(read_var_int(&[0x80], &mut i), None);
}

#[test]
fn forwarding_lists_are_disjoint_and_named_like_packets_json() {
    for n in FORWARDED.iter().chain(FORWARDED_LOCAL) {
        assert!(n.starts_with("minecraft:"), "{n}");
    }
    for n in FORWARDED_LOCAL {
        assert!(!FORWARDED.contains(n), "{n}");
    }
}

#[test]
fn ui_handles_tier2_state_and_forwarded_packets_only() {
    use haste_protocol::packets::play::clientbound as cb;
    // Tier-2 state (health) and a forwarded tier-3 packet (the sidebar display) are kept for
    // the HUD; a plain world packet is not (the boot backlog must stay small).
    let health = ClientboundPlay::SetHealth(cb::SetHealth {
        health: 1.0,
        food: 2,
        saturation: 0.0,
    });
    assert!(ui_handles(&health));
    let display = ClientboundPlay::SetDisplayObjective(cb::SetDisplayObjective {
        position: 1,
        name: "s".to_owned(),
    });
    assert!(ui_handles(&display));
    let block = ClientboundPlay::BlockUpdate(cb::BlockUpdate {
        pos: haste_protocol::haste_core::BlockPos::new(0, 0, 0),
        state: 1,
    });
    assert!(!ui_handles(&block));
}

// ---- tests against a server (ignored: need cache/ and the test server) ----

use haste_protocol::haste_core::gui::GuiCommand;
use haste_protocol::haste_core::DVec3;

/// The frame tests that run without a server (`leaving_the_world_hides_the_hud`,
/// `the_attack_indicator_bar_follows_the_mirrored_charge`) take turns on this lock: they share
/// the process's one JVM and its one game state, and a parallel test would otherwise flip the
/// shared state between another's write and its frame (the frame waits on the JVM's lock
/// while the other test keeps writing).
pub(crate) fn frame_tests() -> &'static std::sync::Mutex<()> {
    static SERIALISE: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    SERIALISE.get_or_init(std::sync::Mutex::default)
}

const NEEDS_SERVER: &str = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1";

/// The process's JVM (one per process), started once.
pub(crate) fn host() -> JvmHost {
    static HOST: std::sync::OnceLock<JvmHost> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
        let t = std::time::Instant::now();
        let host = JvmHost::start(haste_jvm::JvmConfig::from_cache(&cache).unwrap()).unwrap();
        eprintln!("JVM ready in {:?}: {}", t.elapsed(), host.proxy_summary());
        host
    })
    .clone()
}

/// A minimal client for the tests: joins, answers teleports, sends the per-tick movement and
/// tick-end packets (standing still), drives `Ui` like the game loop does.
struct TestClient {
    rt: tokio::runtime::Runtime,
    conn: Option<Connection>,
    ui: Ui,
    pos: Option<(DVec3, f32, f32)>,
    loaded: bool,
    tick: u64,
    received: Vec<&'static str>,
    sent: Vec<ServerboundPlay>,
    kicked: Option<Component>,
}

impl TestClient {
    fn join() -> TestClient {
        use haste_protocol::{Account, ConnectOptions};
        let addr = std::env::var("HASTE_TEST_SERVER").unwrap_or_else(|_| "127.0.0.1:25614".into());
        let ui = Ui::new(host());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .unwrap();
        let opts = ConnectOptions::new(
            addr.clone(),
            Account::Offline {
                username: "haste".into(),
            },
        );
        let (conn, login) = rt.block_on(Connection::connect(opts)).expect("connect");
        let t = std::time::Instant::now();
        ui.joined(&conn, &login, login.game_mode)
            .expect("registries into Java");
        eprintln!(
            "joined {addr}; {} synchronised registries into Java in {:?}",
            conn.registries().names().count(),
            t.elapsed()
        );
        TestClient {
            rt,
            conn: Some(conn),
            ui,
            pos: None,
            loaded: false,
            tick: 0,
            received: Vec::new(),
            sent: Vec::new(),
            kicked: None,
        }
    }

    fn conn(&mut self) -> &mut Connection {
        self.conn.as_mut().expect("connected")
    }

    fn send(&mut self, p: ServerboundPlay) {
        self.conn().send(p).unwrap();
    }

    /// `n` game ticks (50 ms each).
    fn run(&mut self, n: u64) {
        let _rt = self.rt.enter();
        for _ in 0..n {
            while let Some(p) = self.conn.as_mut().and_then(|c| c.try_recv()) {
                self.ui.handle(&p, &|_| BlockStateId::AIR);
                if let Some(n) = p.name() {
                    self.received.push(n);
                }
                match &p {
                    ClientboundPlay::PlayerPosition(t) => {
                        self.pos = Some((t.pos, t.yaw, t.pitch));
                        self.send(ServerboundPlay::AcceptTeleportation(
                            sb::AcceptTeleportation {
                                teleport_id: t.teleport_id,
                                pos: t.pos,
                                yaw: t.yaw,
                                pitch: t.pitch,
                            },
                        ));
                        if !self.loaded {
                            self.send(ServerboundPlay::PlayerLoaded(sb::PlayerLoaded {}));
                            self.loaded = true;
                        }
                    }
                    ClientboundPlay::Disconnect(d) => {
                        self.ui.disconnected(&d.reason);
                        self.kicked = Some(d.reason.clone());
                        self.conn.take();
                        return;
                    }
                    _ => {}
                }
            }
            if let (true, Some((p, yaw, pitch))) = (self.loaded, self.pos) {
                self.send(ServerboundPlay::MovePlayerPosRot(sb::MovePlayerPosRot {
                    pos: p,
                    yaw,
                    pitch,
                    flags: sb::move_flags::ON_GROUND,
                }));
                self.send(ServerboundPlay::ClientTickEnd(sb::ClientTickEnd {}));
            }
            self.ui.tick();
            let mut out = Vec::new();
            self.ui.drain_outgoing(&mut out);
            for p in out {
                self.sent.push(p.clone());
                self.send(p);
            }
            self.tick += 1;
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Runs until the player is loaded in the world.
    fn spawn(&mut self) {
        for _ in 0..200 {
            if self.loaded {
                self.run(10);
                return;
            }
            self.run(1);
        }
        panic!("no spawn teleport");
    }

    /// A command as the chat screen would send it (setup steps of the tests).
    fn command(&mut self, c: &str) {
        self.ui.gui().with_state(|s| {
            s.sent.push(haste_jvm::ChatMessage {
                text: c.into(),
                command: true,
            })
        });
    }

    fn frame(&mut self, mouse: (f64, f64)) -> haste_protocol::haste_core::gui::GuiFrame {
        self.ui.frame(320, 240, 2, mouse, 0.0).clone()
    }

    fn press(&mut self, key: i32) {
        let kc = keys::keycode(key);
        self.ui.key(key, kc, 0, true);
        self.ui.key(key, kc, 0, false);
    }

    fn type_text(&mut self, t: &str) {
        for c in t.chars() {
            self.ui.char_typed(c, 0);
        }
    }

    /// Clicks the button whose label is `label` (the widget sprite under the text).
    fn click(&mut self, label: &str) {
        let f = self.frame((0.0, 0.0));
        let (x, y) =
            text_pos(&f, label).unwrap_or_else(|| panic!("no {label:?} in {:?}", texts(&f)));
        self.ui.mouse_moved(f64::from(x) + 1.0, f64::from(y) + 1.0);
        self.ui.mouse_button(
            f64::from(x) + 1.0,
            f64::from(y) + 1.0,
            keys::MOUSE_LEFT,
            true,
        );
        self.ui.mouse_button(
            f64::from(x) + 1.0,
            f64::from(y) + 1.0,
            keys::MOUSE_LEFT,
            false,
        );
    }

    fn disconnect(mut self) {
        if let Some(c) = self.conn.take() {
            c.disconnect();
        }
        self.ui.gui().with_state(|s| s.in_world = false);
        self.ui.gui().close_screen();
    }
}

/// Vanilla code that hits a refused class fails, and inside a `CompletableFuture` it fails
/// silently: every test checks the class log. These refusals are known and harmless.
fn assert_no_refusals(host: &JvmHost) {
    const KNOWN: &[&str] = &[
        "com.mojang.renderpearl.api.pipeline.RenderPipeline", // a parameter type of draw calls
        "jakarta.servlet.Servlet",                            // log4j probes
        "javax.servlet.Servlet",
        "net.minecraft.client.sounds.MusicManager", // option callbacks (no music)
    ];
    let log = host.class_log().unwrap();
    let bad: Vec<_> = log
        .refused()
        .into_iter()
        .filter(|(n, _)| !KNOWN.contains(n))
        .collect();
    assert!(bad.is_empty(), "classes refused: {bad:#?}");
}

fn texts(f: &haste_protocol::haste_core::gui::GuiFrame) -> Vec<String> {
    f.commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Text { text, .. } | GuiCommand::StyledText { text, .. } => {
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
}

/// Where a text is drawn, in absolute GUI pixels (translations applied).
fn text_pos(f: &haste_protocol::haste_core::gui::GuiFrame, want: &str) -> Option<(i32, i32)> {
    let mut stack: Vec<(f32, f32)> = vec![(0.0, 0.0)];
    for c in &f.commands {
        match c {
            GuiCommand::TranslatePush { dx, dy, .. } => {
                let (x, y) = *stack.last().unwrap();
                stack.push((x + dx, y + dy));
            }
            GuiCommand::TranslatePop => {
                stack.pop();
            }
            GuiCommand::Text { text, x, y, .. } | GuiCommand::StyledText { text, x, y, .. }
                if text == want =>
            {
                let (dx, dy) = *stack.last().unwrap();
                return Some((x + dx as i32, y + dy as i32));
            }
            _ => {}
        }
    }
    None
}

/// Joins the test server as "haste", gives itself stone and says hello through commands (the
/// player must be an operator: `ops.json` of the test server), plays a few seconds and renders
/// the HUD: the hotbar with the stone and the chat line must be in the frame.
#[test]
#[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
fn joins_and_renders_the_hud() {
    let mut c = TestClient::join();
    c.spawn();
    c.command("clear @s");
    c.command("give @s minecraft:stone 64");
    c.command("say hello from haste");
    c.run(40);
    assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
    eprintln!(
        "tier-3 packets seen: {:?}",
        c.received
            .iter()
            .filter(|n| FORWARDED.contains(n) || FORWARDED_LOCAL.contains(n))
            .collect::<std::collections::BTreeSet<_>>()
    );
    let stone =
        c.ui.gui()
            .with_state(|s| s.slot(0).cloned())
            .expect("stone in hotbar slot 0 (from the server's container_set_slot)");
    assert_eq!(stone.count, 64);
    let t = std::time::Instant::now();
    let f = c.frame((0.0, 0.0));
    eprintln!(
        "HUD frame in {:?}: {} commands",
        t.elapsed(),
        f.commands.len()
    );
    for cmd in &f.commands {
        eprintln!("  {cmd:?}");
    }
    assert!(f
        .commands
        .iter()
        .any(|c| matches!(c, GuiCommand::Sprite { sprite, .. } if sprite.path() == "hud/hotbar")));
    assert!(f.commands.iter().any(|c| matches!(c,
        GuiCommand::Item { item, count: 64, x: 72, y: 221 } if Some(*item) == stone.item)));
    let text = texts(&f);
    assert!(
        text.iter().any(|t| t.contains("hello from haste")),
        "chat line in the HUD: {text:?}"
    );
    let chat = c.ui.gui().chat_history().unwrap();
    eprintln!("chat log: {chat:?}");
    assert!(
        chat.iter().any(|l| l == "[haste] hello from haste"),
        "{chat:?}"
    );
    let unimpl = c.ui.host.unimplemented().unwrap();
    eprintln!("Unimplemented: {unimpl:#?}");
    assert_no_refusals(&c.ui.host);
    c.disconnect();
}

/// The chat screen parses commands with the server's command tree (from its `commands`
/// packet) and asks the server for suggestions where the tree says so (`command_suggestion`
/// round trip, here the recipe names of `/recipe give <player> `).
#[test]
#[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
fn command_suggestions_from_the_server() {
    let mut c = TestClient::join();
    c.spawn();
    assert!(c.received.contains(&"minecraft:commands"));
    let names = c.ui.gui().command_names().unwrap();
    eprintln!(
        "permission level {}, {} commands: {names:?}",
        c.ui.gui().with_state(|s| s.permission_level),
        names.len()
    );
    assert!(names.iter().any(|n| n == "gamemode"), "{names:?}");
    let comp = c.ui.gui().completions("gamem").unwrap();
    eprintln!(
        "completions of 'gamem': {comp:?}; autoSuggestions {:?}",
        c.ui.gui().option("autoSuggestions")
    );
    // "/" opens the chat screen with "/" typed
    c.press(keys::SLASH);
    assert_eq!(
        c.ui.gui().screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.ChatScreen")
    );
    c.type_text("gamem");
    c.run(2);
    let f = c.frame((0.0, 0.0));
    eprintln!(
        "unimplemented: {:#?}; last error {:?}",
        c.ui.host.unimplemented().unwrap(),
        c.ui.gui().take_last_error()
    );
    let t = texts(&f);
    assert!(
        t.iter().any(|x| x == "gamemode"),
        "client-side suggestions: {t:?}"
    );
    // an argument the server completes: the screen sends command_suggestion, the answer shows
    for _ in 0.."gamem".len() {
        c.press(keys::BACKSPACE);
    }
    c.type_text("recipe give haste ");
    c.run(20);
    assert!(
        c.sent.iter().any(|p| matches!(p, ServerboundPlay::CommandSuggestion(s) if s.text.starts_with("/recipe give haste"))),
        "{:?}",
        c.sent
    );
    assert!(c.received.contains(&"minecraft:command_suggestions"));
    let t = texts(&c.frame((0.0, 0.0)));
    assert!(
        t.iter().any(|x| x.starts_with("minecraft:")),
        "server suggestions: {t:?}"
    );
    // like vanilla: the first Escape hides the suggestion list, the second closes the screen
    c.press(keys::ESCAPE);
    assert!(c.ui.screen_open());
    c.press(keys::ESCAPE);
    assert!(!c.ui.screen_open());
    assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
    assert_no_refusals(&c.ui.host);
    c.disconnect();
}

/// The server's `game_event` CHANGE_GAME_MODE (an operator's `/gamemode`) reaches the Java
/// side where vanilla stores the local mode (`MultiPlayerGameMode.setLocalMode`): the HUD
/// shows the hearts and the hunger bar in survival and drops them again in creative.
#[test]
#[ignore = "needs cache/ (JDK, client jar, libraries, ASM, data generator reports)"]
fn game_mode_change_shows_and_hides_the_vitals() {
    use haste_protocol::packets::play::clientbound as cb;
    let mut ui = Ui::new(host());
    ui.gui().with_state(|s| {
        s.in_world = true;
        s.game_mode = GameMode::Creative;
        s.vitals.health = 20.0;
        s.vitals.food = 20;
    });
    let vitals = |ui: &mut Ui| {
        let f = ui.frame(320, 240, 2, (0.0, 0.0), 0.0);
        let sprites: Vec<String> = f
            .commands
            .iter()
            .filter_map(|c| match c {
                GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                    Some(sprite.to_string())
                }
                _ => None,
            })
            .collect();
        let count = |prefix: &str| sprites.iter().filter(|s| s.starts_with(prefix)).count();
        (count("minecraft:hud/heart"), count("minecraft:hud/food"))
    };
    // creative: no health, no hunger
    assert_eq!(vitals(&mut ui), (0, 0));
    // CHANGE_GAME_MODE (event 3), the param is the GameType id: 0 = survival
    ui.handle(
        &ClientboundPlay::GameEvent(cb::GameEvent {
            event: 3,
            value: 0.0,
        }),
        &|_| BlockStateId::AIR,
    );
    assert_eq!(ui.gui().with_state(|s| s.game_mode), GameMode::Survival);
    let (hearts, food) = vitals(&mut ui);
    assert!(hearts >= 10, "hearts: {hearts}");
    assert!(food >= 10, "food: {food}");
    // and back to creative (1): the hearts and the hunger bar go away again
    ui.handle(
        &ClientboundPlay::GameEvent(cb::GameEvent {
            event: 3,
            value: 1.0,
        }),
        &|_| BlockStateId::AIR,
    );
    assert_eq!(vitals(&mut ui), (0, 0));
    // the respawn packet carries the game mode too (a respawn in another mode)
    ui.handle(
        &ClientboundPlay::Respawn(cb::Respawn {
            spawn: cb::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".parse().unwrap(),
                hashed_seed: 0,
                game_mode: 0,
                previous_game_mode: 0,
                is_debug: false,
                is_flat: false,
                death_location: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            data_kept: 0,
        }),
        &|_| BlockStateId::AIR,
    );
    let (hearts, food) = vitals(&mut ui);
    assert!(hearts >= 10, "hearts after the respawn: {hearts}");
    assert!(food >= 10, "food after the respawn: {food}");
}

/// Dying opens vanilla's death screen; its Respawn button sends `client_command` and the
/// server's respawn closes it.
#[test]
#[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
fn death_screen_and_respawn() {
    let mut c = TestClient::join();
    c.spawn();
    if c.ui.gui().with_state(|s| s.vitals.health <= 0.0) {
        // left dead by an earlier run: respawn first
        c.send(ServerboundPlay::ClientCommand(sb::ClientCommand {
            action: 0,
        }));
        c.run(20);
    }
    c.command("kill @s");
    c.run(10);
    assert_eq!(
        c.ui.gui().screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.DeathScreen"),
        "received {:?}",
        c.received
    );
    let t = texts(&c.frame((0.0, 0.0)));
    assert!(t.iter().any(|x| x == "You Died!"), "{t:?}");
    // the buttons are enabled after a second
    c.run(25);
    c.click("Respawn");
    c.run(10);
    assert!(
        c.sent
            .iter()
            .any(|p| matches!(p, ServerboundPlay::ClientCommand(x) if x.action == 0)),
        "{:?}",
        c.sent
    );
    assert!(c.received.contains(&"minecraft:respawn"));
    assert!(!c.ui.screen_open(), "{:?}", c.ui.gui().screen_class());
    assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
    let unimpl = c.ui.host.unimplemented().unwrap();
    eprintln!("Unimplemented: {unimpl:#?}");
    assert_no_refusals(&c.ui.host);
    c.disconnect();
}

/// A chest the player opens is vanilla's container screen over the server's container
/// packets; clicks are predicted and sent with hashed stacks, and the server agrees (no
/// correction of the clicked slots), Escape closes it with `container_close`.
#[test]
#[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
fn chest_screen() {
    let mut c = TestClient::join();
    c.spawn();
    let (p, _, _) = c.pos.unwrap();
    let chest = haste_protocol::haste_core::BlockPos::new(
        p.x.floor() as i32 + 2,
        p.y.floor() as i32,
        p.z.floor() as i32,
    );
    c.command(&format!(
        "setblock {} {} {} minecraft:air",
        chest.x, chest.y, chest.z
    ));
    c.command(&format!(
        "setblock {} {} {} minecraft:chest{{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:5}}]}}",
        chest.x, chest.y, chest.z
    ));
    c.run(10);
    c.send(ServerboundPlay::UseItemOn(sb::UseItemOn {
        hand: 0,
        pos: chest,
        face: 4, // west face (the player is to the west)
        cursor_x: 0.0,
        cursor_y: 0.5,
        cursor_z: 0.5,
        inside_block: false,
        world_border_hit: false,
        sequence: 1,
    }));
    c.run(10);
    assert!(
        c.received.contains(&"minecraft:open_screen"),
        "{:?}",
        c.received
    );
    assert_eq!(
        c.ui.gui().screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.inventory.ContainerScreen")
    );
    let f = c.frame((0.0, 0.0));
    let t = texts(&f);
    assert!(t.iter().any(|x| x == "Chest"), "{t:?}");
    assert!(t.iter().any(|x| x == "5"), "the diamonds' count: {t:?}");
    // pick up the diamonds: slot 0 of the chest, at (8, 18) in the 176x166 container
    let origin = ((320 - 176) / 2, (240 - 166) / 2);
    let at = (f64::from(origin.0 + 8 + 8), f64::from(origin.1 + 18 + 8));
    let corrections_before = c
        .received
        .iter()
        .filter(|n| **n == "minecraft:container_set_slot")
        .count();
    c.ui.mouse_moved(at.0, at.1);
    c.ui.mouse_button(at.0, at.1, keys::MOUSE_LEFT, true);
    c.ui.mouse_button(at.0, at.1, keys::MOUSE_LEFT, false);
    c.run(10);
    let click = c
        .sent
        .iter()
        .find_map(|p| match p {
            ServerboundPlay::ContainerClick(x) => Some(x.clone()),
            _ => None,
        })
        .expect("container_click sent");
    assert_eq!((click.slot, click.button, click.mode), (0, 0, 0));
    assert!(click.window_id > 0);
    assert_eq!(click.changed.len(), 1, "{click:?}");
    assert!(click.carried.item.is_some(), "carried diamonds: {click:?}");
    let corrections = c
        .received
        .iter()
        .filter(|n| **n == "minecraft:container_set_slot")
        .count()
        - corrections_before;
    assert_eq!(corrections, 0, "the server corrected our prediction");
    c.press(keys::ESCAPE);
    c.run(5);
    assert!(!c.ui.screen_open());
    assert!(
        c.sent.iter().any(
            |p| matches!(p, ServerboundPlay::ContainerClose(x) if x.window_id == click.window_id)
        ),
        "{:?}",
        c.sent
    );
    assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
    let unimpl = c.ui.host.unimplemented().unwrap();
    eprintln!("Unimplemented: {unimpl:#?}");
    assert_no_refusals(&c.ui.host);
    c.disconnect();
}

/// The pause menu's Advancements screen: the server's update_advancements packets feed
/// vanilla's ClientAdvancements (its tree and progress), the screen opens with the server's
/// tabs (a vanilla server sends at least the "Minecraft" story root) and asks the server for
/// the selected tab (seen_advancements).
#[test]
#[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
fn advancements_screen() {
    let mut c = TestClient::join();
    c.spawn();
    c.run(40);
    assert!(
        c.received.contains(&"minecraft:update_advancements"),
        "no update_advancements from the server: {:?}",
        c.received
    );
    c.press(keys::ESCAPE);
    c.click("Advancements");
    assert_eq!(
        c.ui.gui().screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.advancements.AdvancementsScreen"),
        "{:?}",
        c.ui.gui().take_last_error()
    );
    let f = c.frame((0.0, 0.0));
    let t = texts(&f);
    // the vanilla server's story root: the "Minecraft" tab title and the window's title
    assert!(
        t.iter().any(|x| x == "Advancements"),
        "missing title: {t:?}"
    );
    assert!(
        t.iter().any(|x| x == "Minecraft"),
        "missing the story tab: {t:?}"
    );
    // Escape closes the advancements screen back to the pause menu it opened from
    // (vanilla AdvancementsScreen.onClose -> setScreen(this.lastScreen)), and the second
    // Escape closes the pause menu (in a world, so no title-screen fallback)
    c.press(keys::ESCAPE);
    c.run(5);
    assert_eq!(
        c.ui.gui().screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.PauseScreen"),
        "the advancements screen closes back to the pause menu"
    );
    c.press(keys::ESCAPE);
    c.run(5);
    assert!(!c.ui.screen_open());
    // the screen told the server it closed the tab (vanilla sends seen_advancements)
    assert!(
        c.sent
            .iter()
            .any(|p| matches!(p, ServerboundPlay::SeenAdvancements(_))),
        "no seen_advancements: {:?}",
        c.sent
    );
    let unimpl = c.ui.host.unimplemented().unwrap();
    eprintln!("Unimplemented: {unimpl:#?}");
    assert_no_refusals(&c.ui.host);
    assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
    c.disconnect();
}

/// A kick shows vanilla's disconnect screen with the reason.
#[test]
#[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
fn kick_shows_the_disconnect_screen() {
    let mut c = TestClient::join();
    c.spawn();
    c.command("kick haste Bye from the test");
    c.run(20);
    assert!(c.kicked.is_some(), "not kicked");
    assert_eq!(
        c.ui.gui().screen_class().as_deref(),
        Some("net.minecraft.client.gui.screens.DisconnectedScreen")
    );
    let t = texts(&c.frame((0.0, 0.0)));
    assert!(t.iter().any(|x| x.contains("Bye from the test")), "{t:?}");
    c.ui.gui().close_screen();
}

/// The HUD's server-driven parts, all vanilla: effect icons, a boss bar, the sidebar
/// scoreboard, title and subtitle, the action bar, the player list while Tab is held.
#[test]
#[ignore = "needs cache/ and a test server (HASTE_TEST_SERVER, default 127.0.0.1:25614) where 'haste' is an operator; run with --test-threads=1"]
fn hud_elements_from_the_server() {
    let mut c = TestClient::join();
    c.spawn();
    for cmd in [
        "effect give @s minecraft:speed 60 1",
        "bossbar add haste:test \"Test Boss\"",
        "bossbar set haste:test players haste",
        "scoreboard objectives add hastetest dummy \"Haste Sidebar\"",
        "scoreboard objectives setdisplay sidebar hastetest",
        "scoreboard players set haste hastetest 42",
        "title @s times 10 100 10",
        "title @s subtitle \"Small subtitle\"",
        "title @s title \"Big Title\"",
        "title @s actionbar \"Action!\"",
    ] {
        c.command(cmd);
    }
    c.run(20);
    c.ui.key(keys::TAB, keys::keycode(keys::TAB), 0, true);
    let f = c.frame((0.0, 0.0));
    c.ui.key(keys::TAB, keys::keycode(keys::TAB), 0, false);
    for cmd in [
        "effect clear @s",
        "bossbar remove haste:test",
        "scoreboard objectives remove hastetest",
        "title @s clear",
    ] {
        c.command(cmd);
    }
    c.run(5);
    for cmd in &f.commands {
        eprintln!("  {cmd:?}");
    }
    let t = texts(&f);
    for want in [
        "Test Boss",
        "Haste Sidebar",
        "42",
        "Big Title",
        "Small subtitle",
        "Action!",
    ] {
        assert!(
            t.iter().any(|x| x.contains(want)),
            "missing {want:?} in {t:?}"
        );
    }
    // the player list shows our name (and the sidebar its score holder)
    assert!(
        t.iter().filter(|x| x.as_str() == "haste").count() >= 2,
        "{t:?}"
    );
    let sprites: Vec<String> = f
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                Some(sprite.to_string())
            }
            _ => None,
        })
        .collect();
    assert!(
        sprites.iter().any(|s| s.contains("effect_background")),
        "effect icon background: {sprites:?}"
    );
    assert!(
        sprites.iter().any(|s| s.contains("speed")),
        "speed effect icon: {sprites:?}"
    );
    assert!(
        sprites.iter().any(|s| s.contains("boss_bar")),
        "boss bar: {sprites:?}"
    );
    let unimpl = c.ui.host.unimplemented().unwrap();
    eprintln!("Unimplemented: {unimpl:#?}");
    assert_no_refusals(&c.ui.host);
    assert!(c.kicked.is_none(), "kicked: {:?}", c.kicked);
    c.disconnect();
}

/// Leaving the server stops the HUD like vanilla `Minecraft.disconnectClient` (the pause
/// screen's "Disconnect"): after `left_world` the title screen draws without the hotbar,
/// hearts or crosshair, over the plain menu background instead of the last world frame.
///
/// Starts the JVM from cache/ (the client jar under it); like the other cache-bound tests,
/// it skips (no ignore: CI has the cache) when the jar is absent.
#[test]
fn leaving_the_world_hides_the_hud() {
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if !cache
        .join("versions")
        .join(haste_core::paths::MC_VERSION)
        .join("client.jar")
        .is_file()
    {
        eprintln!("skipped: no cache/");
        return;
    }
    let _serial = frame_tests().lock();
    let mut ui = Ui::new(host());
    ui.gui().with_state(|s| {
        s.in_world = true;
        s.game_mode = GameMode::Survival;
        s.vitals.health = 20.0;
        s.vitals.food = 20;
    });
    let in_world = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
    let sprites: Vec<String> = in_world
        .commands
        .iter()
        .filter_map(|c| match c {
            GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                Some(sprite.to_string())
            }
            _ => None,
        })
        .collect();
    assert!(
        sprites
            .iter()
            .any(|s| s.starts_with("minecraft:hud/hotbar")),
        "hotbar while in the world: {sprites:?}"
    );
    // the window's `leave_server` path
    ui.left_world();
    let left = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
    assert!(
        !left.commands.iter().any(|c| match c {
            GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                sprite.path().starts_with("hud/")
            }
            _ => false,
        }),
        "no HUD sprites after leaving: {:?}",
        texts(&left)
    );
}

/// The attack indicator follows the mirrored attack charge (vanilla `Hud.extractCrosshair`):
/// while the charge is below 1 the bar under the crosshair draws (`crosshair_attack_indicator_
/// background` + `_progress`, the progress 17 * scale pixels wide), and at a full charge
/// neither sprite draws.
///
/// Starts the JVM from cache/ (the client jar under it); like the other cache-bound tests,
/// it skips (no ignore: CI has the cache) when the jar is absent.
#[test]
fn the_attack_indicator_bar_follows_the_mirrored_charge() {
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
    if !cache
        .join("versions")
        .join(haste_core::paths::MC_VERSION)
        .join("client.jar")
        .is_file()
    {
        eprintln!("skipped: no cache/");
        return;
    }
    let _serial = frame_tests().lock();
    let mut ui = Ui::new(host());
    let attack_sprites = |f: &GuiFrame| -> Vec<String> {
        f.commands
            .iter()
            .filter_map(|c| match c {
                GuiCommand::Sprite { sprite, .. } | GuiCommand::SpriteRegion { sprite, .. } => {
                    if sprite.path().starts_with("hud/crosshair_attack_indicator") {
                        Some(sprite.path().to_owned())
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect()
    };
    // Mid-charge: two of the bare hand's five ticks recharged.
    ui.gui().with_state(|s| {
        s.in_world = true;
        s.game_mode = GameMode::Survival;
        s.vitals.health = 20.0;
        s.attack_strength = haste_jvm::AttackStrengthHud {
            ticker: 2.0,
            delay: 5.0,
        };
    });
    let bar = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
    assert_eq!(
        attack_sprites(&bar),
        vec![
            "hud/crosshair_attack_indicator_background",
            "hud/crosshair_attack_indicator_progress",
        ],
        "recharge bar while charging: {:?}",
        texts(&bar)
    );
    // The progress sprite is (int)(scale * 17) = 6 of the 17 pixels wide.
    let progress = bar.commands.iter().find_map(|c| match c {
        GuiCommand::SpriteRegion { sprite, w, .. }
            if sprite.path() == "hud/crosshair_attack_indicator_progress" =>
        {
            Some(*w)
        }
        _ => None,
    });
    assert_eq!(progress, Some(6), "progress width");
    // Fully charged: no indicator at all (the full icon needs a picked entity, step 2).
    ui.gui().with_state(|s| {
        s.attack_strength = haste_jvm::AttackStrengthHud {
            ticker: 5.0,
            delay: 5.0,
        };
    });
    let full = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
    assert!(
        attack_sprites(&full).is_empty(),
        "no indicator at full charge: {:?}",
        texts(&full)
    );
    // The full icon: a living, alive entity under the crosshair and a slow item (a sword's
    // 12.5 ticks > vanilla Hud.MIN_CROSSHAIR_ATTACK_SPEED's 5).
    ui.gui().with_state(|s| {
        s.attack_strength = haste_jvm::AttackStrengthHud {
            ticker: 12.5,
            delay: 12.5,
        };
        s.crosshair_pick_living = true;
    });
    let icon = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
    assert_eq!(
        attack_sprites(&icon),
        vec!["hud/crosshair_attack_indicator_full"],
        "full icon on a picked living entity: {:?}",
        texts(&icon)
    );
    // A fist (delay exactly 5) shows no full icon even on a mob: vanilla requires
    // delay > MIN_CROSSHAIR_ATTACK_SPEED.
    ui.gui().with_state(|s| {
        s.attack_strength = haste_jvm::AttackStrengthHud {
            ticker: 5.0,
            delay: 5.0,
        };
    });
    let fist = ui.frame(320, 240, 2, (0.0, 0.0), 0.0).clone();
    assert!(
        attack_sprites(&fist).is_empty(),
        "no full icon for the bare hand: {:?}",
        texts(&fist)
    );
}
