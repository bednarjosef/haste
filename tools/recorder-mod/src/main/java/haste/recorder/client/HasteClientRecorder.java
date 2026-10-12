package haste.recorder.client;

import com.mojang.brigadier.Command;
import com.mojang.brigadier.arguments.StringArgumentType;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.command.v2.ClientCommandRegistrationCallback;
import net.fabricmc.fabric.api.client.command.v2.ClientCommands;
import net.fabricmc.fabric.api.client.command.v2.FabricClientCommandSource;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;

/**
 * Client side of the recorder: {@code /haste record|screenshot|bench <name>} and {@code /haste stop}.
 *
 * - {@code /haste record <name>}: swaps the player's input for a {@link ScriptedInput} and
 *   replays each scripted sequence (walk, sprint, jump, sprint-jump, stop, sneak,
 *   walk-into-wall, step-up-slab, swim, fall), writing
 *   {@code cache/oracle/26.3/traces/<name>-<sequence>.json} (FORMATS.md).
 * - {@code /haste screenshot <name>}: captures the framebuffer with the HUD hidden, plus the
 *   camera JSON, into {@code cache/oracle/26.3/screenshots/}.
 * - {@code /haste bench <name>}: records frame times + RSS for 60 s into
 *   {@code cache/oracle/26.3/bench/<name>.jsonl}.
 *
 * Client commands run on the client thread, safe to start recordings directly; see
 * docs/RECORDING.md for how the user drives all of this.
 */
public class HasteClientRecorder implements ClientModInitializer {
    @Override
    public void onInitializeClient() {
        ClientTickEvents.END_CLIENT_TICK.register(RecorderState::tick);
        ClientCommandRegistrationCallback.EVENT.register((dispatcher, registryAccess) ->
            dispatcher.register(ClientCommands.literal("haste")
                .then(ClientCommands.literal("record")
                    .then(ClientCommands.argument("name", StringArgumentType.string())
                        .executes(ctx -> {
                            RecorderState.startRecording(StringArgumentType.getString(ctx, "name"));
                            return Command.SINGLE_SUCCESS;
                        })))
                .then(ClientCommands.literal("screenshot")
                    .then(ClientCommands.argument("name", StringArgumentType.string())
                        .executes(ctx -> {
                            RecorderState.takeScreenshot(StringArgumentType.getString(ctx, "name"));
                            return Command.SINGLE_SUCCESS;
                        })))
                .then(ClientCommands.literal("bench")
                    .then(ClientCommands.argument("name", StringArgumentType.string())
                        .executes(ctx -> {
                            RecorderState.startBench(StringArgumentType.getString(ctx, "name"));
                            return Command.SINGLE_SUCCESS;
                        })))
                .then(ClientCommands.literal("stop")
                    .executes(ctx -> {
                        RecorderState.stopAll();
                        return Command.SINGLE_SUCCESS;
                    }))));
    }
}
