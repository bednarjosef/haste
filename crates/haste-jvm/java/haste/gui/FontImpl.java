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
}
