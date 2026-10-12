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

    /** Every KeyMapping field of 26.3's Options (javap); the ones without a known default stay unbound. */
    private static final String[] ALL = ("keyUp keyLeft keyDown keyRight keyJump keyShift keySprint keyInventory keySwapOffhand keyDrop "
        + "keyUse keyAttack keyPickItem keyChat keyPlayerList keyCommand keyFriends keySocialInteractions keyScreenshot "
        + "keyTogglePerspective keySmoothCamera keyFullscreen keyAdvancements keyQuickActions keyToggleGui "
        + "keyToggleSpectatorShaderEffects keySaveHotbarActivator keyLoadHotbarActivator keySpectatorOutlines keySpectatorHotbar "
        + "keyDebugOverlay keyDebugModifier keyDebugCrash keyDebugReloadChunk keyDebugShowHitboxes keyDebugClearChat "
        + "keyDebugShowChunkBorders keyDebugShowAdvancedTooltips keyDebugCopyRecreateCommand keyDebugSpectate "
        + "keyDebugSwitchGameMode keyDebugDebugOptions keyDebugFocusPause keyDebugDumpDynamicTextures keyDebugReloadResourcePacks "
        + "keyDebugProfiling keyDebugCopyLocation keyDebugDumpVersion keyDebugPofilingChart keyDebugFpsCharts keyDebugNetworkCharts "
        + "keyDebugLightmapTexture keyDebugSwitchTranslucencyMode").split(" ");

    /** Mouse-bound defaults (vanilla button numbering: 1 left, 2 middle, 3 right). */
    private static final Map<String, Integer> MOUSE = Map.of("keyAttack", 1, "keyPickItem", 2, "keyUse", 3);
    private static final Map<KeyMapping, Integer> BUTTONS = new IdentityHashMap<>();

    static void installDefaults(Options o) {
        java.util.List<KeyMapping> all = new java.util.ArrayList<>();
        for (String name : ALL) {
            KeyMapping k = Proxies.make(KeyMapping.class);
            KEYS.put(k, DEFAULTS.getOrDefault(name, -1));
            if (MOUSE.containsKey(name)) BUTTONS.put(k, MOUSE.get(name));
            NAMES.put(k, name);
            Proxies.set(o, name, KeyMapping.class, k);
            all.add(k);
        }
        KeyMapping[] arr = new KeyMapping[9];
        for (int i = 0; i < 9; i++) {
            arr[i] = Proxies.make(KeyMapping.class); KEYS.put(arr[i], 30 + i); NAMES.put(arr[i], "keyHotbarSlot" + (i + 1));
            all.add(arr[i]);
        }
        Proxies.set(o, "keyHotbarSlots", KeyMapping[].class, arr);
        Proxies.set(o, "keyMappings", KeyMapping[].class, all.toArray(new KeyMapping[0]));
        Proxies.set(o, "debugKeys", KeyMapping[].class, new KeyMapping[0]);
    }

    public static boolean matches(KeyMapping k, KeyEvent e) { Integer c = KEYS.get(k); return c != null && c >= 0 && c == e.key(); }
    public static boolean matchesMouse(KeyMapping k, MouseButtonEvent e) { Integer b = BUTTONS.get(k); return b != null && b == e.button(); }
    public static boolean isDown(KeyMapping k) { return false; }
    public static boolean consumeClick(KeyMapping k) { return false; }
    public static boolean isUnbound(KeyMapping k) { Integer c = KEYS.get(k); return (c == null || c < 0) && !BUTTONS.containsKey(k); }
    public static void setDown(KeyMapping k, boolean down) {}
}
