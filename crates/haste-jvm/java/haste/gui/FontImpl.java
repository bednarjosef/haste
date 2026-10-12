package haste.gui;

import haste.rt.Native;
import java.util.List;
import net.minecraft.client.StringSplitter;
import net.minecraft.client.gui.Font;
import net.minecraft.locale.Language;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.network.chat.Style;
import net.minecraft.util.FormattedCharSequence;

/**
 * Bodies of the {@code Font} proxy. Plain-string widths go straight to Rust's TextMetrics; styled text and
 * line splitting use vanilla's real {@link StringSplitter} with per-code-point widths from Rust.
 */
public final class FontImpl {
    private FontImpl() {}

    static final StringSplitter SPLITTER = new StringSplitter(Text::charWidth);

    public static int width(Font f, String s) { return s == null ? 0 : Native.textWidth(s); }
    public static int width(Font f, FormattedText t) { return (int) Math.ceil(Text.width(t)); }
    public static int width(Font f, FormattedCharSequence t) { return (int) Math.ceil(Text.width(t)); }

    public static String plainSubstrByWidth(Font f, String s, int w) { return SPLITTER.plainHeadByWidth(s, w, Style.EMPTY); }
    public static String plainSubstrByWidth(Font f, String s, int w, boolean reverse) {
        return reverse ? SPLITTER.plainTailByWidth(s, w, Style.EMPTY) : SPLITTER.plainHeadByWidth(s, w, Style.EMPTY);
    }
    public static FormattedText substrByWidth(Font f, FormattedText t, int w) { return SPLITTER.headByWidth(t, w, Style.EMPTY); }
    public static int wordWrapHeight(Font f, FormattedText t, int w) { return f.lineHeight * SPLITTER.splitLines(t, w, Style.EMPTY).size(); }
    public static List<FormattedCharSequence> split(Font f, FormattedText t, int w) {
        return Language.getInstance().getVisualOrder(SPLITTER.splitLines(t, w, Style.EMPTY));
    }
    public static List<FormattedText> splitIgnoringLanguage(Font f, FormattedText t, int w) { return SPLITTER.splitLines(t, w, Style.EMPTY); }
    public static boolean isBidirectional(Font f) { return false; }
    public static String bidirectionalShaping(Font f, String s) { return s; }
    public static StringSplitter getSplitter(Font f) { return SPLITTER; }

    /**
     * Prepared text, as vanilla's click and hover handling sees it: the bounds of the run and one active area per
     * character (with its style, so click/hover events on components are found under the cursor). Glyphs are
     * not rasterised here (Rust draws text from GuiFrame).
     */
    public static Font.PreparedText prepareText(Font f, FormattedCharSequence seq, float x, float y, int color, boolean shadow,
                                                boolean includeEmpty, int background) {
        java.util.List<net.minecraft.client.gui.font.EmptyArea> areas = new java.util.ArrayList<>();
        float[] cx = {x};
        seq.accept((i, style, cp) -> {
            float adv = Text.charWidth(cp, style);
            areas.add(new net.minecraft.client.gui.font.EmptyArea(cx[0], y, adv,
                net.minecraft.client.gui.font.EmptyArea.DEFAULT_ASCENT, net.minecraft.client.gui.font.EmptyArea.DEFAULT_HEIGHT, style));
            cx[0] += adv;
            return true;
        });
        int left = (int) Math.floor(x), top = (int) Math.floor(y);
        var bounds = new net.minecraft.client.gui.navigation.ScreenRectangle(left, top, Math.max(0, (int) Math.ceil(cx[0]) - left), f.lineHeight);
        return new Font.PreparedText() {
            @Override public void visit(Font.GlyphVisitor v) { for (var a : areas) v.acceptEmptyArea(a); }
            @Override public net.minecraft.client.gui.navigation.ScreenRectangle bounds() { return bounds; }
        };
    }
    public static Font.PreparedText prepareText(Font f, String s, float x, float y, int color, boolean shadow, int background) {
        return prepareText(f, FormattedCharSequence.forward(s, Style.EMPTY), x, y, color, shadow, false, background);
    }
    public static Font.PreparedText prepare8xTextOutline(Font f, FormattedCharSequence seq, float x, float y, int color) {
        return prepareText(f, seq, x, y, color, false, false, 0);
    }
}
