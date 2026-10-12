package haste.rt;

import java.util.Set;
import java.util.TreeSet;

/** Every proxy method without an implementation lands here: the log is the to-do list. */
public final class Unimplemented {
    private static final Set<String> SEEN = new TreeSet<>();

    private Unimplemented() {}

    /** Records {@code what} (owner#name+desc) and returns the exception the stub throws. */
    public static RuntimeException fail(String what) {
        synchronized (SEEN) { SEEN.add(what); }
        return new UnsupportedOperationException("haste proxy: " + what);
    }

    /** Everything recorded so far, one per line, sorted. */
    public static String seen() { synchronized (SEEN) { return String.join("\n", SEEN); } }

    /** Forgets the log (tests call this before exercising one screen). */
    public static void clear() { synchronized (SEEN) { SEEN.clear(); } }
}
