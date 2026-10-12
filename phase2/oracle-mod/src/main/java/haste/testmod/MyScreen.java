package haste.testmod;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.network.chat.Component;

/** A mod screen: a vanilla type the mod extends. Rust drives it through the Screen proxy. */
public class MyScreen extends Screen {
    public int presses;
    public boolean initialised;

    public MyScreen(Component title) { super(title); }

    @Override
    protected void init() { initialised = true; presses = 0; }

    @Override
    public void extractRenderState(GuiGraphicsExtractor g, int mouseX, int mouseY, float partialTick) {
        g.fill(0, 0, this.width, this.height, 0xFF000000);
        g.text(this.font, "presses=" + presses, 10, 10, 0xFFFFFF);
    }

    @Override
    public boolean keyPressed(KeyEvent event) {
        if (event.key() == 32 /* GLFW_KEY_SPACE */) { presses++; return true; }
        return false; // a real mod would call super.keyPressed(event): that would land in the Unimplemented log
    }
}
