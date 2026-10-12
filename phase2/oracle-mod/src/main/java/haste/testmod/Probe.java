package haste.testmod;

/** Static scratch space the Rust test reads back through JNI. */
public final class Probe {
    public static double lastX;
    public static int lastCount, score;
    public static String helper;
    public static boolean benchEnabled;
    public static double sink;
}
