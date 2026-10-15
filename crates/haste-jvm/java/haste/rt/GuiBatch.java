package haste.rt;

import java.util.Arrays;

/**
 * The per-frame GUI command buffer. Draw calls on the GuiGraphicsExtractor proxy append int records here;
 * Rust takes the whole frame with one call ({@link #frame}) and decodes it into {@code haste_core::gui::GuiFrame}.
 *
 * <p>Record layout: opcode, then its operands (floats as raw int bits). Strings (texts, identifiers) go into
 * one per-frame string table referenced by index. Must stay in sync with {@code src/gui.rs}.
 */
public final class GuiBatch {
    public static final int FILL = 1;            // x1 y1 x2 y2 argb
    public static final int GRADIENT = 2;        // x1 y1 x2 y2 top bottom
    public static final int TEXT = 3;            // x y argb shadow(0/1) str
    public static final int TEXTURE = 4;         // tex x y w h u v uw vh texW texH tint blend  (u v uw vh float bits)
    public static final int SPRITE = 5;          // sprite x y w h tint blend
    public static final int ITEM = 6;            // item x y count
    public static final int SCISSOR_PUSH = 7;    // x1 y1 x2 y2
    public static final int SCISSOR_POP = 8;
    public static final int TRANSLATE_PUSH = 9;  // dx dy dz (float bits)
    public static final int TRANSLATE_POP = 10;
    public static final int SPRITE_REGION = 11;  // sprite spriteW spriteH u v x y w h tint
    public static final int STYLED_TEXT = 12;    // x y argb shadow(0/1) str flags(1 bold, 2 italic, 4 underline, 8 strikethrough, 16 obfuscated)
    public static final int ENTITY = 13;         // entityId x1 y1 x2 y2 scale(float bits) mouseX(float bits) mouseY(float bits)

    private static int[] buf = new int[4096];
    private static int len;
    private static String[] strings = new String[256];
    private static int nstrings;

    private GuiBatch() {}

    private static void ensure(int n) { if (len + n > buf.length) buf = Arrays.copyOf(buf, Math.max(buf.length * 2, len + n)); }

    /** Adds a string to this frame's table and returns its index. */
    public static int str(String s) {
        if (nstrings == strings.length) strings = Arrays.copyOf(strings, strings.length * 2);
        strings[nstrings] = s;
        return nstrings++;
    }

    public static void op(int op) { ensure(1); buf[len++] = op; }
    public static void op(int op, int a, int b, int c, int d) { ensure(5); int[] x = buf; int l = len; x[l] = op; x[l + 1] = a; x[l + 2] = b; x[l + 3] = c; x[l + 4] = d; len = l + 5; }
    public static void op(int op, int a, int b, int c, int d, int e) { ensure(6); int[] x = buf; int l = len; x[l] = op; x[l + 1] = a; x[l + 2] = b; x[l + 3] = c; x[l + 4] = d; x[l + 5] = e; len = l + 6; }
    public static void op(int op, int a, int b, int c, int d, int e, int f) { ensure(7); int[] x = buf; int l = len; x[l] = op; x[l + 1] = a; x[l + 2] = b; x[l + 3] = c; x[l + 4] = d; x[l + 5] = e; x[l + 6] = f; len = l + 7; }
    public static void ops(int... v) { ensure(v.length); System.arraycopy(v, 0, buf, len, v.length); len += v.length; }

    /** Discards the current frame. */
    public static void reset() { len = 0; Arrays.fill(strings, 0, nstrings, null); nstrings = 0; }

    /** Number of ints recorded so far this frame (tests). */
    public static int length() { return len; }

    /**
     * Returns {int[] commands, String[] strings, int[] {cursor, blurAfter}} for this frame and resets the
     * buffer. The cursor code is the shape the screen asked for (0 default arrow, then CursorTypes' order:
     * 1 arrow, 2 I-beam, 3 crosshair, 4 pointing hand, 5 resize north-south, 6 resize east-west, 7 resize
     * all, 8 not allowed). {@code blurAfter} is the command index the open screen's blur goes after
     * (vanilla: {@code Screen.extractBlurredBackground}), or -1 when no screen asked for one.
     */
    public static Object[] frame(int cursor, int blurAfter) {
        Object[] r = { Arrays.copyOf(buf, len), Arrays.copyOf(strings, nstrings), new int[] { cursor, blurAfter } };
        reset();
        return r;
    }
}
