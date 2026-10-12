package haste.rt;

import java.util.Set;
import java.util.TreeSet;

/** Every proxy method without an implementation lands here: the log is the to-do list. */
public final class Unimplemented {
    private static final Set<String> SEEN = new TreeSet<>();
    public static RuntimeException fail(String what) {
        synchronized (SEEN) { if (SEEN.add(what)) System.err.println("[haste] UNIMPLEMENTED proxy method: " + what); }
        return new UnsupportedOperationException("haste proxy: " + what);
    }
    public static String seen() { synchronized (SEEN) { return String.join("\n", SEEN); } }
}
