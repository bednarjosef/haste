package haste.testmod;

/** Plain static scratch space for the server-side twin of the test; lets both mixins report together. */
public final class Stats {
    public static int ticks, counter, cancels;
    public static final StringBuilder order = new StringBuilder();
    public static boolean reported;

    public static void report() {
        reported = true;
        HasteTestMod.LOGGER.info("HASTE-VERIFY ticks={} counter={} cancels={} first={} order={}",
            ticks, counter, cancels, order.charAt(0), order.substring(0, Math.min(order.length(), 24)));
    }
}
