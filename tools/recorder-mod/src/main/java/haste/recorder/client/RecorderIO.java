package haste.recorder.client;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/** All file I/O for the client recorder: traces, screenshots, bench — into the shared cache. */
public final class RecorderIO {
    private RecorderIO() {}

    static final Logger LOGGER = LoggerFactory.getLogger("haste-recorder");

    static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

    /** cache/oracle/26.3 — same layout the server dumps use. Resolved from the game dir. */
    public static Path oracleDir() {
        Path runDir = Minecraft.getInstance().gameDirectory.toPath().toAbsolutePath();
        // runDir is .minecraft (or the server dir); the shared cache is its sibling "cache",
        // mirroring the repo layout. Fallback: an "oracle" dir next to the run dir.
        Path cache = runDir.resolve("..").resolve("cache").normalize();
        if (Files.isDirectory(cache)) {
            return cache.resolve("oracle").resolve("26.3");
        }
        return runDir.resolve("oracle").resolve("26.3");
    }

    public static Path tracesDir() {
        return oracleDir().resolve("traces");
    }

    public static Path screenshotsDir() {
        return oracleDir().resolve("screenshots");
    }

    public static Path benchDir() {
        return oracleDir().resolve("bench");
    }

    public static void chat(Minecraft mc, String message) {
        if (mc.player != null) {
            mc.player.sendSystemMessage(net.minecraft.network.chat.Component.literal(message));
        }
    }

    // ---------------------------------------------------------------- traces

    static final class TraceWriter {
        private final Path file;
        private final StringBuilder json = new StringBuilder(1 << 20);
        private final String baseName;
        private int tickIndex = 0;
        private boolean anyTick = false;

        TraceWriter(String name, LocalPlayer player) {
            this.baseName = name;
            this.file = tracesDir().resolve(name + ".json");
            json.append("{\n  \"version\": 1,\n  \"minecraft\": \"26.3\",\n  \"name\": \"")
                .append(name).append("\",\n");
            // World context: the client cannot read the level seed (no accessor in 26.3); the
            // recording world's seed is fixed by docs/RECORDING.md and cross-checked there.
            // Spawn: the player's start position (the recording world's fixed spawn).
            json.append("  \"world\": { \"seed\": 0, \"spawn\": [")
                .append(fmt(player.getX())).append(", ")
                .append(fmt(player.getY())).append(", ")
                .append(fmt(player.getZ())).append("], \"dimension\": \"")
                .append(player.level().dimension().identifier()).append("\" },\n");
            json.append("  \"player\": { \"yaw\": ").append(fmt(player.getYRot()))
                .append(", \"pitch\": ").append(fmt(player.getXRot()))
                .append(", \"initial_pos\": [").append(fmt(player.getX())).append(", ")
                .append(fmt(player.getY())).append(", ").append(fmt(player.getZ())).append("] },\n");
            json.append("  \"ticks\": [\n");
        }

        String fileName() {
            return file.getFileName().toString();
        }

        void write(float forward, float strafe, boolean jump, boolean sneak, boolean sprint,
                   double x, double y, double z, double vx, double vy, double vz,
                   boolean onGround, String pose, float width, float height) {
            if (anyTick) json.append(",\n");
            anyTick = true;
            json.append("    { \"tick\": ").append(tickIndex)
                .append(", \"input\": { \"forward\": ").append(fmt(forward))
                .append(", \"strafe\": ").append(fmt(strafe))
                .append(", \"jump\": ").append(jump)
                .append(", \"sneak\": ").append(sneak)
                .append(", \"sprint\": ").append(sprint).append(" }")
                .append(", \"pos\": [").append(fmt(x)).append(", ").append(fmt(y)).append(", ").append(fmt(z)).append("]")
                .append(", \"velocity\": [").append(fmt(vx)).append(", ").append(fmt(vy)).append(", ").append(fmt(vz)).append("]")
                .append(", \"on_ground\": ").append(onGround)
                .append(", \"pose\": \"").append(pose).append("\"")
                .append(", \"dimensions\": [").append(fmt(width)).append(", ").append(fmt(height)).append("] }");
            tickIndex++;
        }

        void close() {
            json.append(anyTick ? "\n" : "").append("  ]\n}\n");
            try {
                Files.createDirectories(file.getParent());
                Files.write(file, json.toString().getBytes(StandardCharsets.UTF_8));
            } catch (IOException e) {
                LOGGER.error("writing trace {}", file, e);
            }
        }

        private static String fmt(double v) {
            return String.format(java.util.Locale.ROOT, "%.6f", v);
        }

        private static String fmt(float v) {
            return String.format(java.util.Locale.ROOT, "%.6f", v);
        }
    }

    // ---------------------------------------------------------------- screenshots

    /**
     * Captures the main render target on the next frame (GL thread) with the HUD hidden
     * ({@link HasteHooks#hideGui} makes the Gui mixin skip it), writes
     * {@code screenshots/<name>.png} plus the camera JSON alongside.
     */
    static void captureScreenshot(String name) {
        Minecraft mc = Minecraft.getInstance();
        mc.execute(() -> {
            try {
                HasteHooks.hideGui = true;
                var target = mc.gameRenderer.mainRenderTarget();
                net.minecraft.client.Screenshot.takeScreenshot(target, 1, image -> {
                    try {
                        Path png = screenshotsDir().resolve(name + ".png");
                        Files.createDirectories(png.getParent());
                        image.writeToFile(png);
                        writeScreenshotJson(name, mc, image.getWidth(), image.getHeight());
                        chat(mc, "haste: screenshot " + png);
                    } catch (IOException e) {
                        LOGGER.error("writing screenshot", e);
                    } finally {
                        image.close();
                        HasteHooks.hideGui = false;
                    }
                });
            } catch (Throwable t) {
                HasteHooks.hideGui = false;
                LOGGER.error("screenshot failed", t);
            }
        });
    }

    private static void writeScreenshotJson(String name, Minecraft mc, int width, int height) throws IOException {
        var camera = mc.gameRenderer.mainCamera();
        var camPos = camera.position();
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("minecraft", "26.3");
        root.addProperty("name", name);
        JsonObject cam = new JsonObject();
        cam.add("pos", arrayOf(camPos.x, camPos.y, camPos.z));
        cam.addProperty("yaw", camera.yRot());
        cam.addProperty("pitch", camera.xRot());
        cam.addProperty("fov_deg", mc.options.fov().get().doubleValue());
        root.add("camera", cam);
        var level = mc.level;
        root.addProperty("time_of_day", level != null ? level.getOverworldClockTime() : 0);
        root.addProperty("render_distance", mc.options.renderDistance().get());
        root.addProperty("gui_hidden", true);
        root.addProperty("width", width);
        root.addProperty("height", height);
        Path json = screenshotsDir().resolve(name + ".json");
        Files.write(json, GSON.toJson(root).getBytes(StandardCharsets.UTF_8));
    }

    private static JsonArray arrayOf(double... values) {
        JsonArray a = new JsonArray();
        for (double v : values) a.add(v);
        return a;
    }

    // ---------------------------------------------------------------- bench

    private static java.io.PrintWriter benchWriter = null;
    private static long benchFrame = 0;
    private static long lastFrameNanos = 0;

    static void startBench(String name) {
        try {
            Files.createDirectories(benchDir());
            benchWriter = new java.io.PrintWriter(Files.newBufferedWriter(
                benchDir().resolve(name + ".jsonl"), StandardCharsets.UTF_8));
            benchFrame = 0;
            lastFrameNanos = 0;
            HasteHooks.benchRunning = true;
        } catch (IOException e) {
            LOGGER.error("starting bench", e);
        }
    }

    /** Called once per frame (from the frame hook) with the frame time and section counts. */
    public static void benchFrame(double frameMs, long rssKib, int sectionsDrawn, int sectionsTotal, int drawCalls) {
        if (benchWriter == null) return;
        benchWriter.printf(java.util.Locale.ROOT,
            "{\"frame\":%d,\"frame_ms\":%.4f,\"rss_kib\":%d,\"sections_drawn\":%d,\"sections_total\":%d,\"draw_calls\":%d}%n",
            benchFrame++, frameMs, rssKib, sectionsDrawn, sectionsTotal, drawCalls);
    }

    static void benchTick() {
        // flush periodically so a crash does not lose the whole run
        if (benchWriter != null) benchWriter.flush();
    }

    static void finishBench(String name) {
        HasteHooks.benchRunning = false;
        if (benchWriter != null) {
            benchWriter.flush();
            benchWriter.close();
            benchWriter = null;
            LOGGER.info("bench {} written to {}", name, benchDir());
        }
    }
}
