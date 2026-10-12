package haste.testmod.mixin;

import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Default priority (1000): increments a real Minecraft int field every tick. */
@Mixin(Minecraft.class)
public abstract class MinecraftMixinA {
    @Shadow private int rightClickDelay;

    @Inject(method = "tick", at = @At("HEAD"))
    private void haste$countTick(CallbackInfo ci) {
        this.rightClickDelay++;
    }
}
