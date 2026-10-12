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
        ClientImpl.running = true;
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

    /**
     * One frame: the vanilla HUD (when in a world) and then the open screen on top, like the game, into the batch.
     * Returns {int[] commands, String[] strings}.
     */
    public Object[] render(int width, int height, int scale, double mouseX, double mouseY, float partial) {
        ClientImpl.scale = scale;
        Mirror.update();
        Screen s = ClientImpl.screen;
        if (s != null && (width != GfxImpl.width || height != GfxImpl.height)) {
            GfxImpl.width = width; GfxImpl.height = height;
            s.resize(width, height);
        }
        GfxImpl.begin(width, height);
        if (Mirror.inWorld()) {
            Delta.frame(partial);
            Proxies.hud.extractRenderState(Proxies.graphics, Delta.INSTANCE);
            GfxImpl.POSE.reset();
        }
        s = ClientImpl.screen;
        if (s != null) s.extractRenderStateWithTooltipAndSubtitles(Proxies.graphics, (int) mouseX, (int) mouseY, partial);
        GfxImpl.end();
        return GuiBatch.frame(GfxImpl.cursor);
    }

    /** One 20 Hz client tick: the HUD (chat fade, titles, overlay message), the chat queue and the open screen. */
    public void tick() {
        Mirror.update();
        if (Mirror.inWorld()) {
            Proxies.hud.tick(false);
            Proxies.chatListener.tick();
            PlayerImpl.tickEffects();
        }
        if (ClientImpl.screen != null) ClientImpl.screen.tick();
        InventorySync.sync();
    }

    /** The local player as the server introduced it (login packet, game profile). */
    public void setPlayer(int entityId, String name, long uuidMost, long uuidLeast) {
        Handlers.localEntityId = entityId;
        Proxies.set(Proxies.player, "id", int.class, entityId);
        java.util.UUID uuid = new java.util.UUID(uuidMost, uuidLeast);
        Proxies.set(Proxies.player, "uuid", java.util.UUID.class, uuid);
        Proxies.set(Proxies.player, "stringUUID", String.class, uuid.toString());
        PlayerImpl.profile = new com.mojang.authlib.GameProfile(uuid, name);
    }

    /** The joined world's dimension type (its id in the synchronised dimension_type registry). */
    public void setDimensionType(int id) { LevelImpl.dimensionTypeId = id; }

    /** F1: hides or shows the HUD (vanilla's toggle). */
    public void toggleHud() { Proxies.hud.toggle(); }

    /** F3: shows or hides the debug overlay (Haste's stub). */
    public void toggleDebug() { DebugImpl.shown = !DebugImpl.shown; }

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
