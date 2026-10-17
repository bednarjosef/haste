package haste.gui;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.Panorama;
import net.minecraft.client.renderer.RenderPipelines;

/**
 * The title screen's panorama, the one object the game renderer hands out (vanilla {@code Panorama}). It keeps
 * the spin of the background; {@code extractRenderState} advances it and records it for the frame, and Rust's
 * renderer draws the rotating cube map under the frame's GUI commands (vanilla {@code GuiRenderer.render} ->
 * {@code CubeMap.render}).
 */
public final class PanoramaImpl {
    private PanoramaImpl() {}

    private static Panorama instance;
    /** Vanilla {@code Panorama.shouldSpin}: true until a screen holds it. */
    private static boolean spinning = true;
    /** Vanilla {@code Panorama.spin}, in degrees, wrapped to [-180, 180). */
    private static float spin;

    /** Vanilla's {@code GameRenderer.panorama()}: the single panorama of the game. */
    public static Panorama panorama(GameRenderer g) {
        if (instance == null) instance = Proxies.make(Panorama.class);
        return instance;
    }

    public static void startSpin(Panorama p) { spinning = true; }

    public static void holdSpin(Panorama p) { spinning = false; }

    /**
     * Vanilla {@code Panorama.extractRenderState}: while spinning, the spin grows by a tenth of the frame's
     * real-time delta in ticks times the panorama speed option; the frame's panorama state is the negated
     * spin, and the overlay texture is blitted over the whole screen.
     */
    public static void extractRenderState(Panorama p, GuiGraphicsExtractor graphics, int width, int height) {
        if (spinning) {
            float delta = (float) (Delta.INSTANCE.getRealtimeDeltaTicks() * Proxies.options.panoramaSpeed().get());
            spin = net.minecraft.util.Mth.wrapDegrees(spin + delta * 0.1F);
        }
        GfxImpl.panorama = true;
        GfxImpl.panoramaSpin = -spin;
        graphics.blit(RenderPipelines.GUI_TEXTURED, Panorama.PANORAMA_OVERLAY, 0, 0, 0.0F, 0.0F, width, height, 16, 128, 16, 128);
    }
}
