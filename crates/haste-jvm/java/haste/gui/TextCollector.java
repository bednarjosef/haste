package haste.gui;

import haste.rt.GuiBatch;
import net.minecraft.client.gui.ActiveTextCollector;
import net.minecraft.client.gui.TextAlignment;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.navigation.ScreenRectangle;
import net.minecraft.network.chat.Component;
import net.minecraft.util.FormattedCharSequence;

/**
 * The {@link ActiveTextCollector} widgets draw their labels through. Positioning and scrolling are vanilla's own
 * default methods ({@code defaultScrollingHelper}, {@code TextAlignment.calculateLeft}); only the final
 * {@code accept} is ours and appends TEXT commands (with a scissor when the parameters carry one).
 */
public final class TextCollector implements ActiveTextCollector {
    static final TextCollector DEFAULT = new TextCollector();
    private static final TextCollector WIDGET = new TextCollector();

    private Parameters params;
    private int color = -1;

    /**
     * Widget labels: white, or grey for inactive buttons (vanilla's {@code AbstractWidget.getColor} returns
     * the inactive grey only where the widget's own rendering consults {@code active}; the pause screen's
     * title is an inactive {@code StringWidget} that vanilla draws white, so only button-like widgets grey).
     */
    static TextCollector forWidget(AbstractWidget w) {
        boolean button = w instanceof net.minecraft.client.gui.components.AbstractButton;
        WIDGET.color = w != null && !w.active && button ? 0xFFA0A0A0 : -1;
        WIDGET.params = null;
        return WIDGET;
    }

    @Override
    public Parameters defaultParameters() {
        if (params == null) params = new Parameters(GfxImpl.POSE);
        return params;
    }

    @Override
    public void defaultParameters(Parameters p) { params = p; }

    @Override
    public void accept(TextAlignment alignment, int anchorX, int y, Parameters p, FormattedCharSequence text) {
        int w = (int) Math.ceil(Text.width(text));
        int x = alignment.calculateLeft(anchorX, w);
        float opacity = p == null ? 1f : p.opacity();
        int c = opacity >= 1f ? color : ((Math.round(opacity * ((color >>> 24) & 0xFF)) & 0xFF) << 24) | (color & 0xFFFFFF);
        ScreenRectangle s = p == null ? null : p.scissor();
        GfxImpl.POSE.sync();
        if (s != null) GuiBatch.op(GuiBatch.SCISSOR_PUSH, s.left(), s.top(), s.right(), s.bottom());
        Text.emit(text, x, y, c, true);
        if (s != null) GuiBatch.op(GuiBatch.SCISSOR_POP);
    }

    @Override
    public void acceptScrolling(Component message, int centerX, int left, int right, int top, int bottom, Parameters p) {
        int w = (int) Math.ceil(Text.width(message.getVisualOrderText()));
        defaultScrollingHelper(message, centerX, left, right, top, bottom, w, haste.rt.Native.lineHeight(), p);
    }
}
