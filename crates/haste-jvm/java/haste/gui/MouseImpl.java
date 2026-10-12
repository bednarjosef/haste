package haste.gui;

import com.mojang.blaze3d.platform.Window;
import net.minecraft.client.MouseHandler;

/**
 * Bodies of the {@code MouseHandler} proxy. Rust's window owns the real mouse; screens only ask for the position
 * and request grab/release (e.g. "Back to Game" grabs it), which Rust reads with {@code GuiPipeline::mouse_grabbed}.
 */
public final class MouseImpl {
    private MouseImpl() {}

    static boolean grabbed;
    static double x, y;      // GUI coordinates of the last mouse event
    static int pressed = -1; // button held, -1 none

    public static void grabMouse(MouseHandler m) { grabbed = true; }
    public static void releaseMouse(MouseHandler m) { grabbed = false; }
    public static boolean isMouseGrabbed(MouseHandler m) { return grabbed; }
    public static double xpos(MouseHandler m) { return x * ClientImpl.scale; }
    public static double ypos(MouseHandler m) { return y * ClientImpl.scale; }
    public static double getScaledXPos(MouseHandler m, Window w) { return x; }
    public static double getScaledYPos(MouseHandler m, Window w) { return y; }
    public static double getScaledXPos(Window w, double px) { return px / ClientImpl.scale; }
    public static double getScaledYPos(Window w, double py) { return py / ClientImpl.scale; }
    public static boolean isLeftPressed(MouseHandler m) { return pressed == 1; }
    public static boolean isMiddlePressed(MouseHandler m) { return pressed == 2; }
    public static boolean isRightPressed(MouseHandler m) { return pressed == 3; }
}
