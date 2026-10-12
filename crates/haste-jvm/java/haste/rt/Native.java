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
    /** Called when a screen sends a chat message or command (Rust signs and sends it). */
    public static native void sendChat(String message, boolean command);
}
