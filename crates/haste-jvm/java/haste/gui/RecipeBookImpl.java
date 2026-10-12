package haste.gui;

import net.minecraft.client.gui.screens.recipebook.RecipeBookComponent;

/**
 * The few {@code RecipeBookComponent} bodies that matter while the book is closed (it never opens: recipes are
 * not bridged). Everything else on the proxy is a no-op.
 */
public final class RecipeBookImpl {
    private RecipeBookImpl() {}

    /** A closed book leaves the container centred. */
    public static int updateScreenPosition(RecipeBookComponent<?> b, int width, int imageWidth) { return (width - imageWidth) / 2; }

    /** A closed book: outside means outside the container image. */
    public static boolean hasClickedOutside(RecipeBookComponent<?> b, double mx, double my, int leftPos, int topPos, int imageWidth, int imageHeight) {
        return mx < leftPos || my < topPos || mx >= leftPos + imageWidth || my >= topPos + imageHeight;
    }

    public static net.minecraft.client.gui.narration.NarratableEntry.NarrationPriority narrationPriority(RecipeBookComponent<?> b) {
        return net.minecraft.client.gui.narration.NarratableEntry.NarrationPriority.NONE;
    }
}
