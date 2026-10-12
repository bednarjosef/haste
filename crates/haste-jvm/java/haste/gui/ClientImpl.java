package haste.gui;

import com.mojang.blaze3d.platform.Window;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.social.PlayerSocialManager;
import net.minecraft.client.sounds.SoundManager;

/** Bodies of the {@code Minecraft}, {@code Gui} and {@code Window} proxies. */
public final class ClientImpl {
    private ClientImpl() {}

    /** Current modifier state (SDL keymod bits as in {@code InputConstants.MOD_*}). */
    static int mods;
    static Screen screen;
    static int scale = 2;

    // ---- Minecraft ----
    public static Minecraft getInstance() { return Proxies.minecraft; }
    public static Window getWindow(Minecraft m) { return Proxies.window; }
    public static SoundManager getSoundManager(Minecraft m) { return Proxies.sound; }
    /** No narrator: a no-op proxy (isActive() is false). */
    public static net.minecraft.client.GameNarrator getNarrator(Minecraft m) { return Proxies.service(net.minecraft.client.GameNarrator.class); }
    /** No player reporting: a no-op proxy (no draft report). */
    public static net.minecraft.client.multiplayer.chat.report.ReportingContext getReportingContext(Minecraft m) {
        return Proxies.service(net.minecraft.client.multiplayer.chat.report.ReportingContext.class);
    }
    public static net.minecraft.client.renderer.entity.EntityRenderDispatcher getEntityRenderDispatcher(Minecraft m) {
        return Proxies.service(net.minecraft.client.renderer.entity.EntityRenderDispatcher.class);
    }
    public static PlayerSocialManager getPlayerSocialManager(Minecraft m) { return Proxies.service(PlayerSocialManager.class); }
    /** Haste always plays through a server connection (singleplayer = a local vanilla server process, WS7). */
    public static boolean isDemo(Minecraft m) { return false; }
    public static boolean isLocalServer(Minecraft m) { return false; }
    public static boolean hasSingleplayerServer(Minecraft m) { return false; }
    public static net.minecraft.client.server.IntegratedServer getSingleplayerServer(Minecraft m) { return null; }
    public static boolean isMultiplayerServer(Minecraft m) { return true; }
    public static net.minecraft.client.multiplayer.ServerData getCurrentServer(Minecraft m) { return null; }
    public static boolean isPaused(Minecraft m) { return false; }
    public static net.minecraft.client.multiplayer.ClientPacketListener getConnection(Minecraft m) {
        return Proxies.service(net.minecraft.client.multiplayer.ClientPacketListener.class);
    }
    /** Not a developer build. */
    public static boolean isOfflineDeveloperMode(Minecraft m) { return false; }
    /** Whether a text field has focus (vanilla starts SDL text input / IME then); Rust reads it for winit IME. */
    static boolean textInput;
    public static void onTextInputFocusChange(Minecraft m, net.minecraft.client.gui.components.events.GuiEventListener l, boolean focused) {
        textInput = focused;
    }
    /** IME candidate area (GUI pixels) the focused text field asked for: x, y, w, h. */
    static final int[] textInputArea = new int[4];
    public static com.mojang.blaze3d.platform.TextInputManager textInputManager(Minecraft m) {
        return Proxies.service(com.mojang.blaze3d.platform.TextInputManager.class);
    }
    public static void setTextInputArea(com.mojang.blaze3d.platform.TextInputManager t, int x, int y, int w, int h) {
        textInputArea[0] = x; textInputArea[1] = y; textInputArea[2] = w; textInputArea[3] = h;
    }
    public static void startTextInput(com.mojang.blaze3d.platform.TextInputManager t, Object owner) { textInput = true; }
    public static void stopTextInput(com.mojang.blaze3d.platform.TextInputManager t, Object owner) { textInput = false; }
    public static void stopTextInput(com.mojang.blaze3d.platform.TextInputManager t) { textInput = false; }
    public static void onTextInputFocusChange(com.mojang.blaze3d.platform.TextInputManager t, Object owner, boolean focused) { textInput = focused; }
    static net.minecraft.client.InputType lastInput = net.minecraft.client.InputType.NONE;
    public static net.minecraft.client.InputType getLastInputType(Minecraft m) { return lastInput; }
    public static void setLastInputType(Minecraft m, net.minecraft.client.InputType t) { lastInput = t; }
    public static boolean hasShiftDown(Minecraft m) { return (mods & 3) != 0; }
    public static boolean hasControlDown(Minecraft m) { return (mods & 192) != 0; }
    public static boolean hasAltDown(Minecraft m) { return (mods & 768) != 0; }

    // ---- Gui ----
    public static Screen screen(Gui g) { return screen; }

    /**
     * Switches screens like vanilla: the old screen is told it was removed, the new one is added and initialised
     * at the current GUI size. Rust sees the change through {@code GuiPipeline::is_open}.
     */
    public static void setScreen(Gui g, Screen s) {
        Screen old = screen;
        if (old != null && old != s) old.removed();
        screen = s;
        if (s != null) {
            s.added();
            s.init(GfxImpl.width, GfxImpl.height);
        }
    }

    // ---- Hud (drawn by Rust; everything else is a no-op) ----
    static int guiTicks;
    public static net.minecraft.client.gui.Font getFont(net.minecraft.client.gui.Hud h) { return Proxies.font; }
    public static int getGuiTicks(net.minecraft.client.gui.Hud h) { return guiTicks; }
    public static net.minecraft.client.gui.components.ChatComponent getChat(net.minecraft.client.gui.Hud h) {
        return Proxies.service(net.minecraft.client.gui.components.ChatComponent.class);
    }

    // ---- Window ----
    public static int getGuiScaledWidth(Window w) { return GfxImpl.width; }
    public static int getGuiScaledHeight(Window w) { return GfxImpl.height; }
    public static int getGuiScale(Window w) { return scale; }
    public static int getWidth(Window w) { return GfxImpl.width * scale; }
    public static int getHeight(Window w) { return GfxImpl.height * scale; }
    public static int getScreenWidth(Window w) { return GfxImpl.width * scale; }
    public static int getScreenHeight(Window w) { return GfxImpl.height * scale; }
    public static boolean isFocused(Window w) { return true; }
}
