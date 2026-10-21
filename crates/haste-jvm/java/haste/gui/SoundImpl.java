package haste.gui;

import com.mojang.blaze3d.audio.ListenerTransform;
import haste.rt.Native;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.client.resources.sounds.SimpleSoundInstance;
import net.minecraft.client.resources.sounds.SoundInstance;
import net.minecraft.client.sounds.SoundEventListener;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.client.sounds.WeighedSoundEvents;
import net.minecraft.resources.Identifier;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.RandomSource;
import net.minecraft.world.phys.Vec3;

/**
 * The sound event listeners of the {@code SoundManager} proxy (vanilla {@code SoundEngine.listeners}): the HUD's real
 * {@code SubtitleOverlay} registers itself while "Show Subtitles" is on, reads the listener transform every frame and
 * is told about every sound played. The sound is Rust's (haste-audio): Rust records the sounds with a subtitle it
 * played while a listener is registered, and {@link #frame} hands them to the listeners once per frame, before the
 * HUD draws, as vanilla's {@code SoundEngine.notifyListeners} does at each play.
 */
public final class SoundImpl {
    private SoundImpl() {}

    /** Doubles per played sound (Rust's {@code sound_manager::VALUES_PER_SOUND}). */
    private static final int VALUES = 9;
    private static final int X = 0, Y = 1, Z = 2, RANGE = 3, VOLUME = 4, PITCH = 5, SOURCE = 6, LOOPING = 7, RELATIVE = 8;

    private static final List<SoundEventListener> LISTENERS = new ArrayList<>();
    /** The listener transform Rust filled this frame (position, forward, up). */
    private static final double[] LISTENER = new double[9];
    /** What {@link #transform} was built from, so an unmoved listener builds nothing. */
    private static final double[] BUILT = new double[9];
    private static ListenerTransform transform = ListenerTransform.INITIAL;
    /** The sound events played so far by name (vanilla's {@code SoundManager} registry entries). */
    private static final Map<String, WeighedSoundEvents> EVENTS = new HashMap<>();
    /** The instances' random (vanilla gives each its own; nothing samples it here). */
    private static final RandomSource RANDOM = RandomSource.create();
    private static final SoundSource[] SOURCES = SoundSource.values();

    public static void addListener(SoundManager m, SoundEventListener listener) {
        LISTENERS.add(listener);
        Native.soundListening(true);
    }

    public static void removeListener(SoundManager m, SoundEventListener listener) {
        LISTENERS.remove(listener);
        Native.soundListening(!LISTENERS.isEmpty());
    }

    /** Vanilla {@code SoundManager.getListenerTransform}: where the sound engine hears from (the camera, per tick). */
    public static ListenerTransform getListenerTransform(SoundManager m) {
        return transform;
    }

    /** Once per frame (Bridge.render): the listener transform, then the sounds played since the last frame. */
    static void frame() {
        if (LISTENERS.isEmpty()) return;
        Object[] played = Native.takePlayedSounds(LISTENER);
        if (!Arrays.equals(LISTENER, BUILT)) {
            System.arraycopy(LISTENER, 0, BUILT, 0, LISTENER.length);
            transform = new ListenerTransform(new Vec3(LISTENER[0], LISTENER[1], LISTENER[2]),
                new Vec3(LISTENER[3], LISTENER[4], LISTENER[5]), new Vec3(LISTENER[6], LISTENER[7], LISTENER[8]));
        }
        if (played == null) return;
        String[] names = (String[]) played[0];
        double[] values = (double[]) played[1];
        for (int i = 0; i * 2 < names.length; i++) {
            Identifier location = Identifier.tryParse(names[i * 2]);
            if (location == null) continue;
            int o = i * VALUES;
            boolean relative = values[o + RELATIVE] != 0;
            SoundInstance instance = new SimpleSoundInstance(location, SOURCES[(int) values[o + SOURCE]],
                (float) values[o + VOLUME], (float) values[o + PITCH], RANDOM, values[o + LOOPING] != 0, 0,
                relative ? SoundInstance.Attenuation.NONE : SoundInstance.Attenuation.LINEAR,
                values[o + X], values[o + Y], values[o + Z], relative);
            String subtitle = names[i * 2 + 1];
            WeighedSoundEvents event = EVENTS.computeIfAbsent(names[i * 2], n -> new WeighedSoundEvents(location, subtitle));
            float range = (float) values[o + RANGE];
            for (SoundEventListener listener : LISTENERS) listener.onPlaySound(instance, event, range);
        }
    }
}
