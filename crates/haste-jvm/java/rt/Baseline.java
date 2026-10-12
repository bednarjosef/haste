package haste.rt;

/** Step 6 (b): the chatty loop against plain Java objects with the same API shape, timed in Java. */
public final class Baseline {
    public static final class PlainPlayer { double x; PlainInventory inv = new PlainInventory(); public double getX() { return x; } public PlainInventory getInventory() { return inv; } }
    public static final class PlainInventory { PlainStack s0 = new PlainStack(); public PlainStack getItem(int i) { return s0; } }
    public static final class PlainStack { int count; public int getCount() { return count; } }
    public static double sink;

    /** Returns nanoseconds for one "tick" of n iterations (2 calls each), after warm-up. */
    public static long run(int n, int warmupTicks) {
        PlainPlayer p = new PlainPlayer(); p.x = 123.5; p.inv.s0.count = 7;
        for (int w = 0; w < warmupTicks; w++) tick(p, n);
        long t = System.nanoTime();
        tick(p, n);
        return System.nanoTime() - t;
    }
    static void tick(PlainPlayer player, int n) {
        PlainStack s = player.getInventory().getItem(0);
        double sum = 0;
        for (int i = 0; i < n; i++) { sum += player.getX(); sum += s.getCount(); }
        sink = sum;
    }
}
