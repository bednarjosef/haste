package haste.bridge;

/** Game state that lives in Rust. Natives are registered by the Rust host via RegisterNatives. */
public final class RustState {
    private RustState() {}
    public static native int getInt(int id);
    public static native void setInt(int id, int value);
    public static native Object getObject(int id);
    public static native void setObject(int id, Object value);
}
