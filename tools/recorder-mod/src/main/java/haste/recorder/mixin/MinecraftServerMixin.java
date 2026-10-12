package haste.recorder.mixin;

import haste.recorder.ServerDumps;
import java.util.function.BooleanSupplier;
import net.minecraft.server.MinecraftServer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Runs the oracle dumps once, on the first tick after the level is loaded. */
@Mixin(MinecraftServer.class)
public abstract class MinecraftServerMixin {
    private static boolean haste$dumped = false;

    @Inject(method = "tickServer", at = @At("TAIL"))
    private void haste$dumpOracles(BooleanSupplier hasTime, CallbackInfo ci) {
        if (haste$dumped) return;
        haste$dumped = true;
        MinecraftServer server = (MinecraftServer) (Object) this;
        ServerDumps.dumpAll(server);
        // The dump is the only reason this server exists; stop it so scripts terminate.
        server.halt(false);
    }
}
