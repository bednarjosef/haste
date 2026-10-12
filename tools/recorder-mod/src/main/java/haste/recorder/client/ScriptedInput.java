package haste.recorder.client;

import net.minecraft.client.player.ClientInput;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.phys.Vec2;

/**
 * A {@link ClientInput} the recorder drives programmatically: {@link #tick()} (which vanilla
 * calls from LocalPlayer.tick) keeps the scripted keys instead of reading the keyboard, so the
 * user can not disturb a recording by holding keys. moveVector mirrors KeyboardInput's
 * normalization of the held keys (forward/back, left/right; diagonal normalized).
 */
public final class ScriptedInput extends ClientInput {
    private volatile Input scripted = Input.EMPTY;

    /** Sets the keys held from the next tick on. */
    public void set(Input keys) {
        this.scripted = keys;
    }

    @Override
    public void tick() {
        // KeyboardInput.tick() rebuilds keyPresses from Options' KeyMappings and derives
        // moveVector; we do the same derivation from the scripted keys (observed from
        // KeyboardInput.tick's bytecode: Vec2(leftImpulse, forwardImpulse).normalized()).
        keyPresses = scripted;
        float forward = impulse(keyPresses.forward(), keyPresses.backward());
        float strafe = impulse(keyPresses.left(), keyPresses.right());
        moveVector = new Vec2(strafe, forward).normalized();
    }

    private static float impulse(boolean positive, boolean negative) {
        if (positive == negative) return 0.0f;
        return positive ? 1.0f : -1.0f;
    }
}
