package haste.gui;

import net.minecraft.client.Camera;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.world.phys.Vec3;

/**
 * Bodies of the {@code GameRenderer} and {@code Camera} proxies. Rendering and the camera are Rust's (renderer,
 * client); the HUD only asks where the camera is and whether it is first person. Answers come from the mirrored
 * player state: a first-person camera at the player's eyes (standing height), attached to the local player.
 */
public final class CameraImpl {
    private CameraImpl() {}

    public static Camera mainCamera(GameRenderer r) { return Proxies.camera; }

    /** The render state vanilla code writes into (plain data; Rust renders from GuiFrame, not from this). */
    static final net.minecraft.client.renderer.state.GameRenderState STATE = gameState();
    private static net.minecraft.client.renderer.state.GameRenderState gameState() {
        var s = Proxies.make(net.minecraft.client.renderer.state.GameRenderState.class);
        Proxies.set(s, "guiRenderState", net.minecraft.client.renderer.state.gui.GuiRenderState.class, new net.minecraft.client.renderer.state.gui.GuiRenderState());
        return s;
    }
    public static net.minecraft.client.renderer.state.GameRenderState gameRenderState(GameRenderer r) { return STATE; }

    public static net.minecraft.world.entity.Entity entity(Camera c) { return Proxies.player; }
    public static boolean isInitialized(Camera c) { return true; }
    public static boolean isDetached(Camera c) { return false; }
    public static Vec3 position(Camera c) {
        return new Vec3(Mirror.D[Mirror.D_X], Mirror.D[Mirror.D_Y] + 1.62, Mirror.D[Mirror.D_Z]);
    }
    public static net.minecraft.core.BlockPos blockPosition(Camera c) { return net.minecraft.core.BlockPos.containing(position(c)); }
    public static float xRot(Camera c) { return (float) Mirror.D[Mirror.D_PITCH]; }
    public static float yRot(Camera c) { return (float) Mirror.D[Mirror.D_YAW]; }
    public static float getFov(Camera c) { return 70f; }
    /** The fluid at the camera (water/lava fog overlays): only water is known (Rust's eyes-in-water flag). */
    public static net.minecraft.world.level.material.FogType getFluidInCamera(Camera c) {
        return Mirror.flag(Mirror.F_EYE_IN_WATER) ? net.minecraft.world.level.material.FogType.WATER : net.minecraft.world.level.material.FogType.NONE;
    }
}
