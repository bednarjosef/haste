package haste.rt;

/**
 * Every Java -> Rust native in one class (registered once by {@code haste_jvm::natives} with RegisterNatives).
 * Proxy bodies reach these through impl classes in {@code haste.gui}.
 */
public final class Native {
    private Native() {}

    // ---- text metrics (Rust TextMetrics) ----
    /** Width of a plain string in GUI pixels. */
    public static native int textWidth(String s);
    /** Advance of one code point in GUI pixels. */
    public static native int charWidth(int codePoint);
    /** Line height in GUI pixels (vanilla 9). */
    public static native int lineHeight();

    // ---- player state (Rust GameState) ----
    /** Selected hotbar slot. */
    public static native int selectedSlot();
    /** Local player's game mode id (0 survival, 1 creative, 2 adventure, 3 spectator). */
    public static native int gameMode();
    /** The local player's operator permission level (0-4, from the server's entity events). */
    public static native int permissionLevel();
    /** The local player's name. */
    public static native String playerName();

    /** Fills the mirror arrays with Rust's primitive player and debug state (layout: haste.gui.Mirror). */
    public static native void mirror(int[] ints, double[] doubles);

    // ---- Java -> Rust ----
    /**
     * A player inventory slot changed in Java (vanilla numbering; -1 = the stack carried by the cursor). The stack is
     * in vanilla's network encoding (count, item id, component patch).
     */
    public static native void inventorySlot(int slot, byte[] stack);
    /** A serverbound play packet vanilla code sent: VarInt id + body, vanilla's encoding. */
    public static native void sendPacket(byte[] idAndBody);
    /** Vanilla code asked to leave the server (pause screen "Disconnect", death screen "Title Screen"). */
    public static native void requestDisconnect();
    /** Called when a screen sends a chat message or command (Rust signs and sends it). */
    public static native void sendChat(String message, boolean command);

    // ---- menu mode (the connection is Rust's; the screens ask for it) ----
    /** The multiplayer screen's Join was clicked: connect to this "host[:port]" (Rust owns the connection). */
    public static native void requestJoin(String address);
    /** The connecting screen's Cancel: Rust drops a join still in progress. */
    public static native void cancelJoin();
    /** Vanilla code asked to quit the game (the title screen's Quit Game). */
    public static native void requestQuit();
    /** Ask Rust to server-list-ping this "host[:port]" (haste-protocol); the result arrives through {@link #takePingResults}. */
    public static native void startPing(String address);
    /**
     * Finished ping results, oldest first, as "address\nstatus-json\nping-millis" lines (an empty
     * status-json means the ping failed). One call drains the queue.
     */
    public static native String[] takePingResults();

    // ---- singleplayer (the local server is Rust's; the screens ask for it) ----
    /** The world list's "Play Selected World": start the local server for this world folder and join it. */
    public static native void requestOpenWorld(String levelId);
    /**
     * The create screen's "Create New World": make the world with these settings on the local
     * server and join it. {@code gameMode}/{@code difficulty} are vanilla's ids (0-3),
     * {@code levelType} a world-preset id like "minecraft:normal", {@code seed} the decimal seed
     * (the world folder is the target; an empty seed means random).
     */
    public static native void requestCreateWorld(String folder, String name, int gameMode, int difficulty,
        boolean hardcore, boolean allowCommands, String seed, boolean generateStructures, String levelType);
    /** Whether the local singleplayer server is running (the pause screen's button label). */
    public static native boolean localWorldRunning();
}
