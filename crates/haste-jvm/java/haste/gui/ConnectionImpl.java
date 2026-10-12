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

    /** Chat message from a screen: handed to Rust, which signs/sends it. */
    public static void sendChat(ClientPacketListener c, String message) { Native.sendChat(message, false); }

    /** Command (without the leading slash). */
    public static void sendCommand(ClientPacketListener c, String command) { Native.sendChat(command, true); }
}
