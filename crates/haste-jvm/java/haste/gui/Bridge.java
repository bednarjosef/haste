package haste.gui;

import haste.rt.GuiBatch;
import net.minecraft.client.gui.screens.ChatScreen;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.inventory.InventoryScreen;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.input.MouseButtonInfo;

/**
 * Game-side entry point. Rust holds the single instance and calls these methods (one JNI call each); every
 * method runs vanilla screen code with the 26.3 signatures.
 */
public final class Bridge {
    public static final int CHAT = 0, PAUSE = 1, INVENTORY = 2;

    private int pressedButton = -1;
    private double lastX, lastY;

    private Bridge() {}

    /** How long vanilla's bootstrap took (ms). */
    public static long bootstrapMillis;

    public static Bridge create() {
        // like vanilla's startup: version.json from the player's jar (resources come through HasteClassLoader)
        net.minecraft.SharedConstants.tryDetectVersion();
        // vanilla's bootstrap fills every built-in registry from the player's jar (blocks, items, entity types,
        // data component types, ...): the registry data tier-3 code looks up. See NOTES.md "Registries".
        long t = System.nanoTime();
        net.minecraft.server.Bootstrap.bootStrap();
        bootstrapMillis = (System.nanoTime() - t) / 1_000_000;
        GfxImpl.width = 320;
        GfxImpl.height = 240;
        Proxies.create();
        return new Bridge();
    }

    /** Opens one of the supported vanilla screens (replacing the current one). */
    public void open(int kind) {
        Screen s = switch (kind) {
            case CHAT -> new ChatScreen("", false);
            case PAUSE -> new PauseScreen(true);
            case INVENTORY -> new InventoryScreen(Proxies.player);
            default -> throw new IllegalArgumentException("screen kind " + kind);
        };
        Proxies.gui.setScreen(s);
    }

    public void close() { Proxies.gui.setScreen(null); }

    public boolean isOpen() { return ClientImpl.screen != null; }

    /** Name of the open screen's class (tests), or null. */
    public String screenName() { return ClientImpl.screen == null ? null : ClientImpl.screen.getClass().getName(); }

    /** Runs the open screen's extraction into the batch and returns {int[] commands, String[] strings}. */
    public Object[] render(int width, int height, int scale, double mouseX, double mouseY, float partial) {
        ClientImpl.scale = scale;
        Screen s = ClientImpl.screen;
        if (s != null && (width != GfxImpl.width || height != GfxImpl.height)) {
            GfxImpl.width = width; GfxImpl.height = height;
            s.resize(width, height);
        }
        GfxImpl.begin(width, height);
        s = ClientImpl.screen;
        if (s != null) s.extractRenderStateWithTooltipAndSubtitles(Proxies.graphics, (int) mouseX, (int) mouseY, partial);
        GfxImpl.end();
        return GuiBatch.frame();
    }

    /** Ticks the open screen (20 Hz: cursor blink, chat history refresh). */
    public void tick() {
        ClientImpl.guiTicks++;
        if (ClientImpl.screen != null) ClientImpl.screen.tick();
    }

    public boolean key(int key, int keycode, int mods, boolean pressed) {
        ClientImpl.mods = mods;
        Screen s = ClientImpl.screen;
        if (s == null) return false;
        KeyEvent e = new KeyEvent(key, keycode, mods);
        boolean r = pressed ? s.keyPressed(e) : s.keyReleased(e);
        InventorySync.sync();
        return r;
    }

    public boolean charTyped(int codepoint, int mods) {
        Screen s = ClientImpl.screen;
        if (s == null) return false;
        return s.charTyped(new CharacterEvent(codepoint));
    }

    /** Mouse move (button < 0) or press/release of {@code button} (vanilla numbering: 1 left, 2 middle, 3 right). */
    public boolean mouse(double x, double y, int button, boolean pressed) {
        Screen s = ClientImpl.screen;
        if (s == null) return false;
        double dx = x - lastX, dy = y - lastY;
        lastX = x; lastY = y;
        MouseImpl.x = x; MouseImpl.y = y;
        ClientImpl.lastInput = net.minecraft.client.InputType.MOUSE;
        if (button < 0) {
            s.mouseMoved(x, y);
            if (pressedButton >= 0) return s.mouseDragged(new MouseButtonEvent(x, y, new MouseButtonInfo(pressedButton, ClientImpl.mods)), dx, dy);
            return false;
        }
        MouseButtonEvent e = new MouseButtonEvent(x, y, new MouseButtonInfo(button, ClientImpl.mods));
        MouseImpl.pressed = pressed ? button : -1;
        boolean r;
        if (pressed) {
            pressedButton = button;
            r = s.mouseClicked(e, false);
        } else {
            pressedButton = -1;
            r = s.mouseReleased(e);
        }
        InventorySync.sync();
        return r;
    }

    public boolean scroll(double x, double y, double dx, double dy) {
        Screen s = ClientImpl.screen;
        return s != null && s.mouseScrolled(x, y, dx, dy);
    }

    /** Whether a screen asked for the mouse to be grabbed (e.g. "Back to Game") and not released since. */
    public boolean mouseGrabbed() { return MouseImpl.grabbed; }

    /** Rust grabbed/released the mouse itself (e.g. on opening a screen). */
    public void setMouseGrabbed(boolean g) { MouseImpl.grabbed = g; }

    // ---- registries and packets (vanilla wire format, see Registries and Packets) ----

    /** A configuration-phase {@code registry_data} packet body. */
    public void registryData(byte[] body) { Registries.registryData(body); }

    /** An {@code update_tags} packet body; {@code play}: received in the play phase (applied at once). */
    public void tags(byte[] body, boolean play) { Registries.tags(body, play); }

    /** End of the configuration phase: builds the synchronised registries. */
    public void finishConfiguration() { Registries.finish(); InventorySync.reset(); }

    /** Synchronised registries from the vanilla data pack in the client jar (no server; tests). */
    public void loadJarRegistries() throws java.io.IOException { Registries.loadFromJar(); InventorySync.reset(); }

    /** A clientbound play packet (VarInt id + body) whose state lives in tier 3. */
    public void packet(byte[] idAndBody) {
        Packets.receive(idAndBody);
        InventorySync.sync();
    }

    /** Sets a player inventory slot (vanilla numbering) from a stack in vanilla's network encoding. */
    public void setInventorySlot(int slot, byte[] stack) {
        Proxies.inventory.setItem(slot, InventorySync.decode(stack));
        InventorySync.sync();
    }

    /** "registry=size" lines of every registry Java sees (tests). */
    public String registrySizes() { return Registries.sizes(); }

    /** Name of an entry by registry and numeric id (tests), or null. */
    public String registryName(String registry, int id) { return Registries.name(registry, id); }

    /** Rust installed new text metrics: drop cached glyph widths. */
    public void metricsChanged() {
        Text.invalidateWidths();
        Proxies.set(Proxies.font, "lineHeight", int.class, haste.rt.Native.lineHeight());
    }
}
