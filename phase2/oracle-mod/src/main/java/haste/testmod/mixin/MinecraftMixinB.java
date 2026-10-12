package haste.testmod.mixin;

import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Higher priority (1500): cancels every 5th tick, tracking ticks in a second shadowed int. */
@Mixin(value = Minecraft.class, priority = 1500)
public abstract class MinecraftMixinB {
    @Shadow protected int missTime;

    @Inject(method = "tick", at = @At("HEAD"), cancellable = true)
    private void haste$maybeCancel(CallbackInfo ci) {
        this.missTime++;
        if (this.missTime % 5 == 0) {
            ci.cancel();
        }
    }
}
