package haste.rt;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** Draw calls on the GuiGraphicsExtractor proxy append to this per-frame command list; Rust drains it once per frame. */
public final class GuiBatch {
    public static final int FILL = 1, TEXT = 2;
    private static int[] buf = new int[1024];
    private static int len;
    private static final List<String> strings = new ArrayList<>();

    private static void push(int... v) {
        if (len + v.length > buf.length) buf = Arrays.copyOf(buf, buf.length * 2);
        System.arraycopy(v, 0, buf, len, v.length);
        len += v.length;
    }
    // delegates: first parameter is the proxy instance (`this`), then the vanilla parameters
    public static void fill(Object g, int x1, int y1, int x2, int y2, int color) { push(FILL, x1, y1, x2, y2, color); }
    public static void text(Object g, Object font, Object s, int x, int y, int color) { strings.add(String.valueOf(s)); push(TEXT, strings.size() - 1, x, y, color); }

    public static int[] drain() { int[] r = Arrays.copyOf(buf, len); len = 0; return r; }
    public static String[] drainStrings() { String[] r = strings.toArray(new String[0]); strings.clear(); return r; }
}
