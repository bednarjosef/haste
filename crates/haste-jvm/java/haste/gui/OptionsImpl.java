package haste.gui;

import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.Map;
import net.minecraft.client.MusicToastDisplayState;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;

/**
 * Bodies of the {@code Options} and {@code OptionInstance} proxies. Each option is one canonical OptionInstance
 * proxy whose value lives here. Values are fixed defaults for now (Haste's config, WS7, will feed them); every
 * value and its source is listed in NOTES.md.
 */
public final class OptionsImpl {
    private OptionsImpl() {}

    private static final Map<String, OptionInstance<?>> BY_NAME = new HashMap<>();
    private static final Map<OptionInstance<?>, Object> VALUES = new IdentityHashMap<>();

    @SuppressWarnings("unchecked")
    private static <T> OptionInstance<T> opt(String name, T initial) {
        return (OptionInstance<T>) BY_NAME.computeIfAbsent(name, n -> {
            OptionInstance<?> o = Proxies.make(OptionInstance.class);
            VALUES.put(o, initial);
            return o;
        });
    }

    // ---- OptionInstance ----
    public static Object get(OptionInstance<?> o) { return VALUES.get(o); }
    public static void set(OptionInstance<?> o, Object v) { VALUES.put(o, v); }

    // ---- Options ----
    /** Vanilla default "Menu Background Blur" (5). Blur is not representable in GuiFrame anyway. */
    public static int getMenuBackgroundBlurriness(Options o) { return 5; }
    public static OptionInstance<Integer> menuBackgroundBlurriness(Options o) { return opt("menuBackgroundBlurriness", 5); }
    /**
     * Text background colour: vanilla derives it from the "Text Background Opacity" options; Haste has no such
     * options yet, so the caller's default is used (approximate).
     */
    public static int getBackgroundColor(Options o, int defaultColor) { return defaultColor; }
    public static int getBackgroundColor(Options o, float defaultOpacity) { return Math.round(defaultOpacity * 255f) << 24; }

    /** Chat drafts are not kept between chat screens (Haste choice; the vanilla default is not determined). */
    public static OptionInstance<Boolean> saveChatDrafts(Options o) { return opt("saveChatDrafts", false); }

    /** "Command Suggestions": on (vanilla's default in Chat Settings). */
    public static OptionInstance<Boolean> autoSuggestions(Options o) { return opt("autoSuggestions", true); }

    /** Haste plays no music: never show the now-playing toast. */
    public static OptionInstance<MusicToastDisplayState> musicToast(Options o) { return opt("musicToast", MusicToastDisplayState.NEVER); }
}
