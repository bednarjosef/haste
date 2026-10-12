package haste.testmod.mixin;

import haste.testmod.HasteDuck;
import net.minecraft.client.player.LocalPlayer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** Step 4: adds a field and an interface to a vanilla class. */
@Mixin(LocalPlayer.class)
public abstract class LocalPlayerMixin implements HasteDuck {
    @Unique private int haste$score;

    @Override public int haste$getScore() { return haste$score; }
    @Override public void haste$setScore(int v) { haste$score = v; }
}
