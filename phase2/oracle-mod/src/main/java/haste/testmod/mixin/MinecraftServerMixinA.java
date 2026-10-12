package haste.testmod.mixin;

import haste.testmod.Stats;
import java.util.function.BooleanSupplier;
import net.minecraft.server.MinecraftServer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Server-side twin of MinecraftMixinA, so real Mixin on the real server tells us the ground truth. */
@Mixin(MinecraftServer.class)
public abstract class MinecraftServerMixinA {
    @Inject(method = "tickServer", at = @At("HEAD"))
    private void haste$countTick(BooleanSupplier hasTime, CallbackInfo ci) {
        if (Stats.reported) return;
        Stats.counter++;
        Stats.order.append('A');
    }
}
