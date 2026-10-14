package haste.gui;

import haste.rt.Native;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.core.RegistryAccess;

/** Bodies of the {@code ClientPacketListener} proxy: the connection lives in Rust (protocol). */
public final class ConnectionImpl {
    private ConnectionImpl() {}

    /** Vanilla's built-in registries plus the server's synchronised ones (see {@link Registries}). */
    public static RegistryAccess.Frozen registryAccess(ClientPacketListener c) { return Registries.access(); }

    /**
     * The hash of a data component in container clicks: CRC32C of the value encoded with vanilla's registry-aware
     * {@code HashOps} (the server computes the same over the same registries), cached per component value like a
     * client must (stacks are hashed on every click).
     */
    private static final java.util.Map<net.minecraft.core.component.TypedDataComponent<?>, Integer> HASHES = new java.util.HashMap<>();
    private static net.minecraft.core.RegistryAccess hashedWith;
    public static net.minecraft.network.HashedPatchMap.HashGenerator decoratedHashOpsGenenerator(ClientPacketListener c) {
        return typed -> {
            if (hashedWith != Registries.access()) { HASHES.clear(); hashedWith = Registries.access(); }
            return HASHES.computeIfAbsent(typed, t -> t.encodeValue(
                net.minecraft.resources.RegistryOps.create(net.minecraft.util.HashOps.CRC32C_INSTANCE, Registries.access()))
                .getOrThrow().asInt());
        };
    }

    /** The chat log vanilla keeps for player reports: kept (no reporting UI uses it yet). */
    static final net.minecraft.client.multiplayer.chat.ChatLog CHAT_LOG = new net.minecraft.client.multiplayer.chat.ChatLog(1024);
    public static net.minecraft.client.multiplayer.chat.ChatLog chatLog(net.minecraft.client.multiplayer.chat.report.ReportingContext r) { return CHAT_LOG; }

    /**
     * The pause screen's Disconnect / "Save and Quit" goes through the report draft check
     * (vanilla: a draft asks whether to discard it first). Haste has no report drafts, so the
     * action runs at once, like vanilla's no-draft path.
     */
    public static void draftReportHandled(net.minecraft.client.multiplayer.chat.report.ReportingContext r,
        net.minecraft.client.Minecraft minecraft, net.minecraft.client.gui.screens.Screen lastScreen,
        Runnable onDiscard, boolean quitToTitle) {
        onDiscard.run();
    }

    public static net.minecraft.world.flag.FeatureFlagSet enabledFeatures(ClientPacketListener c) { return LevelImpl.features; }

    /**
     * Whether the joined server enables the given flags (vanilla: {@code ClientPacketListener.isFeatureEnabled},
     * a subset test over the configuration phase's {@code update_enabled_features} set). Vanilla's kept
     * game-mode bodies ask this before sending a creative slot's item ({@code handleCreativeModeItemAdd/Drop}).
     */
    public static boolean isFeatureEnabled(ClientPacketListener c, net.minecraft.world.flag.FeatureFlagSet required) {
        return required.isSubsetOf(enabledFeatures(c));
    }
    /** Players seen this session (social interactions screen): not tracked yet. */
    public static java.util.Map<java.util.UUID, net.minecraft.client.multiplayer.PlayerInfo> getSeenPlayers(ClientPacketListener c) { return java.util.Map.of(); }

    /** Waypoints (the locator bar) are not forwarded yet: vanilla's manager, empty. */
    static final net.minecraft.client.waypoints.ClientWaypointManager WAYPOINTS = new net.minecraft.client.waypoints.ClientWaypointManager();
    public static net.minecraft.client.waypoints.ClientWaypointManager getWaypointManager(ClientPacketListener c) { return WAYPOINTS; }

    /** Signed-chat bookkeeping (last-seen acknowledgements) is Rust's: nothing to do here. */
    public static void markMessageAsProcessed(ClientPacketListener c, net.minecraft.network.chat.MessageSignature s, boolean acknowledged) {}
    /** Offline-mode servers are what Haste tests against; online mode is not told to Java yet. */
    public static boolean onlineMode(ClientPacketListener c) { return false; }

    /** Serverbound packets from vanilla code: encoded with vanilla's codec and sent by Rust. */
    public static void send(net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl c, net.minecraft.network.protocol.Packet<?> p) { Packets.send(p); }

    /** Server links (custom pause-menu links) are not received by Rust yet: none. */
    public static net.minecraft.server.ServerLinks serverLinks(net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl c) {
        return net.minecraft.server.ServerLinks.EMPTY;
    }

    private static net.minecraft.client.multiplayer.ClientSuggestionProvider suggestions;

    /**
     * Vanilla's real suggestion provider over this connection. Permissions: none until Rust forwards the
     * server's permission level (approximate).
     */
    public static net.minecraft.client.multiplayer.ClientSuggestionProvider getSuggestionsProvider(ClientPacketListener c) {
        if (suggestions == null) {
            suggestions = new net.minecraft.client.multiplayer.ClientSuggestionProvider(c, Proxies.minecraft,
                PlayerImpl.permissions(Proxies.player));
        }
        return suggestions;
    }

    /** The server's command tree (from its commands packet, see {@link Handlers#handleCommands}). */
    public static com.mojang.brigadier.CommandDispatcher<net.minecraft.client.multiplayer.ClientSuggestionProvider> getCommands(ClientPacketListener c) {
        return Handlers.commands;
    }

    /** The scoreboard is the client's own (scoreboard packets, see {@link Handlers}). */
    public static net.minecraft.world.scores.Scoreboard scoreboard(ClientPacketListener c) { return Handlers.SCOREBOARD; }

    /** Chat message from a screen: handed to Rust, which signs/sends it. */
    public static void sendChat(ClientPacketListener c, String message) { Native.sendChat(message, false); }

    /** Command (without the leading slash). */
    public static void sendCommand(ClientPacketListener c, String command) { Native.sendChat(command, true); }

    /**
     * The settings an option change must tell the server (vanilla re-sends its client information). Haste's own
     * settings already went to the server at configuration time, and they are Rust's, not the vanilla options
     * the screens edit: nothing to send yet.
     */
    public static void broadcastClientInformation(ClientPacketListener c, net.minecraft.server.level.ClientInformation information) {}

    /**
     * The creative inventory's search trees: vanilla's own object (tier 3, plain data holder). The screen
     * fills it itself: the creative screen's constructor and per-tick tab refresh call
     * {@code updateCreativeTooltips} and {@code updateCreativeTags} (vanilla:
     * {@code CreativeModeInventoryScreen.tryRebuildTabContents}), and a play-phase {@code update_tags}
     * rebuilds the tag tree (vanilla: {@code ClientPacketListener.handleUpdateTags}, see
     * {@link Registries#tags}); both build on vanilla's background executor, off the render thread.
     */
    public static net.minecraft.client.multiplayer.SessionSearchTrees searchTrees(ClientPacketListener c) {
        return SEARCH_TREES;
    }
    static final net.minecraft.client.multiplayer.SessionSearchTrees SEARCH_TREES = new net.minecraft.client.multiplayer.SessionSearchTrees();
}
