package haste.gui;

import haste.rt.Native;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.DebugScreenOverlay;

/**
 * Bodies of the {@code DebugScreenOverlay} proxy: Haste's F3 overlay. Vanilla's overlay reads the
 * renderer, the integrated server and the profiler; this one shows what Rust provides
 * ({@code GameState::debug}) in vanilla's default entry list and formatting
 * (DebugScreenEntries' default profile: game version, fps, tps, memory, system specs, position,
 * section position, performance impactors — plus the entries Haste has data for: chunk render
 * stats, entity render stats, biome, light and the looking-at block/fluid with states and tags),
 * laid out like vanilla's DebugScreenOverlay.extractRenderState: priority lines alternating
 * left/right, regular lines split in half, then groups (first half left, rest right).
 */
public final class DebugImpl {
    private DebugImpl() {}

    /** Parsed {@code Native.debugText()} of the current frame: key -> values. */
    private static final Map<String, List<String>> TEXT = new HashMap<>();

    /** The targeted block/fluid of the current frame (null: crosshair on nothing in reach). */
    private static final Map<String, int[]> TARGET_POS = new HashMap<>();

    public static boolean showDebugScreen(DebugScreenOverlay o) { return Mirror.flag(Mirror.F_DEBUG_SHOWN); }

    /** One frame: pulls Rust's string values before the overlay is drawn. */
    static void pull() {
        TEXT.clear();
        TARGET_POS.clear();
        for (String line : Native.debugText().split("\n", -1)) {
            int sp = line.indexOf(' ');
            if (sp <= 0) continue;
            String key = line.substring(0, sp);
            String value = line.substring(sp + 1);
            if (key.equals("block") || key.equals("fluid")) {
                String[] parts = value.split(" ");
                if (parts.length == 4) {
                    int[] pos = new int[3];
                    for (int i = 0; i < 3; i++) pos[i] = Integer.parseInt(parts[i + 1]);
                    TARGET_POS.put(key, pos);
                    TEXT.computeIfAbsent(key, k -> new ArrayList<>()).add(parts[0]);
                }
                continue;
            }
            TEXT.computeIfAbsent(key, k -> new ArrayList<>()).add(value);
        }
    }

    private static List<String> texts(String key) { return TEXT.getOrDefault(key, List.of()); }
    private static String text(String key) { return texts(key).isEmpty() ? "" : texts(key).get(0); }
    private static int[] pos(String key) { return TARGET_POS.get(key); }

    /** The overlay's left and right columns, vanilla's DebugScreenOverlay.extractRenderState order. */
    static List<List<String>> columns() {
        int[] i = Mirror.I;
        double[] d = Mirror.D;
        List<String> left = new ArrayList<>();
        List<String> right = new ArrayList<>();
        List<String> regular = new ArrayList<>();
        // LinkedHashMap: group order is the order the entries displayed in (identifier-sorted).
        Map<String, List<String>> groups = new java.util.LinkedHashMap<>();

        // game_version (priority line 1 -> left), DebugEntryVersion
        left.add("Minecraft " + net.minecraft.SharedConstants.getCurrentVersion().name() + " (haste/vanilla)");
        // fps (priority line 2 -> right), DebugEntryFps
        right.add(String.format(java.util.Locale.ROOT, "%d fps T: %s @%sHz",
            i[Mirror.I_FPS], i[Mirror.I_FRAMERATE_LIMIT] == 260 ? "inf" : i[Mirror.I_FRAMERATE_LIMIT],
            i[Mirror.I_REFRESH_RATE] == 0 ? "0" : i[Mirror.I_REFRESH_RATE]));

        // biome (regular line), DebugEntryBiome
        regular.add("Biome: " + text("biome"));
        // chunk_render_stats (regular line), DebugEntryChunkRenderStats / LevelExtractor
        regular.add(String.format(java.util.Locale.ROOT, "C: %d/%d D: %d, pC: %03d, aB: %02d",
            i[Mirror.I_SECTIONS_DRAWN], i[Mirror.I_SECTIONS_TOTAL], i[Mirror.I_VIEW_DISTANCE],
            0, 0));
        // entity_render_stats (regular line), DebugEntryEntityRenderStats
        regular.add(String.format(java.util.Locale.ROOT, "E: %d/%d, SD: %d",
            i[Mirror.I_ENTITIES_RENDERED], i[Mirror.I_ENTITIES], i[Mirror.I_SIMULATION_DISTANCE]));
        // simple_performance_impactors (three regular lines)
        regular.add(String.format(java.util.Locale.ROOT, "B: %d", 0));
        regular.add("Filtering: Nearest");
        regular.add(String.format(java.util.Locale.ROOT, "Terrain Rendering: %s", "naive"));
        // tps (regular line), DebugEntryTps: remote server branch
        regular.add(String.format(java.util.Locale.ROOT, "\"%s\" server%s, %.0f tx, %.0f rx",
            text("brand"), "", d[Mirror.D_TX_PER_S], d[Mirror.D_RX_PER_S]));

        // light group, DebugEntryLight
        groups.computeIfAbsent("light", k -> new ArrayList<>()).add(String.format(
            java.util.Locale.ROOT, "Client Light: %d (%d sky, %d block)",
            Math.max(i[Mirror.I_LIGHT_SKY], i[Mirror.I_LIGHT_BLOCK]), i[Mirror.I_LIGHT_SKY], i[Mirror.I_LIGHT_BLOCK]));

        // looking_at block/fluid (states and tags), DebugEntryLookingAt
        for (String k : new String[] {"block", "fluid"}) {
            int[] p = pos(k);
            if (p == null) continue;
            List<String> g = groups.computeIfAbsent("looking_at_" + k, k2 -> new ArrayList<>());
            g.add(net.minecraft.ChatFormatting.UNDERLINE + "Targeted " + (k.equals("block") ? "Block" : "Fluid")
                + ": " + p[0] + ", " + p[1] + ", " + p[2]);
            g.addAll(texts(k));
            g.addAll(texts(k + "prop"));
            g.addAll(texts(k + "tag"));
        }

        // memory group, DebugEntryMemory
        long max = Runtime.getRuntime().maxMemory();
        long total = Runtime.getRuntime().totalMemory();
        long free = Runtime.getRuntime().freeMemory();
        long used = total - free;
        groups.computeIfAbsent("memory", k -> new ArrayList<>()).addAll(List.of(
            String.format(java.util.Locale.ROOT, "Mem: %2d%% %03d/%03dMiB", used * 100L / max, used >> 20, max >> 20),
            String.format(java.util.Locale.ROOT, "Allocated: %2d%% %03dMiB", total * 100L / max, total >> 20)));

        // player_position + player_section_position group, DebugEntryPosition
        int bx = (int) Math.floor(d[Mirror.D_X]);
        int by = (int) Math.floor(d[Mirror.D_Y]);
        int bz = (int) Math.floor(d[Mirror.D_Z]);
        float yaw = net.minecraft.util.Mth.wrapDegrees((float) d[Mirror.D_YAW]);
        float pitch = net.minecraft.util.Mth.wrapDegrees((float) d[Mirror.D_PITCH]);
        String direction = net.minecraft.core.Direction.fromYRot(yaw).getSerializedName();
        String faceString = switch (direction) {
            case "north" -> "Towards negative Z";
            case "south" -> "Towards positive Z";
            case "west" -> "Towards negative X";
            case "east" -> "Towards positive X";
            default -> "Invalid";
        };
        List<String> position = groups.computeIfAbsent("position", k -> new ArrayList<>());
        position.add(String.format(java.util.Locale.ROOT, "XYZ: %.3f / %.5f / %.3f", d[Mirror.D_X], d[Mirror.D_Y], d[Mirror.D_Z]));
        position.add(String.format(java.util.Locale.ROOT, "Block: %d %d %d", bx, by, bz));
        position.add(String.format(java.util.Locale.ROOT, "Chunk: %d %d %d [%d %d in r.%d.%d.mca]",
            bx >> 4, by >> 4, bz >> 4, (bx >> 4) & 31, (bz >> 4) & 31, bx >> 9, bz >> 9));
        position.add(String.format(java.util.Locale.ROOT, "Facing: %s (%s) (%.1f / %.1f)", direction, faceString, yaw, pitch));
        position.add(text("dimension") + " FC: 0");
        position.add(String.format(java.util.Locale.ROOT, "Section-relative: %02d %02d %02d", bx & 15, by & 15, bz & 15));

        // system_specs group, DebugEntrySystemSpecs
        List<String> system = groups.computeIfAbsent("system", k -> new ArrayList<>());
        system.add("Java: " + System.getProperty("java.version"));
        system.add("CPU: " + text("cpu"));
        system.add(String.format(java.util.Locale.ROOT, "Display: %dx%d", i[Mirror.I_DISPLAY_W], i[Mirror.I_DISPLAY_H]));
        system.add(String.format(java.util.Locale.ROOT, "Window: %dx%d (%.2fx pixel density)",
            i[Mirror.I_DISPLAY_W], i[Mirror.I_DISPLAY_H], (double) i[Mirror.I_GUI_SCALE]));
        system.add(text("gpu"));

        // vanilla's layout: blank after priority lines, regular split in half, groups split in half
        if (!left.isEmpty()) left.add("");
        if (!right.isEmpty()) right.add("");
        if (!regular.isEmpty()) {
            int mid = (regular.size() + 1) / 2;
            left.addAll(regular.subList(0, mid));
            right.addAll(regular.subList(mid, regular.size()));
            left.add("");
            if (mid < regular.size()) right.add("");
        }
        List<List<String>> finalGroups = new ArrayList<>(groups.values());
        if (!finalGroups.isEmpty()) {
            int mid = (finalGroups.size() + 1) / 2;
            for (int gi = 0; gi < finalGroups.size(); gi++) {
                List<String> lines = finalGroups.get(gi);
                if (lines.isEmpty()) continue;
                if (gi < mid) {
                    left.addAll(lines);
                    left.add("");
                } else {
                    right.addAll(lines);
                    right.add("");
                }
            }
        }

        // the overlay-visible footer (vanilla adds it to the left column)
        left.add("");
        left.add("Debug charts: [F3+l] Profiler hidden; [F3+2] fps hidden;");
        left.add("[F3+3] Ping hidden; [F3+4] Lightmap hidden");
        left.add("To edit: press [F3+q]");
        return List.of(left, right);
    }

    public static void extractRenderState(DebugScreenOverlay o, GuiGraphicsExtractor g) {
        if (!showDebugScreen(null)) return;
        pull();
        List<List<String>> columns = columns();
        int width = GfxImpl.guiWidth(g);
        drawLines(g, columns.get(0), true, width);
        drawLines(g, columns.get(1), false, width);
    }

    /** Vanilla's DebugScreenOverlay.extractLines: grey boxes, light text without shadow. */
    private static void drawLines(GuiGraphicsExtractor g, List<String> lines, boolean alignLeft, int width) {
        int height = 9;
        for (int pass = 0; pass < 2; pass++) {
            for (int li = 0; li < lines.size(); li++) {
                String line = lines.get(li);
                if (line.isEmpty()) continue;
                int w = FontImpl.width(Proxies.font, line);
                int left = alignLeft ? 2 : width - 2 - w;
                int top = 2 + height * li;
                if (pass == 0) {
                    GfxImpl.fill(g, left - 1, top - 1, left + w + 1, top + height - 1, 0x90505050);
                } else {
                    GfxImpl.text(g, Proxies.font, line, left, top, 0xFFE0E0E0, false);
                }
            }
        }
    }
}
