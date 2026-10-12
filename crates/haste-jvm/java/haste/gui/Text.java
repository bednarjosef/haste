package haste.gui;

import haste.rt.GuiBatch;
import haste.rt.Native;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.network.chat.Style;
import net.minecraft.network.chat.TextColor;
import net.minecraft.util.FormattedCharSequence;

/**
 * Text flattening for the batch: a styled sequence becomes one TEXT command per colour run, each placed after
 * the previous run's measured width. Bold/italic/underline/obfuscated are not represented (GuiCommand::Text
 * has no style; see NOTES.md).
 */
public final class Text {
    private Text() {}

    private static final StringBuilder RUN = new StringBuilder();

    /** Width of one code point with a style (bold adds 1 px, like the vanilla font's bold offset). */
    public static float charWidth(int cp, Style style) {
        int w = cp < CACHE.length ? cachedWidth(cp) : Native.charWidth(cp);
        return w + (style != null && style.isBold() ? 1 : 0);
    }

    private static final int[] CACHE = new int[0x10000];
    private static boolean cacheValid;

    private static int cachedWidth(int cp) {
        if (!cacheValid) { java.util.Arrays.fill(CACHE, -1); cacheValid = true; }
        int w = CACHE[cp];
        if (w < 0) { w = Native.charWidth(cp); CACHE[cp] = w; }
        return w;
    }

    /** Called when Rust installs new text metrics. */
    public static void invalidateWidths() { cacheValid = false; }

    /** Plain string of a sequence. */
    public static String plain(FormattedCharSequence seq) {
        StringBuilder sb = new StringBuilder();
        seq.accept((i, style, cp) -> { sb.appendCodePoint(cp); return true; });
        return sb.toString();
    }

    public static float width(FormattedCharSequence seq) {
        float[] w = {0};
        seq.accept((i, style, cp) -> { w[0] += charWidth(cp, style); return true; });
        return w[0];
    }

    public static float width(FormattedText text) {
        float[] w = {0};
        text.visit((style, s) -> {
            for (int i = 0; i < s.length(); ) { int cp = s.codePointAt(i); w[0] += charWidth(cp, style); i += Character.charCount(cp); }
            return java.util.Optional.empty();
        }, Style.EMPTY);
        return w[0];
    }

    /** Emits a plain string. */
    public static void emit(String s, int x, int y, int color, boolean shadow) {
        if (s == null || s.isEmpty()) return;
        GuiBatch.op(GuiBatch.TEXT, x, y, color, shadow ? 1 : 0, GuiBatch.str(s));
    }

    /** Emits a component (colour runs). */
    public static void emit(Component c, int x, int y, int color, boolean shadow) {
        if (c == null) return;
        emit(c.getVisualOrderText(), x, y, color, shadow);
    }

    /** Emits a styled sequence as one TEXT per colour run. Returns the total advance. */
    public static int emit(FormattedCharSequence seq, int x, int y, int color, boolean shadow) {
        if (seq == null) return 0;
        RUN.setLength(0);
        int[] state = {x, x, 0}; // run start x, cursor x, run colour
        Object[] last = {null};   // last TextColor (identity is fine: TextColor values are interned per colour)
        boolean[] started = {false};
        seq.accept((i, style, cp) -> {
            int c = colorOf(style, color);
            if (started[0] && c != state[2]) {
                flush(state[0], y, state[2], shadow);
                state[0] = state[1];
            }
            started[0] = true;
            state[2] = c;
            RUN.appendCodePoint(cp);
            state[1] += (int) charWidth(cp, style);
            return true;
        });
        if (started[0]) flush(state[0], y, state[2], shadow);
        return state[1] - x;
    }

    private static void flush(int x, int y, int color, boolean shadow) {
        if (RUN.length() > 0) GuiBatch.op(GuiBatch.TEXT, x, y, color, shadow ? 1 : 0, GuiBatch.str(RUN.toString()));
        RUN.setLength(0);
    }

    /** The run colour: the style's RGB with the base colour's alpha, or the base colour. */
    static int colorOf(Style style, int base) {
        TextColor tc = style == null ? null : style.getColor();
        if (tc == null) return base;
        return (base & 0xFF000000) | (tc.getValue() & 0xFFFFFF);
    }
}
