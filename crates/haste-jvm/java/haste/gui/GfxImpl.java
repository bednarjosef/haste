package haste.gui;

import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import haste.rt.GuiBatch;
import haste.rt.GuiBlend;
import java.util.ArrayList;
import java.util.Arrays;
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
    /**
     * Debug counters for the tests: how this frame's blits got their blend mode, and which pipelines decided
     * the non-normal ones (a blit comes from its pipeline when {@code p} is not null, see {@link #blend}).
     */
    static int pipelineBlits, nameBlits;
    private static final RenderPipeline[] SPECIAL = new RenderPipeline[8];
    private static final int[] SPECIAL_CODE = new int[8];
    private static final int[] SPECIAL_COUNT = new int[8];
    private static int specialCount;

    /** Starts a frame of the given GUI size. */
    static void begin(int w, int h) {
        GuiBatch.reset();
        POSE.reset();
        width = w; height = h;
        scissorDepth = 0;
        tooltip = null;
        cursor = 0;
        blurAfter = -1;
        panorama = false;
        entityBoxes.clear();
        pipelineBlits = 0;
        nameBlits = 0;
        Arrays.fill(SPECIAL, null);
        specialCount = 0;
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
    /**
     * The open screen asked to blur the world behind it (vanilla: the blur post chain runs here,
     * between the HUD and the screen, when the accessibility option is >= 1). Records the command
     * index the blur goes after; {@code GuiBatch.frame} reports it to Rust.
     */
    public static void blurBeforeThisStratum(GuiGraphicsExtractor g) {
        if (blurAfter < 0 && Proxies.options.getMenuBackgroundBlurriness() >= 1.0F) {
            blurAfter = GuiBatch.length();
        }
    }
    /** The batch index the blur goes after this frame, or -1 when no screen asked for one. */
    static int blurAfter;
    /**
     * Whether a screen extracted the panorama this frame and its spin (vanilla: the
     * {@code PanoramaRenderState} of {@code GuiRenderState}, reset every frame); {@code GuiBatch.frame}
     * reports both to Rust, whose renderer draws the cube map under the frame's commands.
     */
    static boolean panorama;
    static float panoramaSpin;
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
        if (s == null) return;
        POSE.sync();
        // vanilla GuiGraphicsExtractor.text(Font, String, ...) reorders the string through the language
        // (bidi), so right-to-left languages lay raw strings out like components
        Text.emit(net.minecraft.locale.Language.getInstance().getVisualOrder(net.minecraft.network.chat.FormattedText.of(s)),
            x, y, color, shadow);
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
     * The blend mode of a blit: vanilla picks one per draw call in the {@code RenderPipeline} the caller passes,
     * and Haste's tier-2 {@link Pipelines} give the GUI's blits a real pipeline, so the blend function is read
     * from there (vignette multiplies, crosshair inverts, the text selection highlight adds, everything else
     * blends normally). The identifier rule below is only the fallback for a draw call whose pipeline is still
     * null (Haste's own blits, or a caller that built none). Codes: 0 normal, 1 multiply, 2 invert, 3 additive.
     */
    private static int blend(RenderPipeline p, Identifier t) {
        if (p != null) {
            pipelineBlits++;
            int b = Pipelines.blendOf(p);
            if (b != GuiBlend.NORMAL) noteSpecial(p, b);
            return b;
        }
        nameBlits++;
        return blendByName(t);
    }

    /** Debug: collects the pipeline behind a multiply/invert blit, so a test can check where the mode came from. */
    private static void noteSpecial(RenderPipeline p, int b) {
        for (int i = 0; i < specialCount; i++) {
            if (SPECIAL[i] == p) { SPECIAL_COUNT[i]++; return; }
        }
        if (specialCount < SPECIAL.length) {
            SPECIAL[specialCount] = p;
            SPECIAL_CODE[specialCount] = b;
            SPECIAL_COUNT[specialCount] = 1;
            specialCount++;
        }
    }

    /** Debug: the blend modes of this frame, as "{@code <pipeline location> <code> <count>}" lines. */
    static String specialBlends() {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < specialCount; i++) {
            sb.append(SPECIAL[i].getLocation()).append(' ').append(SPECIAL_CODE[i])
                .append(' ').append(SPECIAL_COUNT[i]).append('\n');
        }
        return sb.toString();
    }

    /**
     * The blend mode of a blit without a pipeline, by identifier: every GUI pipeline except three blends
     * normally, so the identifier says which pipeline it was: {@code textures/misc/vignette.png} is
     * {@code RenderPipelines.VIGNETTE} (multiplicative: darkens the edges) and the crosshair sprites
     * ({@code hud/crosshair...}) are drawn with {@code RenderPipelines.CROSSHAIR} (inverts the colour behind it).
     */
    private static int blendByName(Identifier t) {
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
        GuiBatch.ops(GuiBatch.SPRITE, id(s), x, y, w, h, tint, blend(p, s));
    }
    /** Sub-region of a sprite: region (u, v, w, h) of a sprite drawn as {@code spriteW x spriteH}, placed at x, y. */
    public static void blitSprite(GuiGraphicsExtractor g, RenderPipeline p, Identifier s, int spriteW, int spriteH, int u, int v, int x, int y, int w, int h) {
        blitSprite(g, p, s, spriteW, spriteH, u, v, x, y, w, h, -1);
    }
    public static void blitSprite(GuiGraphicsExtractor g, RenderPipeline p, Identifier s, int spriteW, int spriteH, int u, int v, int x, int y, int w, int h, int tint) {
        POSE.sync();
        GuiBatch.ops(GuiBatch.SPRITE_REGION, id(s), spriteW, spriteH, u, v, x, y, w, h, tint, blend(p, s));
    }

    private static void texture(RenderPipeline p, Identifier t, int x, int y, int w, int h, float u, float v, float uw, float vh, int texW, int texH, int tint) {
        POSE.sync();
        GuiBatch.ops(GuiBatch.TEXTURE, id(t), x, y, w, h, Float.floatToRawIntBits(u), Float.floatToRawIntBits(v),
            Float.floatToRawIntBits(uw), Float.floatToRawIntBits(vh), texW, texH, tint, blend(p, t));
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int texW, int texH) {
        texture(p, t, x, y, w, h, u, v, w, h, texW, texH, -1);
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int texW, int texH, int tint) {
        texture(p, t, x, y, w, h, u, v, w, h, texW, texH, tint);
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int uw, int vh, int texW, int texH) {
        texture(p, t, x, y, w, h, u, v, uw, vh, texW, texH, -1);
    }
    public static void blit(GuiGraphicsExtractor g, RenderPipeline p, Identifier t, int x, int y, float u, float v, int w, int h, int uw, int vh, int texW, int texH, int tint) {
        texture(p, t, x, y, w, h, u, v, uw, vh, texW, texH, tint);
    }
    /** blit(texture, x1, y1, x2, y2, u0, u1, v0, v1) with normalised UVs (no pipeline: normal blending). */
    public static void blit(GuiGraphicsExtractor g, Identifier t, int x1, int y1, int x2, int y2, float u0, float u1, float v0, float v1) {
        texture(null, t, x1, y1, x2 - x1, y2 - y1, u0, v0, u1 - u0, v1 - v0, 1, 1, -1);
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
    /** Item tooltips hovered through {@code setTooltipForNextFrame(Font, ItemStack, ...)} land here with a positioner. */
    public static void setTooltipForNextFrame(GuiGraphicsExtractor g, Font f, List<FormattedCharSequence> lines,
            java.util.Optional<net.minecraft.world.inventory.tooltip.TooltipComponent> image,
            net.minecraft.client.gui.screens.inventory.tooltip.ClientTooltipPositioner pos, int x, int y, boolean focused,
            Identifier style) {
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

    /**
     * The enchanting table's book (vanilla: a 3D model in a GUI box, drawn by the entity renderer). Haste's
     * GUI batch has no model rendering yet, so nothing is drawn; the book model the screen built is the
     * replacement class ({@code haste/replace/.../BookModel.java}), whose shape matches this signature.
     */
    public static void book(GuiGraphicsExtractor g, net.minecraft.client.model.object.book.BookModel bookModel,
                             Identifier texture, float scale, float open, float flip, int x0, int y0, int x1, int y1) {}
}
