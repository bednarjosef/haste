package haste.gui;

import haste.rt.Native;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.core.RegistryAccess;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.GameProtocols;
import net.minecraft.network.protocol.game.ServerGamePacketListener;

/**
 * Play packets crossing between Rust and tier-3 Java in vanilla's own wire format.
 *
 * <p>Clientbound: Rust (which owns the connection) re-encodes the packets whose state is tier 3 (chat, boss bars,
 * scoreboard, titles, tab list, command tree, containers, ...) and Java decodes them with vanilla's play protocol
 * codec and hands them to the {@code ClientPacketListener} proxy, whose handlers ({@link ConnectionImpl}) feed the
 * real vanilla components. Serverbound: packets vanilla code sends ({@code connection.send(packet)}) are encoded
 * with vanilla's codec and handed to Rust as id + body bytes, so they are byte-exact by construction.
 */
public final class Packets {
    private Packets() {}

    private static RegistryAccess boundTo;
    private static ProtocolInfo<ClientGamePacketListener> clientbound;
    private static ProtocolInfo<ServerGamePacketListener> serverbound;

    private static void bind() {
        RegistryAccess a = Registries.access();
        if (a == boundTo) return;
        clientbound = GameProtocols.CLIENTBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(a));
        serverbound = GameProtocols.SERVERBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(a), new GameProtocols.Context() {
            @Override public boolean hasInfiniteMaterials() { return PlayerImpl.isCreative(Proxies.player); }
            @Override public boolean canUseCommandBlocks() { return false; }
        });
        boundTo = a;
    }

    /** Decodes one clientbound play packet (VarInt id + body) and handles it on the connection proxy. */
    @SuppressWarnings({"unchecked", "rawtypes"})
    static void receive(byte[] idAndBody) {
        bind();
        ByteBuf buf = Unpooled.wrappedBuffer(idAndBody);
        Packet<? super ClientGamePacketListener> p = clientbound.codec().decode(buf);
        if (buf.isReadable()) {
            throw new IllegalStateException("packet " + p.type() + ": " + buf.readableBytes() + " bytes left after decoding");
        }
        ((Packet) p).handle(Proxies.connection);
    }

    /** Encodes a serverbound play packet with vanilla's codec and hands it to Rust (VarInt id + body). */
    @SuppressWarnings({"unchecked", "rawtypes"})
    static void send(Packet<?> p) {
        bind();
        ByteBuf buf = Unpooled.buffer();
        ((net.minecraft.network.codec.StreamCodec) serverbound.codec()).encode(buf, p);
        byte[] b = new byte[buf.readableBytes()];
        buf.readBytes(b);
        Native.sendPacket(b);
    }
}
