package haste.gui;

import java.util.IdentityHashMap;
import java.util.Map;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Options;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;

/**
 * Bodies of the {@code KeyMapping} proxy. Each Options key field gets a canonical KeyMapping proxy bound to its
 * default key (the vanilla defaults from the controls screen; rebinding is not supported yet).
 */
public final class KeyImpl {
    private KeyImpl() {}

    private static final Map<KeyMapping, Integer> KEYS = new IdentityHashMap<>();
    private static final Map<KeyMapping, String> NAMES = new IdentityHashMap<>();

    /**
     * Default keys by Options field name, in 26.3's key numbering ({@code KeyEvent.key()}: SDL3 scancodes, the
     * values of {@code InputConstants.KEY_*}). Defaults as listed on the controls screen.
     */
    private static final Map<String, Integer> DEFAULTS = Map.ofEntries(
        Map.entry("keyUp", 26), Map.entry("keyLeft", 4), Map.entry("keyDown", 22), Map.entry("keyRight", 7),
        Map.entry("keyJump", 44), Map.entry("keyShift", 225), Map.entry("keySprint", 224), Map.entry("keyInventory", 8),
        Map.entry("keySwapOffhand", 9), Map.entry("keyDrop", 20), Map.entry("keyChat", 23), Map.entry("keyPlayerList", 43),
        Map.entry("keyCommand", 56), Map.entry("keyScreenshot", 59), Map.entry("keyTogglePerspective", 62),
        Map.entry("keyFullscreen", 68), Map.entry("keySocialInteractions", 19), Map.entry("keyAdvancements", 15));

    static void installDefaults(Options o) {
        for (String name : DEFAULTS.keySet()) {
            KeyMapping k = Proxies.make(KeyMapping.class);
            KEYS.put(k, DEFAULTS.get(name));
            NAMES.put(k, name);
            Proxies.set(o, name, KeyMapping.class, k);
        }
        KeyMapping[] arr = new KeyMapping[9];
        for (int i = 0; i < 9; i++) { arr[i] = Proxies.make(KeyMapping.class); KEYS.put(arr[i], 30 + i); NAMES.put(arr[i], "keyHotbarSlot" + (i + 1)); }
        Proxies.set(o, "keyHotbarSlots", KeyMapping[].class, arr);
    }

    public static boolean matches(KeyMapping k, KeyEvent e) { Integer c = KEYS.get(k); return c != null && c >= 0 && c == e.key(); }
    public static boolean matchesMouse(KeyMapping k, MouseButtonEvent e) { return false; }
    public static boolean isDown(KeyMapping k) { return false; }
    public static boolean consumeClick(KeyMapping k) { return false; }
    public static boolean isUnbound(KeyMapping k) { Integer c = KEYS.get(k); return c == null || c < 0; }
    public static void setDown(KeyMapping k, boolean down) {}
}
