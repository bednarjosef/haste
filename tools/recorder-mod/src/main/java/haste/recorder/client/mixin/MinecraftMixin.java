package haste.recorder.client.mixin;

import haste.recorder.client.FrameTimes;
import haste.recorder.client.HasteHooks;
import haste.recorder.client.RecorderIO;
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Frame hook for the bench: runTick is once per rendered frame, so HEAD-to-HEAD distance is
 * the frame time. RSS comes from /proc/self/status (VmRSS) as the testkit format expects.
 * Section counts are filled in from the level renderer stats when available (TODO after the
 * first real run; the format keeps them optional).
 */
@Mixin(Minecraft.class)
public abstract class MinecraftMixin {
    private static long haste$lastFrameNanos = 0;

    @Inject(method = "runTick", at = @At("HEAD"))
    private void haste$benchFrame(boolean renderLevel, CallbackInfo ci) {
        long now = System.nanoTime();
        FrameTimes.frame(now);
        if (haste$lastFrameNanos != 0 && HasteHooks.benchRunning) {
            double frameMs = (now - haste$lastFrameNanos) / 1e6;
            // Section/draw-call counts are not reachable from here yet; the testkit treats
            // them as optional columns (0 = not measured this run).
            RecorderIO.benchFrame(frameMs, readRssKib(), 0, 0, 0);
        }
        haste$lastFrameNanos = now;
    }

    /** VmRSS from /proc/self/status, in KiB; -1 when unavailable (non-Linux). */
    private static long readRssKib() {
        try {
            for (String line : java.nio.file.Files.readAllLines(java.nio.file.Path.of("/proc/self/status"))) {
                if (line.startsWith("VmRSS:")) {
                    // "VmRSS:\t  123456 kB"
                    return Long.parseLong(line.replaceAll("[^0-9]", ""));
                }
            }
        } catch (Exception ignored) {
        }
        return -1;
    }
}
