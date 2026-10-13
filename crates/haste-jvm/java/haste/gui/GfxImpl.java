package haste.gui;

import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import haste.rt.GuiBatch;
import haste.rt.GuiBlend;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.gui.ActiveTextCollector;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.Renderable;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.resources.Identifier;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.item.ItemStack;
import org.joml.Matrix3x2fStack;

/**
 * Bodies of the {@code GuiGraphicsExtractor} proxy (wired by name/descriptor through {@code impl} in
 * proxies.conf). Every draw call appends to {@link GuiBatch}; nothing crosses JNI until Rust takes the frame.
 * There is one canonical extractor proxy; the per-frame state lives here.
 */
public final class GfxImpl {
    private GfxImpl() {}

    static final Pose POSE = new Pose();
    static int width, height;
    private static final int[] scissor = new int[4 * 32];
    private static int scissorDepth;
    private static List<FormattedCharSequence> tooltip;
    private static int tooltipX, tooltipY;

    /** Starts a frame of the given GUI size. */
    static void begin(int w, int h) {
        GuiBatch.reset();
        POSE.reset();
        width = w; height = h;
        scissorDepth = 0;
        tooltip = null;
        cursor = 0;
        entityBoxes.clear();
    }

    /** Ends a frame: deferred tooltip, then closes open translations and scissors. */
    static void end() {
        if (tooltip != null) drawTooltip();
        POSE.finish();
        while (scissorDepth > 0) { scissorDepth--; GuiBatch.op(GuiBatch.SCISSOR_POP); }
    }

    // ---- geometry ----
    public static int guiWidth(GuiGraphicsExtractor g) { return width; }
    public static int guiHeight(GuiGraphicsExtractor g) { return height; }
    public static Matrix3x2fStack pose(GuiGraphicsExtractor g) { return POSE; }
    public static void nextStratum(GuiGraphicsExtractor g) {}
    public static void blurBeforeThisStratum(GuiGraphicsExtractor g) {}
    /** The cursor the screen asked for this frame (GuiBatch.frame's code; 0 = default). */
    static int cursor;
    public static void requestCursor(GuiGraphicsExtractor g, com.mojang.blaze3d.platform.cursor.CursorType c) {
        cursor = cursorCode(c);
    }
    private static int cursorCode(com.mojang.blaze3d.platform.cursor.CursorType c) {
        if (c == null) return 0;
        com.mojang.blaze3d.platform.cursor.CursorType[] order = {
            com.mojang.blaze3d.platform.cursor.CursorTypes.ARROW, com.mojang.blaze3d.platform.cursor.CursorTypes.IBEAM,
            com.mojang.blaze3d.platform.cursor.CursorTypes.CROSSHAIR, com.mojang.blaze3d.platform.cursor.CursorTypes.POINTING_HAND,
            com.mojang.blaze3d.platform.cursor.CursorTypes.RESIZE_NS, com.mojang.blaze3d.platform.cursor.CursorTypes.RESIZE_EW,
            com.mojang.blaze3d.platform.cursor.CursorTypes.RESIZE_ALL, com.mojang.blaze3d.platform.cursor.CursorTypes.NOT_ALLOWED };
        for (int i = 0; i < order.length; i++) if (order[i] == c) return i + 1;
        return 0;
    }

    public static void enableScissor(GuiGraphicsExtractor g, int x1, int y1, int x2, int y2) {
        POSE.sync();
        if (scissorDepth < 32) {
            int b = scissorDepth * 4;
            // kept in absolute GUI coordinates for containsPointInScissor
            scissor[b] = x1 + (int) POSE.tx(); scissor[b + 1] = y1 + (int) POSE.ty();
            scissor[b + 2] = x2 + (int) POSE.tx(); scissor[b + 3] = y2 + (int) POSE.ty();
            scissorDepth++;
        }
        GuiBatch.op(GuiBatch.SCISSOR_PUSH, x1, y1, x2, y2);
    }

    public static void disableScissor(GuiGraphicsExtractor g) {
        if (scissorDepth > 0) scissorDepth--;
        GuiBatch.op(GuiBatch.SCISSOR_POP);
    }

    public static boolean containsPointInScissor(GuiGraphicsExtractor g, int x, int y) {
        if (scissorDepth == 0) return true;
        int b = (scissorDepth - 1) * 4;
        return x >= scissor[b] && x < scissor[b + 2] && y >= scissor[b + 1] && y < scissor[b + 3];
    }

    // ---- rectangles ----
    public static void fill(GuiGraphicsExtractor g, int x1, int y1, int x2, int y2, int color) {
        POSE.sync();
        if (x1 > x2) { int t = x1; x1 = x2; x2 = t; }
        if (y1 > y2) { int t = y1; y1 = y2; y2 = t; }
        GuiBatch.op(GuiBatch.FILL, x1, y1, x2, y2, color);
    }

    public static void fill(GuiGraphicsExtractor g, RenderPipeline p, int x1, int y1, int x2, int y2, int color) {
        fill(g, x1, y1, x2, y2, color);
    }

    public static void fillGradient(GuiGraphicsExtractor g, int x1, int y1, int x2, int y2, int top, int bottom) {
        POSE.sync();
        GuiBatch.op(GuiBatch.GRADIENT, x1, y1, x2, y2, top, bottom);
    }

    public static void horizontalLine(GuiGraphicsExtractor g, int x1, int x2, int y, int color) {
        if (x2 < x1) { int t = x1; x1 = x2; x2 = t; }
        fill(g, x1, y, x2 + 1, y + 1, color);
    }

    public static void verticalLine(GuiGraphicsExtractor g, int x, int y1, int y2, int color) {
        if (y2 < y1) { int t = y1; y1 = y2; y2 = t; }
        fill(g, x, y1 + 1, x + 1, y2, color);
    }

    public static void outline(GuiGraphicsExtractor g, int x, int y, int w, int h, int color) {
        fill(g, x, y, x + w, y + 1, color);
        fill(g, x, y + h - 1, x + w, y + h, color);
        fill(g, x, y + 1, x + 1, y + h - 1, color);
        fill(g, x + w - 1, y + 1, x + w, y + h - 1, color);
    }

    /** Text selection highlight (EditBox): an inverted-looking fill; approximated as a translucent blue fill. */
    public static void textHighlight(GuiGraphicsExtractor g, int x1, int y1, int x2, int y2, boolean invert) {
        fill(g, x1, y1, x2, y2, invert ? 0xFF0000FF : 0x800000FF);
    }

    // ---- text ----
    public static void text(GuiGraphicsExtractor g, Font f, String s, int x, int y, int color) { text(g, f, s, x, y, color, true); }
    public static void text(GuiGraphicsExtractor g, Font f, String s, int x, int y, int color, boolean shadow) {
        POSE.sync();
        Text.emit(s, x, y, color, shadow);
    }
    public static void text(GuiGraphicsExtractor g, Font f, FormattedCharSequence s, int x, int y, int color) { text(g, f, s, x, y, color, true); }
    public static void text(GuiGraphicsExtractor g, Font f, FormattedCharSequence s, int x, int y, int color, boolean shadow) {
        POSE.sync();
        Text.emit(s, x, y, color, shadow);
    }
    public static void text(GuiGraphicsExtractor g, Font f, Component s, int x, int y, int color) { text(g, f, s, x, y, color, true); }
    public static void text(GuiGraphicsExtractor g, Font f, Component s, int x, int y, int color, boolean shadow) {
        POSE.sync();
        Text.emit(s, x, y, color, shadow);
    }

    public static void centeredText(GuiGraphicsExtractor g, Font f, String s, int x, int y, int color) {
        text(g, f, s, x - FontImpl.width(f, s) / 2, y, color, true);
    }
    public static void centeredText(GuiGraphicsExtractor g, Font f, Component s, int x, int y, int color) {
        FormattedCharSequence seq = s.getVisualOrderText();
        text(g, f, seq, x - FontImpl.width(f, seq) / 2, y, color, true);
    }
    public static void centeredText(GuiGraphicsExtractor g, Font f, FormattedCharSequence s, int x, int y, int color) {
        text(g, f, s, x - FontImpl.width(f, s) / 2, y, color, true);
    }

    public static int textWithWordWrap(GuiGraphicsExtractor g, Font f, FormattedText t, int x, int y, int w, int color) {
        return textWithWordWrap(g, f, t, x, y, w, color, true);
    }
    public static int textWithWordWrap(GuiGraphicsExtractor g, Font f, FormattedText t, int x, int y, int w, int color, boolean shadow) {
        for (FormattedCharSequence line : FontImpl.split(f, t, w)) {
            text(g, f, line, x, y, color, shadow);
            y += f.lineHeight;
        }
        return y;
    }

    public static void textWithBackdrop(GuiGraphicsExtractor g, Font f, Component c, int x, int y, int w, int color) {
        // backdrop colour comes from the accessibility option in vanilla; approximated as the chat default
        fill(g, x - 2, y - 2, x + w + 2, y + f.lineHeight + 2, 0x80000000);
        text(g, f, c, x, y, color, true);
    }

    public static ActiveTextCollector textRenderer(GuiGraphicsExtractor g) { return TextCollector.DEFAULT; }
    public static ActiveTextCollector textRenderer(GuiGraphicsExtractor g, GuiGraphicsExtractor.HoveredTextEffects e) { return TextCollector.DEFAULT; }
    public static ActiveTextCollector textRenderer(GuiGraphicsExtractor g, GuiGraphicsExtractor.HoveredTextEffects e, java.util.function.Consumer<net.minecraft.network.chat.Style> c) { return TextCollector.DEFAULT; }
    public static ActiveTextCollector textRendererForWidget(GuiGraphicsExtractor g, AbstractWidget w, GuiGraphicsExtractor.HoveredTextEffects e) {
        return TextCollector.forWidget(w);
    }

    // ---- sprites and textures ----
    private static int id(Identifier i) { return GuiBatch.str(i.toString()); }

    /**
     * The blend mode of a blit, by identifier: vanilla picks the {@code RenderPipeline} per draw call, but the
     * {@code p} arguments are null here ({@code RenderPipelines} has no {@code <clinit>}), and every GUI pipeline
     * except two blends normally — so the identifier says which pipeline it was: {@code textures/misc/vignette.png}
     * is {@code RenderPipelines.VIGNETTE} (multiplicative: darkens the edges) and the crosshair sprites
     * ({@code hud/crosshair...}) are drawn with {@code RenderPipelines.CROSSHAIR} (inverts the colour behind it).
     * Codes: 0 normal, 1 multiply, 2 invert.
     */
    private static int blend(Identifier t) {
        String p = t.getPath();
        if (p.endsWith("vignette.png")) return GuiBlend.MULTIPLY;
        if (p.equals("hud/crosshair") || p.startsWith("hud/crosshair_")) return GuiBlend.INVERT;
        return GuiBlend.NORMAL;
    }

    public static void blitSprite(GuiGraphicsExtractor g, RenderPipeline p, Identifier s, int x, int y, int w, int h) {
        blitSprite(g, p, s, x, y, w, h, -1);
    }
    public static void blitSprite(GuiGraphicsExtractor g, RenderPipeline p, Identifier s, int x, int y, int w, int h, float alpha) {
        int a = Math.round(Math.max(0f, Math.min(1f, alpha)) * 255f);
        blitSprite(g, p, s, x, y, w, h, (a << 24) | 0xFFFFFF);
    }
    public static void blitSprite(GuiGraphicsExtractor g, RenderPipeline p, Identifier s, int x, int y, int w, int h, int tint) {
        POSE.sync();
        GuiBatch.ops(GuiBatch.SPRITE, id(s), x, y, w, h, tint, blend(s));
    }
    /** Sub-region of a sprite: region (u, v, w, h) of a sprite drawn as {@code spriteW x spriteH}, placed at x, y. */
    public static void blitSprite(GuiGraphicsExtractor g, RenderPipeline p, Identifier s, int spriteW, int spriteH, int u, int v, int x, int y, int w, int h) {
        blitSprite(g, p, s, spriteW, spriteH, u, v, x, y, w, h, -1);
    }
    public static void blitSprite(GuiGraphicsExtractor g, RenderPipeline p, Identifier s, int spriteW, int spriteH, int u, int v, int x, int y, int w, int h, int tint) {
        POSE.sync();
        GuiBatch.ops(GuiBatch.SPRITE_REGION, id(s), spriteW, spriteH, u, v, x, y, w, h, tint, blend(s));
    }

    private static void texture(Identifier t, int x, int y, int w, int h, float u, float v, float uw, float vh, int texW, int texH, int tint) {
        POSE.sync();
        GuiBatch.ops(GuiBatch.TEXTURE, id(t), x, y, w, h, Float.floatToRawIntBits(u), Float.floatToRawIntBits(v),
            Float.floatToRawIntBits(uw), Float.floatToRawIntBits(vh), texW, texH, tint, blend(t));
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int texW, int texH) {
        texture(t, x, y, w, h, u, v, w, h, texW, texH, -1);
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int texW, int texH, int tint) {
        texture(t, x, y, w, h, u, v, w, h, texW, texH, tint);
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int uw, int vh, int texW, int texH) {
        texture(t, x, y, w, h, u, v, uw, vh, texW, texH, -1);
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int uw, int vh, int texW, int texH, int tint) {
        texture(t, x, y, w, h, u, v, uw, vh, texW, texH, tint);
    }
    /** blit(texture, x1, y1, x2, y2, u0, u1, v0, v1) with normalised UVs. */
    public static void blit(GuiGraphicsExtractor g, Identifier t, int x1, int y1, int x2, int y2, float u0, float u1, float v0, float v1) {
        texture(t, x1, y1, x2 - x1, y2 - y1, u0, v0, u1 - u0, v1 - v0, 1, 1, -1);
    }

    // ---- items ----
    public static void item(GuiGraphicsExtractor g, ItemStack s, int x, int y) { item(g, s, x, y, 0); }
    public static void item(GuiGraphicsExtractor g, ItemStack s, int x, int y, int seed) {
        if (s == null || s.isEmpty()) return;
        POSE.sync();
        GuiBatch.op(GuiBatch.ITEM, itemId(s), x, y, s.getCount());
    }

    /** Index of the stack's item in the {@code minecraft:item} registry (the same numbering as Rust's). */
    static int itemId(ItemStack s) { return net.minecraft.core.registries.BuiltInRegistries.ITEM.getId(s.getItem()); }
    public static void item(GuiGraphicsExtractor g, net.minecraft.world.entity.LivingEntity e, ItemStack s, int x, int y, int seed) { item(g, s, x, y, seed); }
    public static void fakeItem(GuiGraphicsExtractor g, ItemStack s, int x, int y) { item(g, s, x, y, 0); }
    public static void fakeItem(GuiGraphicsExtractor g, ItemStack s, int x, int y, int seed) { item(g, s, x, y, seed); }

    /**
     * Stack decorations: the count in the bottom-right corner (white, shadow) and the durability bar (13 px wide,
     * 2 px high, black under the item's bar colour), as they appear in the game. Cooldown overlays: not yet.
     */
    public static void itemDecorations(GuiGraphicsExtractor g, Font f, ItemStack s, int x, int y) { itemDecorations(g, f, s, x, y, null); }
    public static void itemDecorations(GuiGraphicsExtractor g, Font f, ItemStack s, int x, int y, String text) {
        if (s == null || s.isEmpty()) return;
        POSE.sync();
        if (s.isBarVisible()) {
            int bx = x + 2, by = y + 13;
            GuiBatch.op(GuiBatch.FILL, bx, by, bx + 13, by + 2, 0xFF000000);
            GuiBatch.op(GuiBatch.FILL, bx, by, bx + s.getBarWidth(), by + 1, 0xFF000000 | s.getBarColor());
        }
        int count = s.getCount();
        String label = text != null ? text : (count != 1 ? Integer.toString(count) : null);
        if (label == null) return;
        Text.emit(label, x + 19 - 2 - FontImpl.width(f, label), y + 6 + 3, -1, true);
    }

    // ---- tooltips (deferred to the end of the frame) ----
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, Component c, int x, int y) {
        tooltip = List.of(c.getVisualOrderText()); tooltipX = x; tooltipY = y;
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, Component c, int x, int y, Identifier bg) {
        setTooltipForNextFrame(g, f, c, x, y);
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<? extends FormattedCharSequence> lines, int x, int y) {
        tooltip = new ArrayList<>(lines); tooltipX = x; tooltipY = y;
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<? extends FormattedCharSequence> lines, int x, int y, Identifier bg) {
        setTooltipForNextFrame(g, f, lines, x, y);
    }
    public static void setComponentTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<Component> lines, int x, int y) {
        List<FormattedCharSequence> l = new ArrayList<>();
        for (Component c : lines) l.add(c.getVisualOrderText());
        tooltip = l; tooltipX = x; tooltipY = y;
    }
    public static void setComponentTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<Component> lines, int x, int y, Identifier bg) {
        setComponentTooltipForNextFrame(g, f, lines, x, y);
    }
    /** Component tooltips with an optional image (bundles, maps: not drawn) and a style sprite (ignored). */
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<Component> lines,
            java.util.Optional<net.minecraft.world.inventory.tooltip.TooltipComponent> image, int x, int y) {
        setComponentTooltipForNextFrame(g, f, lines, x, y);
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<Component> lines,
            java.util.Optional<net.minecraft.world.inventory.tooltip.TooltipComponent> image, int x, int y, Identifier style) {
        setComponentTooltipForNextFrame(g, f, lines, x, y);
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<Component> lines,
            java.util.Optional<net.minecraft.world.inventory.tooltip.TooltipComponent> image, int x, int y, Identifier style, boolean extraSpace) {
        setComponentTooltipForNextFrame(g, f, lines, x, y);
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<FormattedCharSequence> lines,
            net.minecraft.client.gui.screens.inventory.tooltip.ClientTooltipPositioner pos, int x, int y, boolean focused) {
        setTooltipForNextFrame(g, f, lines, x, y);
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, List<FormattedCharSequence> lines, int x, int y) {
        tooltip = new ArrayList<>(lines); tooltipX = x; tooltipY = y;
    }
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Component c, int x, int y) {
        tooltip = List.of(c.getVisualOrderText()); tooltipX = x; tooltipY = y;
    }

    /** An item's tooltip: vanilla's own lines ({@code Screen.getTooltipFromItem}: name, components, enchantments). */
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, ItemStack s, int x, int y) {
        if (s == null || s.isEmpty()) return;
        setComponentTooltipForNextFrame(g, f, net.minecraft.client.gui.screens.Screen.getTooltipFromItem(Proxies.minecraft, s), x, y);
    }
    public static void extractDeferredElements(GuiGraphicsExtractor g, int mx, int my, float partial) {}

    /** Tooltip box: background sprite + lines, offset from the cursor like vanilla (12, -12). Approximate. */
    private static void drawTooltip() {
        int w = 0;
        for (FormattedCharSequence l : tooltip) w = Math.max(w, (int) Math.ceil(Text.width(l)));
        int h = tooltip.size() * 10 - 2;
        int x = tooltipX + 12, y = tooltipY - 12;
        if (x + w + 4 > width) x = Math.max(4, tooltipX - 16 - w);
        if (y + h + 4 > height) y = height - h - 4;
        if (y < 4) y = 4;
        POSE.reset();
        GuiBatch.ops(GuiBatch.SPRITE, GuiBatch.str("minecraft:tooltip/background"), x - 3 - 9, y - 3 - 9, w + 6 + 18, h + 6 + 18, -1, GuiBlend.NORMAL);
        GuiBatch.ops(GuiBatch.SPRITE, GuiBatch.str("minecraft:tooltip/frame"), x - 3 - 9, y - 3 - 9, w + 6 + 18, h + 6 + 18, -1, GuiBlend.NORMAL);
        for (FormattedCharSequence l : tooltip) { Text.emit(l, x, y, -1, true); y += 10; }
        tooltip = null;
    }

    /**
     * An entity model in a GUI box (the inventory's player): GuiCommand::Entity with the local player's entity id,
     * the box, the scale and the mouse position the model looks at (Rust renders the model).
     */
    static final List<int[]> entityBoxes = new ArrayList<>();
    public static void entity(GuiGraphicsExtractor g, net.minecraft.client.renderer.entity.state.EntityRenderState s, float scale,
                              org.joml.Vector3fc translation, org.joml.Quaternionfc rotation, org.joml.Quaternionfc cameraAngle,
                              int x0, int y0, int x1, int y1) {
        entityBoxes.add(new int[]{x0 + (int) POSE.tx(), y0 + (int) POSE.ty(), x1 + (int) POSE.tx(), y1 + (int) POSE.ty()});
        POSE.sync();
        GuiBatch.ops(GuiBatch.ENTITY, Handlers.localEntityId, x0, y0, x1, y1, Float.floatToRawIntBits(scale),
            Float.floatToRawIntBits((float) MouseImpl.x), Float.floatToRawIntBits((float) MouseImpl.y));
    }

    public static void setPreeditOverlay(GuiGraphicsExtractor g, Renderable r) {}
}
