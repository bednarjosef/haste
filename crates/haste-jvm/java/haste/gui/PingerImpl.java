package haste.gui;

import haste.rt.Native;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.ChatFormatting;
import net.minecraft.SharedConstants;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.ServerStatusPinger;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.status.ServerStatus;

/**
 * Bodies of the {@code ServerStatusPinger} proxy: the server list's status pings. Vanilla's pinger
 * opens one netty connection per entry and speaks the status protocol in Java; in Haste the network
 * is Rust's ({@code haste_protocol::ping}, one thread per address through {@link Native#startPing}),
 * and the results are applied here on the screen's tick ({@link Native#takePingResults}), the same
 * places vanilla applies them (its ping callbacks set the same {@code ServerData} fields; see
 * ServerStatusPinger.pingServer's handleStatusResponse).
 */
public final class PingerImpl {
    private PingerImpl() {}

    /** One pending ping: the entry it updates and the callbacks the screen passed. */
    private static final class Pending {
        final ServerData data;
        final Runnable onPongResponse;

        Pending(ServerData data, Runnable onPongResponse) {
            this.data = data;
            this.onPongResponse = onPongResponse;
        }
    }

    /** Saved servers by address (vanilla keys nothing; it holds the listener per connection). */
    private static final Map<String, Pending> PENDING = new HashMap<>();

    /** Starts the ping (vanilla: ServerStatusPinger.pingServer, on the entry's first render). */
    public static void pingServer(
        ServerStatusPinger p, ServerData data, Runnable onPersistentDataChange,
        Runnable onPongResponse, net.minecraft.server.network.EventLoopGroupHolder eventLoopGroupHolder
    ) {
        synchronized (PENDING) {
            PENDING.put(data.ip, new Pending(data, onPongResponse));
        }
        Native.startPing(data.ip);
    }

    /** Applies finished pings (called from JoinMultiplayerScreen.tick through pinger.tick()). */
    public static void tick(ServerStatusPinger p) {
        String[] results = Native.takePingResults();
        if (results.length == 0) return;
        List<Runnable> callbacks = new ArrayList<>();
        synchronized (PENDING) {
            for (String line : results) {
                String[] parts = line.split("\n", -1);
                Pending pending = PENDING.remove(parts[0]);
                if (pending == null) continue; // the screen closed or refreshed meanwhile
                if (parts[1].isEmpty()) {
                    failed(pending.data);
                } else {
                    apply(pending.data, parts[1], Long.parseLong(parts[2]));
                    // vanilla's onPongResponse: the state the entry's status icon shows
                    pending.data.setState(pending.data.protocol == SharedConstants.getCurrentVersion().protocolVersion()
                        ? ServerData.State.SUCCESSFUL : ServerData.State.INCOMPATIBLE);
                }
                callbacks.add(pending.onPongResponse);
            }
        }
        for (Runnable r : callbacks) r.run();
    }

    /** Stops tracking pings (vanilla: removeAll, on the screen's removal). */
    public static void removeAll(ServerStatusPinger p) {
        synchronized (PENDING) {
            PENDING.clear();
        }
    }

    /** The status fields vanilla's successful ping sets: motd, version, player count, ping bars. */
    private static void apply(ServerData data, String json, long millis) {
        ServerStatus status = ServerStatus.CODEC
            .parse(com.mojang.serialization.JsonOps.INSTANCE, com.google.gson.JsonParser.parseString(json))
            .result().orElse(null);
        if (status == null) {
            failed(data);
            return;
        }
        data.motd = status.description();
        data.ping = millis;
        status.version().ifPresentOrElse(version -> {
            data.version = Component.literal(version.name());
            data.protocol = version.protocol();
        }, () -> {
            data.version = Component.translatable("multiplayer.status.old");
            data.protocol = 0;
        });
        status.players().ifPresentOrElse(players -> {
            data.status = formatPlayerCount(players.online(), players.max());
            data.players = players;
            List<Component> names = new ArrayList<>(players.sample().size());
            for (var profile : players.sample()) names.add(Component.literal(profile.name()));
            if (players.sample().size() < players.online()) {
                names.add(Component.translatable("multiplayer.status.and_more", players.online() - players.sample().size()));
            }
            data.playerList = names;
        }, () -> data.status = Component.translatable("multiplayer.status.unknown").withStyle(ChatFormatting.DARK_GRAY));
    }

    /** The reachable-but-incompatible or unreachable display (vanilla: onPingFailed). */
    private static void failed(ServerData data) {
        data.motd = Component.translatable("multiplayer.status.cannot_connect").withColor(-65536);
        data.status = net.minecraft.network.chat.CommonComponents.EMPTY;
    }

    /** "online/max" the entry's right side shows (vanilla: ServerStatusPinger.formatPlayerCount). */
    public static Component formatPlayerCount(int online, int max) {
        Component current = Component.literal(Integer.toString(online)).withStyle(ChatFormatting.GRAY);
        Component maximum = Component.literal(Integer.toString(max)).withStyle(ChatFormatting.GRAY);
        return Component.translatable("multiplayer.status.player_count", current, maximum).withStyle(ChatFormatting.DARK_GRAY);
    }
}
