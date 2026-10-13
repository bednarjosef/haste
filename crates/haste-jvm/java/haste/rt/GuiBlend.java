package haste.rt;

/**
 * The blend mode of a GUI blit, as {@code GuiBatch} records it (mirrors {@code haste_core::gui::GuiBlend}):
 * vanilla picks one per draw call through the {@code RenderPipeline} argument; Haste's proxies see {@code null}
 * there, so {@code GfxImpl} derives it from the texture or sprite that is drawn.
 */
public final class GuiBlend {
    private GuiBlend() {}

    public static final int NORMAL = 0;
    public static final int MULTIPLY = 1;
    public static final int INVERT = 2;
}
