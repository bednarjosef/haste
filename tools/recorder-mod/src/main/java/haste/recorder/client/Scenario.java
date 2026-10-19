package haste.recorder.client;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.ChatScreen;
import net.minecraft.client.gui.screens.DisconnectedScreen;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.inventory.InventoryScreen;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

/**
 * Unattended scenario runner: a plain text script, one step per line, run on the client tick
 * once the player is in the world (scripts/vanilla-client.sh passes it as
 * {@code -Dhaste.scenario=<file>}). The language is documented in
 * {@code tools/recorder-mod/scenarios/README.md}. Every run ends with the game closing itself:
 * after the last step, on a failed step, on a disconnect, or when the join never happens.
 *
 * Log lines (the script greps them): {@code haste-recorder: scenario done} on success,
 * {@code haste-recorder: scenario failed: <reason>} otherwise.
 */
public final class Scenario {
    /** Ticks to wait for the join before giving up (the title screen ticks too). */
    static final int JOIN_TIMEOUT_TICKS = 20 * 120;
    /** Upper bound for one wait_world step. */
    static final int WORLD_TIMEOUT_TICKS = 20 * 60;
    /** wait_world: all sections compiled for this many consecutive ticks. */
    static final int WORLD_STABLE_TICKS = 40;

    record Step(int line, String op, String[] args, String rest) {}

    private final Path file;
    private final List<Step> steps;
    private int index = 0;
    private int ticksAlive = 0;
    private int ticksInWorld = 0;
    private boolean finished = false;

    // per-step state
    private int stepTicks = 0;
    private int stableTicks = 0;
    private int lastVisible = -1;
    private boolean stepStarted = false;
    private boolean waitingForCapture = false;
    private boolean captureDone = false;
    private String captureError = null;
    // turn step: the yaw the turn started from.
    private float turnStartYaw = 0;

    private Scenario(Path file, List<Step> steps) {
        this.file = file;
        this.steps = steps;
    }

    /** Parses the scenario file; throws with the offending line on syntax errors. */
    public static Scenario load(Path file) throws IOException {
        List<Step> steps = new ArrayList<>();
        int n = 0;
        for (String raw : Files.readAllLines(file, StandardCharsets.UTF_8)) {
            n++;
            String line = raw.strip();
            if (line.isEmpty() || line.startsWith("#")) continue;
            String[] parts = line.split("\\s+");
            String op = parts[0];
            String rest = line.substring(op.length()).strip();
            String[] args = rest.isEmpty() ? new String[0] : rest.split("\\s+");
            switch (op) {
                case "wait", "fps", "frametimes", "wait_world", "cmd", "look", "hud", "screen", "chat", "screenshot",
                     "record", "slot", "quit", "log", "dump_models", "dump_block_sounds",
                     "turn", "fly", "frametimes_start", "frametimes_end" -> {}
                default -> throw new IOException(file + ":" + n + ": unknown step '" + op + "'");
            }
            steps.add(new Step(n, op, args, rest));
        }
        return new Scenario(file, steps);
    }

    public boolean finished() {
        return finished;
    }

    /** Called at the end of every client tick. */
    public void tick(Minecraft mc) {
        if (finished) return;
        ticksAlive++;
        if (mc.gui.screen() instanceof DisconnectedScreen) {
            fail(mc, "disconnected from the server (see the client log for the reason)");
            return;
        }
        LocalPlayer player = mc.player;
        if (player == null || mc.level == null) {
            if (ticksInWorld > 0) {
                fail(mc, "left the world");
            } else if (ticksAlive > JOIN_TIMEOUT_TICKS) {
                fail(mc, "never joined the server within " + JOIN_TIMEOUT_TICKS / 20 + " s");
            }
            return;
        }
        // The level loading screen stays up after the player exists; start after it closes.
        if (ticksInWorld == 0 && mc.gui.screen() != null) return;
        ticksInWorld++;
        if (ticksInWorld < 20) return; // let the join settle (level screen closes, first chunks)
        if (ticksInWorld == 20) RecorderIO.LOGGER.info("haste-recorder: scenario {} started", file.getFileName());

        // Run as many zero-time steps as possible this tick; waiting steps return false.
        for (int guard = 0; guard < 64 && !finished; guard++) {
            if (index >= steps.size()) {
                done(mc);
                return;
            }
            Step step = steps.get(index);
            boolean complete;
            try {
                complete = run(mc, player, step);
            } catch (RuntimeException e) {
                RecorderIO.LOGGER.error("haste-recorder: step failed", e);
                fail(mc, "line " + step.line() + " '" + step.op() + " " + step.rest() + "': " + e);
                return;
            }
            if (!complete) {
                stepTicks++;
                return;
            }
            index++;
            stepTicks = 0;
            stableTicks = 0;
            stepStarted = false;
        }
    }

    /** Runs one step; true when it is complete, false to call again next tick. */
    private boolean run(Minecraft mc, LocalPlayer player, Step s) {
        String[] a = s.args();
        switch (s.op()) {
            case "log" -> {
                RecorderIO.LOGGER.info("haste-recorder: {}", s.rest());
                return true;
            }
            case "wait" -> {
                return stepTicks + 1 >= Integer.parseInt(a[0]);
            }
            case "fps" -> {
                // Logs the game's own FPS counter once a second for <seconds> seconds (a quick
                // performance reference for Haste's numbers; the camera stays where it is).
                if (stepTicks % 20 == 19) {
                    RecorderIO.LOGGER.info("haste-recorder: fps {} framebuffer {}x{}", mc.getFps(),
                        mc.getWindow().getWidth(), mc.getWindow().getHeight());
                }
                return stepTicks + 1 >= Integer.parseInt(a[0]) * 20;
            }
            case "frametimes" -> {
                // Records every frame's time for <seconds> seconds without moving the camera and
                // writes the summary to cache/oracle/26.3/bench/<name>.json (FrameTimes).
                if (!stepStarted) {
                    stepStarted = true;
                    FrameTimes.start();
                    return false;
                }
                if (stepTicks + 1 < Integer.parseInt(a[0]) * 20) return false;
                try {
                    RecorderIO.LOGGER.info("haste-recorder: {}", FrameTimes.finish(a[1],
                        mc.options.getEffectiveRenderDistance(),
                        mc.getWindow().getWidth(), mc.getWindow().getHeight(),
                        mc.levelRenderer.visibleSections().size(), FrameTimes.visibleIndexCount(mc)));
                } catch (IOException e) {
                    throw new IllegalStateException("writing frametimes", e);
                }
                return true;
            }
            case "wait_world" -> {
                // Ready = no screen, the player's own chunk is loaded, every visible section is
                // compiled and the visible section count has not changed, for WORLD_STABLE_TICKS
                // ticks in a row. (hasRenderedAllSections alone is trivially true right after a
                // teleport, before any chunk has arrived.)
                int visible = mc.levelRenderer.visibleSections().size();
                boolean ready = mc.gui.screen() == null
                    && mc.level.hasChunk(player.getBlockX() >> 4, player.getBlockZ() >> 4)
                    && mc.levelRenderer.hasRenderedAllSections()
                    && visible > 0 && visible == lastVisible;
                lastVisible = visible;
                if (ready) stableTicks++;
                else stableTicks = 0;
                if (stableTicks >= WORLD_STABLE_TICKS) return true;
                if (stepTicks > WORLD_TIMEOUT_TICKS) {
                    RecorderIO.LOGGER.warn("haste-recorder: wait_world timed out at line {}, continuing", s.line());
                    return true;
                }
                return false;
            }
            case "cmd" -> {
                String cmd = s.rest().startsWith("/") ? s.rest().substring(1) : s.rest();
                player.connection.sendCommand(cmd);
                return true;
            }
            case "chat" -> {
                player.connection.sendChat(s.rest());
                return true;
            }
            case "look" -> {
                float yaw = Float.parseFloat(a[0]);
                float pitch = Float.parseFloat(a[1]);
                player.setYRot(yaw);
                player.setXRot(pitch);
                player.yRotO = yaw;
                player.xRotO = pitch;
                return true;
            }
            case "turn" -> {
                // Turn <degrees> over <ticks> client ticks, smoothly: each tick advances
                // the yaw linearly and the previous tick's target is left as the old
                // rotation, so rendering interpolates a constant-speed turn — the same
                // camera motion `haste --bench-window --bench-path turns` replays.
                int ticks = Integer.parseInt(a[1]);
                if (ticks < 1) throw new IllegalArgumentException("turn needs at least 1 tick");
                if (!stepStarted) {
                    stepStarted = true;
                    turnStartYaw = player.getYRot();
                }
                float degrees = Float.parseFloat(a[0]);
                player.setYRot(turnStartYaw + degrees * (stepTicks + 1) / ticks);
                player.yRotO = turnStartYaw + degrees * stepTicks / ticks;
                return stepTicks + 1 >= ticks;
            }
            case "fly" -> {
                // Fly <x> <z> blocks per tick for <ticks> client ticks (spectator: the
                // client is authoritative there). The vertical part of a flight leg is a
                // `cmd tp` before this step, like Haste's path lifting the camera.
                player.setPos(player.getX() + Double.parseDouble(a[0]), player.getY(),
                    player.getZ() + Double.parseDouble(a[1]));
                return stepTicks + 1 >= Integer.parseInt(a[2]);
            }
            case "frametimes_start" -> {
                // Frame recording around a motion sequence (the blocking `frametimes`
                // step would hold the camera still instead).
                FrameTimes.start();
                return true;
            }
            case "frametimes_end" -> {
                try {
                    RecorderIO.LOGGER.info("haste-recorder: {}", FrameTimes.finish(a[0],
                        mc.options.getEffectiveRenderDistance(),
                        mc.getWindow().getWidth(), mc.getWindow().getHeight(),
                        mc.levelRenderer.visibleSections().size(), FrameTimes.visibleIndexCount(mc)));
                } catch (IOException e) {
                    throw new IllegalStateException("writing frametimes", e);
                }
                return true;
            }
            case "slot" -> {
                player.getInventory().setSelectedSlot(Integer.parseInt(a[0]));
                return true;
            }
            case "hud" -> {
                boolean wantHidden = a[0].equals("off");
                if (mc.gui.hud.isHidden() != wantHidden) mc.gui.hud.toggle();
                return true;
            }
            case "screen" -> {
                switch (a[0]) {
                    case "none" -> mc.gui.setScreen(null);
                    case "inventory" -> mc.gui.setScreen(new InventoryScreen(player));
                    case "pause" -> mc.gui.setScreen(new PauseScreen(true));
                    case "chat" -> mc.gui.setScreen(new ChatScreen(
                        a.length > 1 ? s.rest().substring(4).strip() : "", false));
                    case "use" -> {
                        // right-click the block at x y z (opens chests, furnaces, ...)
                        BlockPos pos = new BlockPos(Integer.parseInt(a[1]), Integer.parseInt(a[2]), Integer.parseInt(a[3]));
                        mc.gameMode.useItemOn(player, InteractionHand.MAIN_HAND,
                            new BlockHitResult(Vec3.atCenterOf(pos), Direction.UP, pos, false));
                    }
                    default -> throw new IllegalArgumentException("unknown screen " + a[0]);
                }
                return true;
            }
            case "screenshot" -> {
                if (!stepStarted) {
                    stepStarted = true;
                    waitingForCapture = true;
                    captureDone = false;
                    captureError = null;
                    RecorderIO.captureScreenshot(a[0], err -> {
                        captureError = err;
                        captureDone = true;
                    });
                    return false;
                }
                if (!captureDone) {
                    if (stepTicks > 200) throw new IllegalStateException("screenshot never completed");
                    return false;
                }
                waitingForCapture = false;
                if (captureError != null) throw new IllegalStateException(captureError);
                return true;
            }
            case "record" -> {
                if (!stepStarted) {
                    stepStarted = true;
                    RecorderState.startRecording(a[0]);
                    return false;
                }
                return !RecorderState.recording();
            }
            case "dump_models" -> {
                // Zero-time: walks the client's baked model layers and writes the JSON dump.
                try {
                    int layers = ModelDumps.dumpModels();
                    RecorderIO.LOGGER.info("haste-recorder: dumped {} model layers", layers);
                } catch (IOException | ReflectiveOperationException e) {
                    throw new IllegalStateException("dump_models failed", e);
                }
                return true;
            }
            case "dump_block_sounds" -> {
                // Zero-time: walks the block registry and writes the JSON dump.
                try {
                    int blocks = BlockSoundDumps.dumpBlockSounds();
                    RecorderIO.LOGGER.info("haste-recorder: dumped {} blocks' sound types", blocks);
                } catch (IOException e) {
                    throw new IllegalStateException("dump_block_sounds failed", e);
                }
                return true;
            }
            case "quit" -> {
                done(mc);
                return true;
            }
            default -> throw new IllegalStateException("unreachable: " + s.op());
        }
    }

    private void done(Minecraft mc) {
        if (finished) return;
        finished = true;
        RecorderIO.LOGGER.info("haste-recorder: scenario done ({} steps)", steps.size());
        mc.stop();
    }

    private void fail(Minecraft mc, String reason) {
        if (finished) return;
        finished = true;
        RecorderIO.LOGGER.error("haste-recorder: scenario failed: {}", reason);
        mc.stop();
    }
}
