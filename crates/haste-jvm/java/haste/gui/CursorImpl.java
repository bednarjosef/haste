package haste.gui;

import com.mojang.blaze3d.platform.cursor.CursorType;
import java.util.IdentityHashMap;
import java.util.Map;

/**
 * Bodies of the {@code CursorType} proxy: vanilla's {@code CursorTypes} constants are real, the cursor objects are
 * proxies that only carry their name (the native SDL cursor belongs to Rust's window). The cursor a screen
 * requests per frame is recorded in {@link GfxImpl#cursor} (not yet part of GuiFrame: interface request).
 */
public final class CursorImpl {
    private CursorImpl() {}

    static final Map<CursorType, String> NAMES = new IdentityHashMap<>();

    public static CursorType createStandardCursor(int sdlCursor, String name, CursorType fallback) {
        CursorType c = Proxies.make(CursorType.class);
        NAMES.put(c, name);
        return c;
    }

    public static String toString(CursorType c) { return NAMES.getOrDefault(c, "default"); }
    public static void select(CursorType c) {}
}
