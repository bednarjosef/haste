package haste.gui;

import com.mojang.blaze3d.platform.InputConstants;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.VarHandle;
import java.lang.reflect.Field;
import java.util.HashMap;
import java.util.Map;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.OptionInstance;
import net.minecraft.sounds.SoundSource;

/**
 * The options bridge between the vanilla {@code Options} object and Rust.
 *
 * <p>Every {@link OptionInstance} of the live options is wrapped so that the moment vanilla
 * applies a change ({@code OptionInstance.set} fires {@code onValueUpdate} when the value
 * actually changed), the options.txt key and value reach Rust through
 * {@link haste.rt.Native#optionsChanged}. Key bindings are polled once per client tick
 * ({@link #tick()}, from {@code Bridge.tick}) and reported the same way when the player
 * rebinds a key in the controls screen.
 *
 * <p>The bridge also mirrors the client's own key and mouse events into the
 * {@link KeyMapping}s (vanilla's {@code KeyboardHandler.keyPress}/{@code MouseHandler.onPress}
 * semantics: press and repeat set the down state and count a click, release clears it), so the
 * controls haste reads are the KeyMapping values and follow the player's bindings. The down
 * states and unconsumed clicks are read back through {@link #gameInput()} and
 * {@link #consumeClicks()} (bit layouts below, mirrored in Rust).
 */
public final class OptionBridge {
    private OptionBridge() {}

    // ---- gameInput() bits: the movement and action KeyMappings' down state ----
    public static final int B_FORWARD = 1;
    public static final int B_BACK = 1 << 1;
    public static final int B_LEFT = 1 << 2;
    public static final int B_RIGHT = 1 << 3;
    public static final int B_JUMP = 1 << 4;
    public static final int B_SNEAK = 1 << 5;
    public static final int B_SPRINT = 1 << 6;
    public static final int B_ATTACK = 1 << 7;
    public static final int B_USE = 1 << 8;

    // ---- consumeClicks() bits: action KeyMappings with clicks not yet consumed ----
    public static final int C_HOTBAR_0 = 1;
    public static final int C_HOTBAR_8 = 1 << 8;
    public static final int C_DROP = 1 << 9;
    public static final int C_SWAP_OFFHAND = 1 << 10;
    public static final int C_JUMP = 1 << 11;

    /** OptionInstance.onValueUpdate, wrapped per instance (private final, like Proxies.set). */
    private static final VarHandle ON_VALUE_UPDATE;

    static {
        try {
            ON_VALUE_UPDATE = MethodHandles.privateLookupIn(OptionInstance.class, MethodHandles.lookup())
                .findVarHandle(OptionInstance.class, "onValueUpdate", OptionInstance.ValueUpdateListener.class);
        } catch (Exception e) {
            throw new IllegalStateException("no onValueUpdate handle on OptionInstance", e);
        }
    }

    /** The key bindings were last reported as, by mapping name (detected per tick). */
    private static final Map<String, String> BINDS = new HashMap<>();

    /**
     * Wraps every OptionInstance of the live options (and every sound category slider) so each
     * change is reported to Rust. Called once from {@code Proxies.create} after the Options
     * object exists (its constructor's own load must not report: those are the initial values).
     */
    public static void install() {
        for (Field f : net.minecraft.client.Options.class.getDeclaredFields()) {
            if (!OptionInstance.class.isAssignableFrom(f.getType())) {
                continue;
            }
            f.setAccessible(true);
            try {
                wrap((OptionInstance<?>) f.get(Proxies.options), optionsKey(f.getName()));
            } catch (IllegalAccessException e) {
                throw new IllegalStateException("reading Options." + f.getName(), e);
            }
        }
        for (Map.Entry<SoundSource, OptionInstance<Double>> e : soundSources().entrySet()) {
            wrap(e.getValue(), "soundCategory_" + e.getKey().getName());
        }
    }

    /** The options.txt key of an Options field name: most match the field, vanilla renames a few
     * (see {@code Options.processOptions}/{@code processDumpedOptions}). */
    private static String optionsKey(String field) {
        return switch (field) {
            case "ambientOcclusion" -> "ao";
            case "panoramaSpeed" -> "panoramaScrollSpeed";
            case "sensitivity" -> "mouseSensitivity";
            case "framerateLimit" -> "maxFps";
            case "hideLightningFlash" -> "hideLightningFlashes";
            case "cloudStatus" -> "renderClouds";
            case "discreteMouseScroll" -> "discrete_mouse_scroll";
            default -> field;
        };
    }

    @SuppressWarnings("unchecked")
    private static void wrap(OptionInstance<?> instance, String key) {
        OptionInstance.ValueUpdateListener<Object> original =
            (OptionInstance.ValueUpdateListener<Object>) ON_VALUE_UPDATE.get(instance);
        ON_VALUE_UPDATE.set(instance, (OptionInstance.ValueUpdateListener<Object>) value -> {
            original.valueChanged(value);
            haste.rt.Native.optionsChanged(key, String.valueOf(value));
        });
    }

    @SuppressWarnings("unchecked")
    private static Map<SoundSource, OptionInstance<Double>> soundSources() throws IllegalAccessException {
        Field f = net.minecraft.client.Options.class.getDeclaredField("soundSourceVolumes");
        f.setAccessible(true);
        return (Map<SoundSource, OptionInstance<Double>>) f.get(Proxies.options);
    }

    /**
     * Once per client tick (from {@code Bridge.tick}): reports key bindings the player changed in
     * the controls screen, and rebuilds the key-to-mapping table so the game input mirroring
     * ({@link #gameKey}) matches the new bindings (vanilla's {@code KeyMapping.resetMapping}).
     */
    public static void tick() {
        boolean changed = false;
        for (KeyMapping m : Proxies.options.keyMappings) {
            String now = m.saveString();
            if (!now.equals(BINDS.get(m.getName()))) {
                BINDS.put(m.getName(), now);
                haste.rt.Native.optionsChanged("key_" + m.getName(), now);
                changed = true;
            }
        }
        if (changed) {
            KeyMapping.resetMapping();
        }
    }

    /**
     * A game key event from the client (SDL3 scancode, what the key mappings match). Vanilla's
     * {@code KeyboardHandler.keyPress}: press and repeat set the down state and count a click,
     * release clears the down state. Only called with no screen open (vanilla's
     * {@code handlesGameInput}).
     */
    public static void gameKey(int scancode, boolean pressed) {
        InputConstants.Key key = InputConstants.Type.KEYBOARD.getOrCreate(scancode);
        if (pressed) {
            KeyMapping.set(key, true);
            KeyMapping.click(key);
        } else {
            KeyMapping.set(key, false);
        }
    }

    /** A game mouse button from the client (vanilla numbering: 1 left, 2 middle, 3 right). */
    public static void gameMouse(int button, boolean pressed) {
        InputConstants.Key key = InputConstants.Type.MOUSE.getOrCreate(button);
        if (pressed) {
            KeyMapping.set(key, true);
            KeyMapping.click(key);
        } else {
            KeyMapping.set(key, false);
        }
    }

    /** The down state of the movement and action KeyMappings as a {@link #B_*} bitmask. */
    public static int gameInput() {
        return bit(Proxies.options.keyUp, B_FORWARD)
            | bit(Proxies.options.keyDown, B_BACK)
            | bit(Proxies.options.keyLeft, B_LEFT)
            | bit(Proxies.options.keyRight, B_RIGHT)
            | bit(Proxies.options.keyJump, B_JUMP)
            | bit(Proxies.options.keyShift, B_SNEAK)
            | bit(Proxies.options.keySprint, B_SPRINT)
            | bit(Proxies.options.keyAttack, B_ATTACK)
            | bit(Proxies.options.keyUse, B_USE);
    }

    private static int bit(KeyMapping m, int b) {
        return m.isDown() ? b : 0;
    }

    /**
     * Consumes the clicks of the action KeyMappings (hotbar, drop, swap offhand, jump) as a
     * {@link #C_*} bitmask: one bit per click since the last call, like the game consuming them
     * once per tick.
     */
    public static int consumeClicks() {
        int out = 0;
        for (int i = 0; i < Proxies.options.keyHotbarSlots.length; i++) {
            if (Proxies.options.keyHotbarSlots[i].consumeClick()) {
                out |= C_HOTBAR_0 << i;
            }
        }
        if (Proxies.options.keyDrop.consumeClick()) {
            out |= C_DROP;
        }
        if (Proxies.options.keySwapOffhand.consumeClick()) {
            out |= C_SWAP_OFFHAND;
        }
        if (Proxies.options.keyJump.consumeClick()) {
            out |= C_JUMP;
        }
        return out;
    }

    /** A screen opened: every KeyMapping goes up (vanilla's {@code Gui.setScreen}). */
    public static void releaseAll() {
        KeyMapping.releaseAll();
    }

    /** A screen closed: toggle mappings that were down come back (vanilla's setScreen(null)). */
    public static void restoreToggles() {
        KeyMapping.restoreToggleStatesOnScreenClosed();
    }
}
