package haste.gui;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.Panorama;

/**
 * The title screen's panorama, the one object the game renderer hands out. It keeps the spin state of the
 * background; Rust's renderer draws the rotating cube map later, so {@code extractRenderState} puts
 * nothing into a frame yet (only the spin angle can be advanced there).
 */
public final class PanoramaImpl {
    private PanoramaImpl() {}

    private static Panorama instance;
    private static boolean spinning;

    /** Vanilla's {@code GameRenderer.panorama()}: the single panorama of the game. */
    public static Panorama panorama(GameRenderer g) {
        if (instance == null) instance = Proxies.make(Panorama.class);
        return instance;
    }

    public static void startSpin(Panorama p) { spinning = true; }

    public static void holdSpin(Panorama p) { spinning = false; }

    /** Vanilla's body advances the spin and writes the cube map into the render state; drawn later. */
    public static void extractRenderState(Panorama p, GuiGraphicsExtractor graphics, int width, int height) {}
}
