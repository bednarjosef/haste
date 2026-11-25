package haste.recorder.client;

import com.google.gson.GsonBuilder;
import com.google.gson.JsonObject;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;

/**
 * The {@code frametimes} scenario step's recorder: the time between two frames (from the
 * {@code Minecraft.runTick} frame hook, so it includes the buffer swap) kept in a preallocated
 * array, with no I/O or allocation per frame so the measurement does not slow the game it
 * measures. At the end it writes one summary JSON to {@code cache/oracle/26.3/bench/<name>.json}
 * in the same fields as Haste's {@code --bench-out}.
 */
public final class FrameTimes {
    private FrameTimes() {}

    /** Room for 20 minutes at 2000 FPS. */
    private static final long[] intervals = new long[2_400_000];
    private static volatile boolean active = false;
    private static int count = 0;
    private static long lastNanos = 0;

    /** Called from the frame hook every frame. */
    public static void frame(long now) {
        if (!active) return;
        if (lastNanos != 0 && count < intervals.length) intervals[count++] = now - lastNanos;
        lastNanos = now;
    }

    static void start() {
        count = 0;
        lastNanos = 0;
        active = true;
    }

    /** Stops recording and writes the summary; returns the log line. */
    /** The summed index count of LevelRenderer.visibleSections' compiled meshes (all layers). */
    static long visibleIndexCount(Minecraft mc) {
        long total = 0;
        for (SectionRenderDispatcher.RenderSection section : mc.levelRenderer.visibleSections()) {
            SectionMesh mesh = section.getSectionMesh();
            for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
                SectionMesh.SectionDraw draw = mesh.getSectionDraw(layer);
                if (draw != null) total += draw.indexCount();
            }
        }
        return total;
    }

    static String finish(String name, int renderDistance, int width, int height, int visibleSections,
        long visibleIndices) throws IOException {
        active = false;
        long[] ns = Arrays.copyOf(intervals, count);
        Arrays.sort(ns);
        double total = 0;
        for (long v : ns) total += v;
        double avgMs = count == 0 ? 0 : total / count / 1e6;
        JsonObject o = new JsonObject();
        o.addProperty("client", "vanilla");
        o.addProperty("windowed", true);
        o.addProperty("frames", count);
        o.addProperty("seconds", total / 1e9);
        o.addProperty("avg_ms", avgMs);
        o.addProperty("fps", avgMs == 0 ? 0 : 1000.0 / avgMs);
        o.addProperty("p50_ms", pct(ns, 0.50));
        o.addProperty("p90_ms", pct(ns, 0.90));
        o.addProperty("p99_ms", pct(ns, 0.99));
        o.addProperty("p999_ms", pct(ns, 0.999));
        o.addProperty("worst_ms", count == 0 ? 0 : ns[count - 1] / 1e6);
        o.addProperty("render_distance", renderDistance);
        o.addProperty("width", width);
        o.addProperty("height", height);
        // LevelRenderer.visibleSections at the end of the run: the non-empty sections in the
        // frustum the frame drew, to compare with Haste's drawn section count.
        o.addProperty("visible_sections", visibleSections);
        // The index count of those sections' compiled meshes, all layers (vanilla draws every
        // face group of a visible section; triangles = indices / 3).
        o.addProperty("visible_indices", visibleIndices);
        Path out = RecorderIO.benchDir().resolve(name + ".json");
        Files.createDirectories(out.getParent());
        Files.writeString(out, new GsonBuilder().setPrettyPrinting().create().toJson(o), StandardCharsets.UTF_8);
        return String.format(java.util.Locale.ROOT,
            "frametimes %s: %d frames, avg %.3f ms (%.0f FPS), p99 %.3f ms, worst %.3f ms, %d visible sections, %d indices -> %s",
            name, count, avgMs, avgMs == 0 ? 0 : 1000.0 / avgMs, pct(ns, 0.99),
            count == 0 ? 0 : ns[count - 1] / 1e6, visibleSections, visibleIndices, out);
    }

    private static double pct(long[] sorted, double p) {
        if (sorted.length == 0) return 0;
        int i = (int) Math.min(sorted.length - 1, Math.floor(p * sorted.length));
        return sorted[i] / 1e6;
    }
}
