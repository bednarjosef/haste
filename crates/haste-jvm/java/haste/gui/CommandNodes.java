package haste.gui;

import com.mojang.brigadier.arguments.ArgumentType;
import com.mojang.brigadier.builder.ArgumentBuilder;
import com.mojang.brigadier.builder.LiteralArgumentBuilder;
import com.mojang.brigadier.builder.RequiredArgumentBuilder;
import net.minecraft.client.multiplayer.ClientSuggestionProvider;
import net.minecraft.commands.synchronization.SuggestionProviders;
import net.minecraft.network.protocol.game.ClientboundCommandsPacket;
import net.minecraft.resources.Identifier;

/**
 * Builds the client-side Brigadier tree from the server's {@code commands} packet (vanilla's packet decoding,
 * Haste's node builder). Client-side the tree is for parsing and suggestions only: executable nodes get a no-op
 * command (so the chat screen knows the input is complete), arguments with a suggestion type get vanilla's
 * provider for it ({@code minecraft:ask_server} asks the server through {@code command_suggestion}).
 * Approximate: nodes the server marks as restricted are kept for operators (level 2 or more) only.
 */
final class CommandNodes {
    private CommandNodes() {}

    static final ClientboundCommandsPacket.NodeBuilder<ClientSuggestionProvider> BUILDER = new ClientboundCommandsPacket.NodeBuilder<>() {
        @Override
        public ArgumentBuilder<ClientSuggestionProvider, ?> createLiteral(String name) {
            return LiteralArgumentBuilder.literal(name);
        }

        @Override
        public ArgumentBuilder<ClientSuggestionProvider, ?> createArgument(String name, ArgumentType<?> type, Identifier suggestions) {
            RequiredArgumentBuilder<ClientSuggestionProvider, ?> b = RequiredArgumentBuilder.argument(name, type);
            if (suggestions != null) b.suggests(SuggestionProviders.getProvider(suggestions));
            return b;
        }

        @Override
        public ArgumentBuilder<ClientSuggestionProvider, ?> configure(ArgumentBuilder<ClientSuggestionProvider, ?> b, boolean executable, boolean restricted) {
            if (executable) b.executes(ctx -> 0);
            if (restricted) b.requires(src -> haste.rt.Native.permissionLevel() >= 2);
            return b;
        }
    };
}
