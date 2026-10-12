package haste.gui;

import haste.rt.Native;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.ChatComponent;
import net.minecraft.util.ArrayListDeque;

/**
 * Bodies of the {@code ChatComponent} proxy. The chat log is Rust state (received messages arrive through the
 * protocol, WS1); Java only keeps the up-arrow history of lines typed this session, like vanilla's recent chat.
 */
public final class ChatImpl {
    private ChatImpl() {}

    private static final ArrayListDeque<String> RECENT = new ArrayListDeque<>(100);

    public static ArrayListDeque<String> getRecentChat(ChatComponent c) { return RECENT; }

    public static void addRecentChat(ChatComponent c, String s) {
        if (!RECENT.isEmpty() && RECENT.peekLast().equals(s)) return;
        if (RECENT.size() >= 100) RECENT.removeFirst();
        RECENT.addLast(s);
    }

    /**
     * Draws the Rust chat log (newest at the bottom) above the input line. Layout is approximate (vanilla's
     * default chat width 320 and 9 px lines, opaque-ish backdrop); styles and per-message fading are not
     * represented. See NOTES.md.
     */
    public static void extractRenderState(ChatComponent c, GuiGraphicsExtractor g, Font font, int tickCount, int mouseX, int mouseY,
                                          ChatComponent.DisplayMode mode, boolean focused) {
        String[] lines = Native.chatLines();
        if (lines == null || lines.length == 0) return;
        int bottom = GfxImpl.height - 40;
        int maxLines = focused ? 20 : 10;
        int n = Math.min(lines.length, maxLines);
        for (int i = 0; i < n; i++) {
            String line = lines[lines.length - 1 - i];
            int y = bottom - (i + 1) * LINE;
            GfxImpl.fill(g, 0, y, WIDTH + 4, y + LINE, 0x80000000);
            GfxImpl.text(g, font, line, 4, y + 1, 0xFFFFFFFF, true);
        }
    }

    static final int WIDTH = 320, LINE = 9;

    public static boolean isChatFocused(ChatComponent c) { return ClientImpl.screen instanceof net.minecraft.client.gui.screens.ChatScreen; }
}
