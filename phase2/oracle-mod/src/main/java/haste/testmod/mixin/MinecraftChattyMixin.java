package haste.testmod.mixin;

import haste.testmod.Probe;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.item.ItemStack;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Step 6: a chatty handler, 10,000 x (getX + getCount) per tick. */
@Mixin(value = Minecraft.class, priority = 950)
public abstract class MinecraftChattyMixin {
    @Shadow public LocalPlayer player;

    @Inject(method = "tick", at = @At("HEAD"))
    private void haste$chatty(CallbackInfo ci) {
        if (!Probe.benchEnabled) return;
        LocalPlayer p = this.player;
        ItemStack s = p.getInventory().getItem(0);
        double sum = 0;
        for (int i = 0; i < 10_000; i++) { sum += p.getX(); sum += s.getCount(); }
        Probe.sink = sum;
    }
}
