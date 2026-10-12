package haste.testmod.mixin;

import haste.testmod.HasteDuck;
import haste.testmod.Helper;
import haste.testmod.Probe;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.item.ItemStack;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Step 3 + 4: object-typed @Shadow, vanilla calls through proxies, a setter into Rust, mod helper code, duck interface. */
@Mixin(value = Minecraft.class, priority = 900)
public abstract class MinecraftObjectMixin {
    @Shadow public LocalPlayer player;

    @Inject(method = "tick", at = @At("HEAD"))
    private void haste$objects(CallbackInfo ci) {
        if (Probe.benchEnabled) return;
        Probe.lastX = this.player.getX();
        ItemStack stack = this.player.getInventory().getItem(0);
        Probe.lastCount = stack.getCount();
        stack.setCount(Probe.lastCount + 1);
        HasteDuck duck = (HasteDuck) this.player;
        Probe.score = duck.haste$getScore() + 10;
        duck.haste$setScore(Probe.score);
        Probe.helper = Helper.describe(Probe.lastX, Probe.lastCount);
    }
}
