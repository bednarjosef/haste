package haste.gui;

import haste.rt.Native;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.core.RegistryAccess;

/** Bodies of the {@code ClientPacketListener} proxy: the connection lives in Rust (WS1). */
public final class ConnectionImpl {
    private ConnectionImpl() {}

    /** Synchronised registries are tier-1 data in Rust; Java sees none yet (lookups come back empty). */
    public static RegistryAccess.Frozen registryAccess(ClientPacketListener c) { return EmptyRegistries.INSTANCE; }

    /** Server links (custom pause-menu links) are not received by Rust yet: none. */
    public static net.minecraft.server.ServerLinks serverLinks(net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl c) {
        return net.minecraft.server.ServerLinks.EMPTY;
    }

    private static net.minecraft.client.multiplayer.ClientSuggestionProvider suggestions;
    private static final com.mojang.brigadier.CommandDispatcher<net.minecraft.client.multiplayer.ClientSuggestionProvider> COMMANDS =
        new com.mojang.brigadier.CommandDispatcher<>();

    /**
     * Vanilla's real suggestion provider over this connection. Permissions: none until Rust forwards the
     * server's permission level (approximate).
     */
    public static net.minecraft.client.multiplayer.ClientSuggestionProvider getSuggestionsProvider(ClientPacketListener c) {
        if (suggestions == null) {
            suggestions = new net.minecraft.client.multiplayer.ClientSuggestionProvider(c, Proxies.minecraft,
                net.minecraft.server.permissions.PermissionSet.NO_PERMISSIONS);
        }
        return suggestions;
    }

    /** The server's command tree arrives through the protocol (WS1); until it is bridged, the dispatcher is empty. */
    public static com.mojang.brigadier.CommandDispatcher<net.minecraft.client.multiplayer.ClientSuggestionProvider> getCommands(ClientPacketListener c) {
        return COMMANDS;
    }

    /** The player list (PlayerInfo) is Rust state not bridged yet: empty (no name completion). */
    public static java.util.Collection<net.minecraft.client.multiplayer.PlayerInfo> getOnlinePlayers(ClientPacketListener c) { return java.util.List.of(); }
    public static java.util.Collection<net.minecraft.client.multiplayer.PlayerInfo> getListedOnlinePlayers(ClientPacketListener c) { return java.util.List.of(); }

    /** Chat message from a screen: handed to Rust, which signs/sends it. */
    public static void sendChat(ClientPacketListener c, String message) { Native.sendChat(message, false); }

    /** Command (without the leading slash). */
    public static void sendCommand(ClientPacketListener c, String command) { Native.sendChat(command, true); }
}
