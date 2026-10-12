package haste.testmod.mixin;

import haste.testmod.Stats;
import java.util.function.BooleanSupplier;
import net.minecraft.server.MinecraftServer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Server-side twin of MinecraftMixinB: cancels every 5th tick for the first 100 ticks, then reports. */
@Mixin(value = MinecraftServer.class, priority = 1500)
public abstract class MinecraftServerMixinB {
    @Inject(method = "tickServer", at = @At("HEAD"), cancellable = true)
    private void haste$maybeCancel(BooleanSupplier hasTime, CallbackInfo ci) {
        if (Stats.reported) return;
        if (Stats.ticks >= 100) { Stats.report(); return; }
        Stats.ticks++;
        Stats.order.append('B');
        if (Stats.ticks % 5 == 0) {
            Stats.cancels++;
            ci.cancel();
        }
    }
}
