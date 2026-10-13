package haste.gui;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.DebugScreenOverlay;

/**
 * Bodies of the {@code DebugScreenOverlay} proxy: Haste's F3 stub. Vanilla's overlay reads the renderer, the
 * integrated server and the profiler; this one shows what Rust provides ({@code GameState::debug}): FPS, position,
 * facing, chunk and entity counts, in the overlay's look (grey boxes, light text, no shadow, top left).
 */
public final class DebugImpl {
    private DebugImpl() {}

    static boolean shown;

    public static boolean showDebugScreen(DebugScreenOverlay o) { return shown; }

    /** Lines of the stub overlay. */
    static List<String> lines() {
        int[] i = Mirror.I;
        double[] d = Mirror.D;
        List<String> l = new ArrayList<>();
        l.add("Minecraft " + net.minecraft.SharedConstants.getCurrentVersion().name() + " (Haste)");
        l.add(i[Mirror.I_FPS] + " fps");
        l.add("");
        l.add(String.format(java.util.Locale.ROOT, "XYZ: %.3f / %.5f / %.3f", d[Mirror.D_X], d[Mirror.D_Y], d[Mirror.D_Z]));
        l.add(String.format(java.util.Locale.ROOT, "Block: %d %d %d", (int) Math.floor(d[Mirror.D_X]), (int) Math.floor(d[Mirror.D_Y]), (int) Math.floor(d[Mirror.D_Z])));
        l.add(String.format(java.util.Locale.ROOT, "Chunk: %d %d", ((int) Math.floor(d[Mirror.D_X])) >> 4, ((int) Math.floor(d[Mirror.D_Z])) >> 4));
        float yaw = net.minecraft.util.Mth.wrapDegrees((float) d[Mirror.D_YAW]);
        l.add(String.format(java.util.Locale.ROOT, "Facing: %s (%.1f / %.1f)",
            net.minecraft.core.Direction.fromYRot(yaw).getSerializedName(), yaw, d[Mirror.D_PITCH]));
        l.add("");
        l.add("Chunks: " + i[Mirror.I_CHUNKS] + ", sections drawn " + i[Mirror.I_SECTIONS_DRAWN] + "/" + i[Mirror.I_SECTIONS_TOTAL]);
        l.add("Entities: " + i[Mirror.I_ENTITIES]);
        return l;
    }

    public static void extractRenderState(DebugScreenOverlay o, GuiGraphicsExtractor g) {
        if (!shown) return;
        int y = 2;
        for (String s : lines()) {
            if (!s.isEmpty()) {
                int w = FontImpl.width(Proxies.font, s);
                GfxImpl.fill(g, 1, y - 1, 2 + w + 1, y + Proxies.font.lineHeight - 1, 0x90505050);
                GfxImpl.text(g, Proxies.font, s, 2, y, 0xFFE0E0E0, false);
            }
            y += Proxies.font.lineHeight;
        }
    }
}
