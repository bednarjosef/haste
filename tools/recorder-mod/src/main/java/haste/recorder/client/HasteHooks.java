package haste.recorder.client;

/** Shared flags the mixins read; set by the recorder, cleared when the capture is done. */
public final class HasteHooks {
    private HasteHooks() {}

    /** True while a screenshot capture is in flight: suppress the HUD for those frames. */
    public static volatile boolean hideGui = false;

    /** True while a bench run is recording: frame hook samples every frame. */
    public static volatile boolean benchRunning = false;
}
