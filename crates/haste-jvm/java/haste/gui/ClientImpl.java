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

    /**
     * The DataFixer every saved file goes through (vanilla builds {@code DataFixers.getDataFixer()} once in
     * {@code Minecraft}'s constructor). Real vanilla Java: options.txt loads through it ({@code Options.dataFix}).
     */
    private static final com.mojang.datafixers.DataFixer FIXER = net.minecraft.util.datafix.DataFixers.getDataFixer();
    public static com.mojang.datafixers.DataFixer getFixerUpper(Minecraft m) { return FIXER; }

    /**
     * The world storage (vanilla: built once in {@code Minecraft}'s constructor over
     * {@code <game dir>/saves}). Haste's singleplayer worlds live in the directory Rust gave in
     * {@code haste.singleplayerDir} (haste-setup's worlds module: {@code <config>/singleplayer},
     * the same dirs the local server plays), falling back to {@code <game dir>/saves}. Real
     * vanilla Java: the world list reads {@code level.dat} of every folder in the base dir.
     */
    private static final net.minecraft.world.level.storage.LevelStorageSource LEVEL_SOURCE = createLevelSource();
    public static net.minecraft.world.level.storage.LevelStorageSource getLevelSource(Minecraft m) {
        return LEVEL_SOURCE;
    }
    private static net.minecraft.world.level.storage.LevelStorageSource createLevelSource() {
        String dir = System.getProperty("haste.singleplayerDir", "");
        java.nio.file.Path base = !dir.isEmpty() ? java.nio.file.Path.of(dir)
            : new java.io.File(System.getProperty("haste.minecraftDir", "."), "saves").toPath();
        return net.minecraft.world.level.storage.LevelStorageSource.createDefault(base);
    }

    /**
     * The world open flows (vanilla: {@code new WorldOpenFlows(this, this.levelSource)}). The
     * class is Haste's replacement (see {@code haste/replace/.../WorldOpenFlows.java}): playing
     * or creating a world starts haste-setup's local server through Rust, not the integrated
     * server.
     */
    private static final net.minecraft.client.gui.screens.worldselection.WorldOpenFlows WORLD_FLOWS =
        new net.minecraft.client.gui.screens.worldselection.WorldOpenFlows(Proxies.minecraft, LEVEL_SOURCE);
    public static net.minecraft.client.gui.screens.worldselection.WorldOpenFlows createWorldOpenFlows(Minecraft m) {
        return WORLD_FLOWS;
    }

    /** The mod check of the title screen and the debug overlay: Haste is the vanilla client rebuilt, no mods. */
    public static net.minecraft.util.ModCheck checkModStatus() {
        return new net.minecraft.util.ModCheck(net.minecraft.util.ModCheck.Confidence.PROBABLY_NOT, "haste");
    }
    public static Window getWindow(Minecraft m) { return Proxies.window; }
    public static SoundManager getSoundManager(Minecraft m) { return Proxies.sound; }
    /** The texture manager: the GPU's textures are Rust's, so registration is recorded only (see TextureImpl). */
    public static net.minecraft.client.renderer.texture.TextureManager getTextureManager(Minecraft m) {
        return TextureImpl.textures();
    }
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
    /** Haste always plays through a server connection (singleplayer = a local vanilla server process, setup). */
    public static boolean isDemo(Minecraft m) { return false; }
    /** A local singleplayer server is running (Rust starts/stops it; the pause screen labels the button "Save and Quit"). */
    public static boolean isLocalServer(Minecraft m) { return haste.rt.Native.localWorldRunning(); }
    public static boolean hasSingleplayerServer(Minecraft m) { return haste.rt.Native.localWorldRunning(); }
    public static net.minecraft.client.server.IntegratedServer getSingleplayerServer(Minecraft m) { return null; }
    public static boolean isMultiplayerServer(Minecraft m) { return true; }
    public static net.minecraft.client.multiplayer.ServerData getCurrentServer(Minecraft m) { return null; }
    public static boolean isPaused(Minecraft m) { return false; }
    /** False while the canonical proxies are built (option changes then skip their side effects, like vanilla at startup). */
    static boolean running;
    public static boolean isRunning(Minecraft m) { return running; }
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
     * The splash text of the title screen. Splash texts come from the resource reload
     * ({@code texts/splashes.txt}), which Haste does not run yet: no splash object, so the title screen
     * draws none until Rust draws it.
     */
    public static net.minecraft.client.resources.SplashManager splashManager(Gui g) {
        return new net.minecraft.client.resources.SplashManager(null);
    }

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

    /** No player is blocked or restricted (Haste has no social/block list yet). */
    public static boolean isBlocked(Minecraft m, java.util.UUID id) { return false; }
    public static boolean isFriendOnlyRestricted(Minecraft m, java.util.UUID id) { return false; }
    /** Haste sends no telemetry: no telemetry button, no telemetry screen (the options screen asks). */
    public static boolean allowsTelemetry(Minecraft m) { return false; }
    /** Haste filters no chat text: the options the screen saves report unfiltered (no player reporting yet). */
    public static boolean isTextFilteringEnabled(Minecraft m) { return false; }
    /** Leaving the server (pause screen "Disconnect", death screen "Title Screen"): Rust disconnects. */
    public static void disconnectFromWorld(Minecraft m, net.minecraft.network.chat.Component reason) { haste.rt.Native.requestDisconnect(); }
    public static void disconnectWithSavingScreen(Minecraft m) { haste.rt.Native.requestDisconnect(); }
    public static boolean allowsMultiplayer(Minecraft m) { return true; }
    /** Quitting the game (the title screen's Quit Game): Rust closes the client. */
    public static void stop(Minecraft m) { haste.rt.Native.requestQuit(); }
    /**
     * Run a task on the client thread (vanilla queues it; Haste's callers are already there:
     * the screen's own code and the ping callbacks the pinger applies on its tick), so it runs
     * at once. Declared on BlockableEventLoop (the generated super chain of the Minecraft
     * proxy) because Minecraft itself does not override execute.
     */
    public static void execute(net.minecraft.util.thread.BlockableEventLoop<?> loop, Runnable task) {
        task.run();
    }

    /** Reduced debug info (the server's game rule, entity events 22/23): not forwarded yet, full info. */
    public static boolean showOnlyReducedInfo(Minecraft m) { return false; }
    /** Vanilla's tutorial (hints and toasts for new players); Haste starts it at the last step (no hints). */
    public static net.minecraft.client.tutorial.Tutorial getTutorial(Minecraft m) { return Proxies.tutorial; }
    /** Frames per second from Rust's render loop. */
    public static int getFps(Minecraft m) { return Mirror.I[Mirror.I_FPS]; }
    public static net.minecraft.client.DeltaTracker getDeltaTracker(Minecraft m) { return Delta.INSTANCE; }

    /** Vanilla's toast manager (advancement, recipe and system toasts), real tier-3 Java. */
    public static net.minecraft.client.gui.components.toasts.ToastManager toastManager(Gui g) { return Proxies.toasts; }

    /**
     * Blocks until {@code isDone} holds (vanilla pumps the render loop meanwhile). The world
     * creation screens call this while the data pack loads (a background future); busy-waiting
     * is enough for them: the loading screen was already set, and the future completes on its
     * own executor threads. Declared on {@code BlockableEventLoop} (the generated super chain of
     * the {@code Minecraft} proxy), like {@code execute}.
     */
    public static void managedBlock(net.minecraft.util.thread.BlockableEventLoop<?> loop, java.util.function.BooleanSupplier isDone) {
        while (!isDone.getAsBoolean()) {
            Thread.onSpinWait();
        }
    }

    /** Sets the screen and (vanilla renders a frame; the next {@code render} call does) shows it. */
    public static void setScreenAndShow(Minecraft m, Screen s) {
        Proxies.gui.setScreen(s);
    }

    /** Vanilla's chat message pipeline (decoration, delays, the chat log), real tier-3 Java. */
    public static net.minecraft.client.multiplayer.chat.ChatListener chatListener(Gui g) { return Proxies.chatListener; }

    /** The camera is always the local player (spectating other entities is not supported yet). */
    public static net.minecraft.world.entity.Entity getCameraEntity(Minecraft m) { return Proxies.player; }

    // ---- CommandHistory (in memory: the last 50 typed lines, like vanilla's cap; no file) ----
    private static final net.minecraft.util.ArrayListDeque<String> HISTORY = new net.minecraft.util.ArrayListDeque<>(50);
    public static void addCommand(net.minecraft.client.CommandHistory h, String line) {
        if (!HISTORY.isEmpty() && HISTORY.peekLast().equals(line)) return;
        if (HISTORY.size() >= 50) HISTORY.removeFirst();
        HISTORY.addLast(line);
    }
    public static java.util.Collection<String> history(net.minecraft.client.CommandHistory h) { return HISTORY; }

    // ---- Window ----
    public static int getGuiScaledWidth(Window w) { return GfxImpl.width; }
    public static int getGuiScaledHeight(Window w) { return GfxImpl.height; }
    public static int getGuiScale(Window w) { return scale; }
    /** The display mode is Rust's decision (Rust owns the window): none is preferred. */
    public static java.util.Optional<com.mojang.blaze3d.platform.VideoMode> getPreferredFullscreenVideoMode(Window w) {
        return java.util.Optional.empty();
    }
    public static int getWidth(Window w) { return GfxImpl.width * scale; }
    public static int getHeight(Window w) { return GfxImpl.height * scale; }
    public static int getScreenWidth(Window w) { return GfxImpl.width * scale; }
    public static int getScreenHeight(Window w) { return GfxImpl.height * scale; }
    public static boolean isFocused(Window w) { return true; }
}
