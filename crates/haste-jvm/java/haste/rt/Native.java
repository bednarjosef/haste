package haste.rt;

/**
 * Every Java -> Rust native in one class (registered once by {@code haste_jvm::natives} with RegisterNatives).
 * Proxy bodies reach these through impl classes in {@code haste.gui}, passing the proxy's {@code haste$handle}.
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

    // ---- inventory / item stacks (Rust state, by handle) ----
    /** Item registry index of the stack with this handle, or -1 for an empty stack. */
    public static native int stackItem(long handle);
    /** Count of the stack with this handle (0 for empty). */
    public static native int stackCount(long handle);
    /** Writes the count back into Rust. */
    public static native void stackSetCount(long handle, int count);
    /** Item registry name (e.g. "minecraft:stone") for a registry index, or null. */
    public static native String itemName(int item);
    /** Number of slots the Rust inventory has. */
    public static native int inventorySize();
    /** Selected hotbar slot. */
    public static native int selectedSlot();

    /** A container click (menu slot numbering; input = vanilla ContainerInput ordinal = wire click type). */
    public static native void containerInput(int containerId, int slot, int button, int input);

    // ---- chat / player ----
    /** Local player's game mode id (0 survival, 1 creative, 2 adventure, 3 spectator). */
    public static native int gameMode();
    /** The chat history lines Rust provides (oldest first). */
    public static native String[] chatLines();
    /** Called when a screen sends a chat message or command. */
    public static native void sendChat(String message, boolean command);
    /** The local player's name. */
    public static native String playerName();
}
