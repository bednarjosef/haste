package haste.rt;

import java.util.Map;
import java.util.TreeMap;

/**
 * Every proxy method without an implementation lands here: the log is the to-do list. Each entry also records the
 * first caller outside Haste and the proxies (class.method:line), which says where vanilla code needed it.
 */
public final class Unimplemented {
    private static final Map<String, String> SEEN = new TreeMap<>();

    private Unimplemented() {}

    /** Records {@code what} (owner#name+desc) and returns the exception the stub throws. */
    public static RuntimeException fail(String what) {
        String caller = "";
        StackTraceElement[] st = new Throwable().getStackTrace();
        for (int i = 1; i < st.length; i++) {
            String c = st[i].getClassName();
            if (c.startsWith("haste.") || what.startsWith(c.replace('.', '/') + "#")) continue;
            caller = st[i].getClassName() + "." + st[i].getMethodName() + ":" + st[i].getLineNumber();
            break;
        }
        synchronized (SEEN) { SEEN.putIfAbsent(what, caller); }
        return new UnsupportedOperationException("haste proxy: " + what);
    }

    /** Everything recorded so far, one per line ("what  <- caller"), sorted. */
    public static String seen() {
        StringBuilder sb = new StringBuilder();
        synchronized (SEEN) {
            for (var e : SEEN.entrySet()) {
                if (sb.length() > 0) sb.append('\n');
                sb.append(e.getKey());
                if (!e.getValue().isEmpty()) sb.append("  <- ").append(e.getValue());
            }
        }
        return sb.toString();
    }

    /** Forgets the log (tests call this before exercising one screen). */
    public static void clear() { synchronized (SEEN) { SEEN.clear(); } }
}
