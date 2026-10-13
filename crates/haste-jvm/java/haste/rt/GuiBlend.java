package haste.rt;

/**
 * The blend mode of a GUI blit, as {@code GuiBatch} records it (mirrors {@code haste_core::gui::GuiBlend}):
 * vanilla picks one per draw call through the {@code RenderPipeline} argument, and {@code GfxImpl} reads it
 * from that pipeline ({@code haste.gui.Pipelines}); the texture name is only the fallback for a null one.
 */
public final class GuiBlend {
    private GuiBlend() {}

    public static final int NORMAL = 0;
    public static final int MULTIPLY = 1;
    public static final int INVERT = 2;
}
