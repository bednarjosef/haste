package haste.gui;

import net.minecraft.client.DeltaTracker;

/**
 * The frame's timing as vanilla's HUD asks for it: the partial tick from Rust's render loop and the time since the
 * previous frame in ticks (measured here, 50 ms per tick). The real-time delta is vanilla's
 * {@code DeltaTracker.Timer.advanceRealTime}: whole milliseconds since the previous frame over 50, read back as
 * 0.5 when over 7 ticks ({@code getRealtimeDeltaTicks}); the title panorama spins by it.
 */
final class Delta implements DeltaTracker {
    static final Delta INSTANCE = new Delta();
    static float partial;
    private static float delta;
    private static long last;
    private static float realtime;
    private static long lastUiMs;

    private Delta() {}

    /** Starts a frame. */
    static void frame(float partialTick) {
        partial = partialTick;
        long now = System.nanoTime();
        delta = last == 0 ? 0f : Math.min(10f, (now - last) / 50_000_000f);
        last = now;
        long ms = now / 1_000_000L;
        realtime = lastUiMs == 0 ? 0f : (ms - lastUiMs) / 50f;
        lastUiMs = ms;
    }

    @Override public float getGameTimeDeltaTicks() { return delta; }
    @Override public float getGameTimeDeltaPartialTick(boolean ignoreFreeze) { return partial; }
    @Override public float getRealtimeDeltaTicks() { return realtime > 7.0F ? 0.5F : realtime; }
}
