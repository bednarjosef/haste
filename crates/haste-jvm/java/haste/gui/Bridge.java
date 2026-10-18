package haste.gui;

import haste.rt.GuiBatch;
import net.minecraft.client.Options;
import net.minecraft.client.gui.screens.ChatScreen;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.gui.screens.inventory.InventoryScreen;
import net.minecraft.client.gui.screens.multiplayer.JoinMultiplayerScreen;
import net.minecraft.client.gui.screens.options.OptionsScreen;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.input.MouseButtonInfo;

/**
 * Game-side entry point. Rust holds the single instance and calls these methods (one JNI call each); every
 * method runs vanilla screen code with the 26.3 signatures.
 */
public final class Bridge {
    public static final int CHAT = 0, PAUSE = 1, INVENTORY = 2, COMMAND = 3, TITLE = 4, OPTIONS = 5, MULTIPLAYER = 6, SELECT_WORLD = 7, PACKS = 8;

    private int pressedButton = -1;
    private double lastX, lastY;

    private Bridge() {}

    /** How long vanilla's bootstrap took (ms). */
    public static long bootstrapMillis;

    public static Bridge create() {
        // like vanilla's startup: version.json from the player's jar (resources come through HasteClassLoader)
        net.minecraft.SharedConstants.tryDetectVersion();
        // vanilla's bootstrap fills every built-in registry from the player's jar (blocks, items, entity types,
        // data component types, ...): the registry data tier-3 code looks up. See NOTES.md "Registries".
        long t = System.nanoTime();
        net.minecraft.server.Bootstrap.bootStrap();
        bootstrapMillis = (System.nanoTime() - t) / 1_000_000;
        // vanilla's GUI code passes RenderPipelines fields to the extractor: Haste's own copies of the ones the
        // GUI uses, so GfxImpl reads the blend mode from the pipeline like vanilla does
        Pipelines.install();
        GfxImpl.width = 320;
        GfxImpl.height = 240;
        Proxies.create();
        ClientImpl.running = true;
        // the saved language (options.txt "lang") loads before the first frame, like vanilla's startup reload
        ClientImpl.getLanguageManager(Proxies.minecraft);
        return new Bridge();
    }

    /**
     * Opens one of the supported vanilla screens (replacing the current one). The options screen's
     * parent is whatever screen was open (vanilla opens it from the title screen) and the multiplayer
     * screen's parent is a title screen, as in vanilla.
     */
    public void open(int kind) {
        Screen s = switch (kind) {
            case CHAT -> new ChatScreen("", false);
            case COMMAND -> new ChatScreen("/", false);
            case PAUSE -> new PauseScreen(true);
            case INVENTORY -> new InventoryScreen(Proxies.player);
            case TITLE -> new TitleScreen();
            case OPTIONS -> new OptionsScreen(ClientImpl.screen, Proxies.options);
            case PACKS -> {
                // the parent this pack screen returns to on Done (vanilla: the OptionsScreen
                // that opened it; captured before the pack screen becomes the current one)
                Screen parent = ClientImpl.screen;
                yield new net.minecraft.client.gui.screens.packs.PackSelectionScreen(
                    ClientImpl.getResourcePackRepository(Proxies.minecraft),
                    // OptionsScreen.applyPacks: save the selection, reload, back to the opener
                    repo -> {
                        Proxies.options.updateResourcePacks(repo);
                        Proxies.gui.setScreen(parent);
                    },
                    ClientImpl.getResourcePackDirectory(Proxies.minecraft),
                    net.minecraft.network.chat.Component.translatable("resourcePack.title"));
            }
            case MULTIPLAYER -> new JoinMultiplayerScreen(new TitleScreen());
            case SELECT_WORLD -> new net.minecraft.client.gui.screens.worldselection.SelectWorldScreen(new TitleScreen());
            default -> throw new IllegalArgumentException("screen kind " + kind);
        };
        try {
            Proxies.gui.setScreen(s);
            OptionBridge.releaseAll();
        } catch (Throwable t) {
            // Gui.setScreen leaves the new screen set even when its init throws: a screen that could not
            // open must not stay half-open (the next open would call removed() on it, and options
            // screens save on that). Drop it and report the failure to Rust.
            ClientImpl.screen = null;
            if (t instanceof RuntimeException e) throw e;
            if (t instanceof Error e) throw e;
            throw new RuntimeException(t);
        }
    }

    public void close() {
        Proxies.gui.setScreen(null);
        OptionBridge.restoreToggles();
    }

    /**
     * Drops everything a test (or a disconnecting client) leaves behind: the chat log and its scroll
     * position (the history stays), the held mouse button and its position, the screen currently open
     * and whether vanilla code still believes the mouse is grabbed. The FOV option goes back to its
     * default (an options.txt a real run saved in the game directory would otherwise leak in).
     */
    public void resetForTest() {
        this.pressedButton = -1;
        this.lastX = 0.0;
        this.lastY = 0.0;
        MouseImpl.pressed = -1;
        MouseImpl.x = 0.0;
        MouseImpl.y = 0.0;
        Proxies.hud.getChat().clearMessages(false);
        Proxies.hud.getChat().resetChatScroll();
        // not Gui.setScreen(null) (which would open the title screen outside a world, like
        // vanilla): the point here is no screen at all
        if (ClientImpl.screen != null) ClientImpl.screen.removed();
        ClientImpl.screen = null;
        MouseImpl.grabbed = false;
        Proxies.options.fov().set(70);
    }

    public boolean isOpen() { return ClientImpl.screen != null; }

    /** Name of the open screen's class (tests), or null. */
    public String screenName() { return ClientImpl.screen == null ? null : ClientImpl.screen.getClass().getName(); }

    /**
     * One frame: the vanilla HUD (when in a world) and then the open screen on top, like the game, into the batch.
     * Returns {int[] commands, String[] strings}.
     */
    public Object[] render(int width, int height, int scale, double mouseX, double mouseY, float partial) {
        ClientImpl.scale = scale;
        Mirror.update();
        Screen s = ClientImpl.screen;
        if (s != null && (width != GfxImpl.width || height != GfxImpl.height)) {
            GfxImpl.width = width; GfxImpl.height = height;
            s.resize(width, height);
        }
        GfxImpl.begin(width, height);
        if (Mirror.inWorld()) {
            Delta.frame(partial);
            Proxies.hud.extractRenderState(Proxies.graphics, Delta.INSTANCE);
            GfxImpl.POSE.reset();
        }
        s = ClientImpl.screen;
        if (s != null) s.extractRenderStateWithTooltipAndSubtitles(Proxies.graphics, (int) mouseX, (int) mouseY, partial);
        // vanilla Gui.extractRenderState draws the F3 debug overlay after the screen (unless the debug
        // options screen is up, which draws its own copy); it is the topmost HUD element.
        if (!(s instanceof net.minecraft.client.gui.screens.debug.DebugOptionsScreen)) {
            Proxies.hud.extractDebugOverlay(Proxies.graphics);
        }
        GfxImpl.end();
        return GuiBatch.frame(GfxImpl.cursor, GfxImpl.blurAfter);
    }

    /** One 20 Hz client tick: the HUD (chat fade, titles, overlay message), the chat queue and the open screen. */
    public void tick() {
        Mirror.update();
        OptionBridge.tick();
        if (Mirror.inWorld()) {
            // a dead player (killed, or joined while dead) sees the death screen; with immediate
            // respawn on, vanilla's Gui.setScreen(null) calls player.respawn() instead
            // (vanilla: Gui.tick -> setScreen(null) -> isDeadOrDying -> shouldShowDeathScreen)
            if (Mirror.health() <= 0f && !(ClientImpl.screen instanceof net.minecraft.client.gui.screens.DeathScreen)) {
                if (Mirror.flag(Mirror.F_IMMEDIATE_RESPAWN)) {
                    Proxies.player.respawn();
                } else {
                    Proxies.gui.setScreen(new net.minecraft.client.gui.screens.DeathScreen(null, Mirror.flag(Mirror.F_HARDCORE), Proxies.player));
                }
            }
            Proxies.hud.tick(false);
            Proxies.chatListener.tick();
            PlayerImpl.tickEffects();
        }
        if (ClientImpl.screen != null) ClientImpl.screen.tick();
        InventorySync.sync();
    }

    /** The local player as the server introduced it (login packet, game profile). */
    public void setPlayer(int entityId, String name, long uuidMost, long uuidLeast) {
        Handlers.localEntityId = entityId;
        Proxies.set(Proxies.player, "id", int.class, entityId);
        java.util.UUID uuid = new java.util.UUID(uuidMost, uuidLeast);
        Proxies.set(Proxies.player, "uuid", java.util.UUID.class, uuid);
        Proxies.set(Proxies.player, "stringUUID", String.class, uuid.toString());
        PlayerImpl.profile = new com.mojang.authlib.GameProfile(uuid, name);
        // Player's own field (its constructor would set it; the proxy's is null without this):
        // `Player.getName` reads it, e.g. the book's sign screen names the author.
        Proxies.set(Proxies.player, "gameProfile", com.mojang.authlib.GameProfile.class, PlayerImpl.profile);
    }

    /** The joined world's dimension type (its id in the synchronised dimension_type registry). */
    public void setDimensionType(int id) { LevelImpl.dimensionTypeId = id; }

    /** The server's enabled feature flags (configuration phase). */
    public void setFeatures(String[] ids) {
        java.util.List<net.minecraft.resources.Identifier> l = new java.util.ArrayList<>();
        for (String id : ids) l.add(net.minecraft.resources.Identifier.parse(id));
        LevelImpl.features = net.minecraft.world.flag.FeatureFlags.REGISTRY.fromNames(l, id -> {});
    }

    /**
     * The pause-menu server links (configuration phase), one string per link:
     * {@code "<b|c>\t<label>\t<url>"} (b = built-in label id, c = custom label text — flattened;
     * the wire carries a component). Built like ClientCommonPacketListenerImpl.handleServerLinks:
     * untrusted strings are parsed as URIs, invalid ones dropped.
     */
    public void setServerLinks(String[] links) {
        java.util.List<net.minecraft.server.ServerLinks.Entry> entries = new java.util.ArrayList<>();
        for (String l : links) {
            String[] parts = l.split("\t", 3);
            if (parts.length != 3) continue;
            try {
                java.net.URI uri = new java.net.URI(parts[2]);
                com.mojang.datafixers.util.Either<net.minecraft.server.ServerLinks.KnownLinkType, net.minecraft.network.chat.Component> type;
                if (parts[0].equals("b")) {
                    int id = Integer.parseInt(parts[1]);
                    net.minecraft.server.ServerLinks.KnownLinkType[] all = net.minecraft.server.ServerLinks.KnownLinkType.values();
                    // continuous ids (ByIdMap.ZERO out-of-bounds strategy): index, clamped to the first
                    net.minecraft.server.ServerLinks.KnownLinkType known = all[Math.floorMod(id, all.length)];
                    type = com.mojang.datafixers.util.Either.left(known);
                } else {
                    type = com.mojang.datafixers.util.Either.right(net.minecraft.network.chat.Component.literal(parts[1]));
                }
                entries.add(new net.minecraft.server.ServerLinks.Entry(type, uri));
            } catch (java.net.URISyntaxException | NumberFormatException e) {
                // like vanilla: an invalid link is dropped with a warning
            }
        }
        ConnectionImpl.SERVER_LINKS = new net.minecraft.server.ServerLinks(java.util.List.copyOf(entries));
    }

    /** Tab held: the player list overlay is shown while the key is down (vanilla's tab list). */
    public void setTabListVisible(boolean visible) {
        Proxies.options.keyPlayerList.setDown(visible);
        Proxies.hud.getTabList().setVisible(visible);
    }

    /**
     * The player started riding (vanilla ClientPacketListener.handleSetEntityPassengersPacket, on
     * the foot-to-riding transition): the "mount.onboard" overlay message with the bound sneak
     * key's name, exactly the component vanilla builds. No cushion seating exists in Haste, so
     * "mount.sit" never applies.
     */
    public void showMountMessage() {
        net.minecraft.network.chat.Component shiftKey = Proxies.options.keyShift.getTranslatedKeyMessage();
        Proxies.hud.setOverlayMessage(net.minecraft.network.chat.Component.translatable("mount.onboard", shiftKey), false);
    }

    /**
     * The connection was lost or the server kicked us: vanilla's disconnect screen with the reason (a text component
     * in its JSON form). In menu mode the open screen (the connecting screen's parent, e.g. the server list) is the
     * parent, so the screen's button returns there like vanilla's ConnectScreen failure path.
     */
    public void disconnected(String reasonJson) {
        net.minecraft.network.chat.Component reason = net.minecraft.network.chat.ComponentSerialization.CODEC
            .parse(com.mojang.serialization.JsonOps.INSTANCE, com.google.gson.JsonParser.parseString(reasonJson))
            .result().orElse(net.minecraft.network.chat.Component.literal(reasonJson));
        Screen parent = Mirror.inWorld() ? null
            : ClientImpl.screen instanceof net.minecraft.client.gui.screens.ConnectScreen c ? c.parent
            : ClientImpl.screen;
        Proxies.gui.setScreen(new net.minecraft.client.gui.screens.DisconnectedScreen(parent,
            net.minecraft.network.chat.Component.translatable("disconnect.lost"), reason));
    }

    /** F1: hides or shows the HUD (vanilla's toggle). */
    public void toggleHud() { Proxies.hud.toggle(); }

    public boolean key(int key, int keycode, int mods, boolean pressed) {
        ClientImpl.mods = mods;
        Screen s = ClientImpl.screen;
        if (s == null) return false;
        KeyEvent e = new KeyEvent(key, keycode, mods);
        boolean r = pressed ? s.keyPressed(e) : s.keyReleased(e);
        InventorySync.sync();
        return r;
    }

    public boolean charTyped(int codepoint, int mods) {
        Screen s = ClientImpl.screen;
        if (s == null) return false;
        return s.charTyped(new CharacterEvent(codepoint));
    }

    /** Mouse move (button < 0) or press/release of {@code button} (vanilla numbering: 1 left, 2 middle, 3 right). */
    public boolean mouse(double x, double y, int button, boolean pressed) {
        Screen s = ClientImpl.screen;
        if (s == null) return false;
        double dx = x - lastX, dy = y - lastY;
        lastX = x; lastY = y;
        MouseImpl.x = x; MouseImpl.y = y;
        ClientImpl.lastInput = net.minecraft.client.InputType.MOUSE;
        if (button < 0) {
            s.mouseMoved(x, y);
            if (pressedButton >= 0) return s.mouseDragged(new MouseButtonEvent(x, y, new MouseButtonInfo(pressedButton, ClientImpl.mods)), dx, dy);
            return false;
        }
        MouseButtonEvent e = new MouseButtonEvent(x, y, new MouseButtonInfo(button, ClientImpl.mods));
        MouseImpl.pressed = pressed ? button : -1;
        boolean r;
        if (pressed) {
            pressedButton = button;
            r = s.mouseClicked(e, false);
        } else {
            pressedButton = -1;
            r = s.mouseReleased(e);
        }
        InventorySync.sync();
        return r;
    }

    public boolean scroll(double x, double y, double dx, double dy) {
        Screen s = ClientImpl.screen;
        return s != null && s.mouseScrolled(x, y, dx, dy);
    }

    /** Whether a screen asked for the mouse to be grabbed (e.g. "Back to Game") and not released since. */
    public boolean mouseGrabbed() { return MouseImpl.grabbed; }

    // ---- game input through the KeyMappings (haste.gui.OptionBridge) ----

    /** A game key event (SDL3 scancode) mirrored into the KeyMappings; no screen may be open. */
    public void gameKey(int scancode, boolean pressed) { OptionBridge.gameKey(scancode, pressed); }

    /** A game mouse button (vanilla numbering) mirrored into the KeyMappings. */
    public void gameMouse(int button, boolean pressed) { OptionBridge.gameMouse(button, pressed); }

    /** The down state of the movement and action KeyMappings as a bitmask (OptionBridge.B_*). */
    public int gameInput() { return OptionBridge.gameInput(); }

    /** Clicks of the action KeyMappings not yet consumed, as a bitmask (OptionBridge.C_*). */
    public int consumeClicks() { return OptionBridge.consumeClicks(); }

    /** Rust grabbed/released the mouse itself (e.g. on opening a screen). */
    public void setMouseGrabbed(boolean g) { MouseImpl.grabbed = g; }

    // ---- registries and packets (vanilla wire format, see Registries and Packets) ----

    /** A configuration-phase {@code registry_data} packet body. */
    public void registryData(byte[] body) { Registries.registryData(body); }

    /** An {@code update_tags} packet body; {@code play}: received in the play phase (applied at once). */
    public void tags(byte[] body, boolean play) { Registries.tags(body, play); }

    /** End of the configuration phase: builds the synchronised registries. */
    public void finishConfiguration() { Registries.finish(); InventorySync.reset(); }

    /** Synchronised registries from the vanilla data pack in the client jar (no server; tests). */
    public void loadJarRegistries() throws java.io.IOException { Registries.loadFromJar(); InventorySync.reset(); }

    /** A clientbound play packet (VarInt id + body) whose state lives in tier 3. */
    public void packet(byte[] idAndBody) {
        Packets.receive(idAndBody);
        InventorySync.sync();
    }

    /** Sets a player inventory slot (vanilla numbering) from a stack in vanilla's network encoding. */
    public void setInventorySlot(int slot, byte[] stack) {
        Proxies.inventory.setItem(slot, InventorySync.decode(stack));
        InventorySync.sync();
    }

    /** The chat log as plain text, oldest first (vanilla's ChatComponent; headless clients and tests). */
    @SuppressWarnings("unchecked")
    public String chatHistory() {
        try {
            var f = java.lang.invoke.MethodHandles.privateLookupIn(net.minecraft.client.gui.components.ChatComponent.class,
                java.lang.invoke.MethodHandles.lookup()).findVarHandle(net.minecraft.client.gui.components.ChatComponent.class,
                "allMessages", java.util.List.class);
            var all = (java.util.List<net.minecraft.client.multiplayer.chat.GuiMessage>) f.get(Proxies.hud.getChat());
            StringBuilder sb = new StringBuilder();
            for (int i = all.size() - 1; i >= 0; i--) sb.append(all.get(i).content().getString()).append('\n');
            return sb.toString();
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    /** A use opened the command block editor (vanilla `LocalPlayer.openCommandBlock`). */
    public void openCommandBlock(int x, int y, int z) {
        Handlers.openCommandBlock(x, y, z);
    }

    /** The server forgot a chunk: its block entities leave the Java-side store (vanilla unloads them with the chunk). */
    public void dropChunkBlockEntities(int chunkX, int chunkZ) {
        BlockEntities.dropChunk(chunkX, chunkZ);
    }

    /** Number of block entities in the Java-side store (tests: chunk drops). */
    public int blockEntityCount() {
        return BlockEntities.count();
    }

    /** A use opened the writable book's editor (vanilla `LocalPlayer.openItemGui`). */
    public void openItemGui(int hand) {
        Handlers.openItemGui(hand);
    }

    /** Names of the server's commands the local player may use (the root of the command tree), one per line. */
    public String commandNames() {        var src = ConnectionImpl.getSuggestionsProvider(Proxies.connection);
        StringBuilder sb = new StringBuilder();
        for (var n : Handlers.commands.getRoot().getChildren()) {
            if (n.canUse(src)) sb.append(n.getName()).append('\n');
        }
        return sb.toString();
    }

    /** Debug: the open chat screen's suggestions: calls updateCommandInfo and showSuggestions directly. */
    public String debugSuggestions() {
        try {
            Screen sc = ClientImpl.screen;
            var f = java.lang.invoke.MethodHandles.privateLookupIn(ChatScreen.class, java.lang.invoke.MethodHandles.lookup())
                .findVarHandle(ChatScreen.class, "commandSuggestions", net.minecraft.client.gui.components.CommandSuggestions.class);
            var cs = (net.minecraft.client.gui.components.CommandSuggestions) f.get(sc);
            cs.updateCommandInfo();
            cs.showSuggestions(false);
            return "visible=" + cs.isVisible();
        } catch (Throwable t) {
            return haste.rt.Errors.describe(t);
        }
    }

    /** The value of a vanilla option by its accessor name (e.g. "autoSuggestions"), as a string (tests). */
    public String option(String name) throws ReflectiveOperationException {
        Object o = Options.class.getMethod(name).invoke(Proxies.options);
        return String.valueOf(((net.minecraft.client.OptionInstance<?>) o).get());
    }

    /** Brigadier's completions of a command line (without "/") with the server's tree, one per line (tests). */
    public String completions(String line) {
        var src = ConnectionImpl.getSuggestionsProvider(Proxies.connection);
        var parse = Handlers.commands.parse(line, src);
        var s = Handlers.commands.getCompletionSuggestions(parse).join();
        StringBuilder sb = new StringBuilder();
        for (var x : s.getList()) sb.append(x.getText()).append('\n');
        return sb.toString();
    }

    /** "registry=size" lines of every registry Java sees (tests). */
    public String registrySizes() { return Registries.sizes(); }

    /** Name of an entry by registry and numeric id (tests), or null. */
    public String registryName(String registry, int id) { return Registries.name(registry, id); }

    /** Rust installed new text metrics: drop cached glyph widths. */
    public void metricsChanged() {
        Text.invalidateWidths();
        Proxies.set(Proxies.font, "lineHeight", int.class, haste.rt.Native.lineHeight());
    }

    /** Debug (tests): where this frame's blend modes came from — the pipeline and name counts, then the
     * multiply/invert blits' pipelines as "{@code <pipeline location> <code> <count>}" lines. */
    public String blendDebug() {
        return "pipeline=" + GfxImpl.pipelineBlits + " name=" + GfxImpl.nameBlits + "\n" + GfxImpl.specialBlends();
    }
}
