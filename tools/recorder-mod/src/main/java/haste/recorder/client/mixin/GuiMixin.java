package haste.recorder.client.mixin;

import haste.recorder.client.HasteHooks;
import net.minecraft.client.gui.Gui;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

/**
 * While {@link HasteHooks#hideGui} is set (screenshot capture), the HUD is skipped: vanilla's
 * F1 behaviour, programmatically and only for the captured frames. The second boolean
 * (renderScreen) is untouched: no screen is open at capture time anyway. The debug overlay is
 * NOT gated by this flag in vanilla (keep F3 closed while capturing; docs say so).
 */
@Mixin(Gui.class)
public abstract class GuiMixin {
    @ModifyVariable(
        method = "extractRenderState(Lnet/minecraft/client/DeltaTracker;ZZ)V",
        at = @At("HEAD"),
        ordinal = 0,
        argsOnly = true
    )
    private boolean haste$hideHudWhenCapturing(boolean renderHud) {
        return renderHud && !HasteHooks.hideGui;
    }
}
