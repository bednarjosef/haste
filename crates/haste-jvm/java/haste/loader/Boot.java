package haste.loader;

import haste.proxy.Gen;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Entry point Rust calls once after starting the JVM. Generates the tier-2 proxies (if stale), creates the
 * {@link HasteClassLoader} and returns the game-side bridge object ({@code haste.gui.Bridge}).
 */
public final class Boot {
    private static HasteClassLoader loader;
    private static String genSummary = "";

    private Boot() {}

    /**
     * @param config newline-separated {@code key=value} lines: clientJar, gameDir, proxyDir, proxiesConf, verbose,
     *               and repeated lib, allowPackage, allowClass, deny, replacement.
     * @return the {@code haste.gui.Bridge} instance
     */
    public static synchronized Object boot(String config) throws Throwable {
        if (loader != null) throw new IllegalStateException("Boot.boot called twice");
        Map<String, String> one = new LinkedHashMap<>();
        Map<String, List<String>> many = new LinkedHashMap<>();
        for (String line : config.split("\n")) {
            int eq = line.indexOf('=');
            if (eq < 0) continue;
            String k = line.substring(0, eq), v = line.substring(eq + 1);
            one.put(k, v);
            many.computeIfAbsent(k, x -> new ArrayList<>()).add(v);
        }
        Path clientJar = Path.of(one.get("clientJar"));
        Path gameDir = Path.of(one.get("gameDir"));
        Path proxyDir = Path.of(one.get("proxyDir"));
        Path conf = Path.of(one.get("proxiesConf"));
        Path[] libs = many.getOrDefault("lib", List.of()).stream().map(Path::of).toArray(Path[]::new);
        String[] allowPackages = many.getOrDefault("allowPackage", List.of()).toArray(new String[0]);
        String[] allowClasses = many.getOrDefault("allowClass", List.of()).toArray(new String[0]);
        String[] deny = many.getOrDefault("deny", List.of()).toArray(new String[0]);
        String[] replacements = many.getOrDefault("replacement", List.of()).toArray(new String[0]);
        boolean verbose = Boolean.parseBoolean(one.getOrDefault("verbose", "false"));

        // game-side code reads the player's jar for data it cannot get through the class loader (data pack listing)
        System.setProperty("haste.clientJar", clientJar.toString());
        // the launcher's downloaded assets (the index and its objects): the client jar carries only
        // en_us, so the rest of the languages and the pack.mcmeta language list come from here
        System.setProperty("haste.assetsDir", one.getOrDefault("assetsDir", ""));
        System.setProperty("haste.assetsIndex", one.getOrDefault("assetsIndex", ""));
        System.setProperty("haste.minecraftDir", one.getOrDefault("minecraftDir", "."));
        // where singleplayer worlds live (Rust and Java share it; see haste-setup's worlds module)
        System.setProperty("haste.singleplayerDir", one.getOrDefault("singleplayerDir", ""));
        long t0 = System.nanoTime();
        HasteClassLoader l = new HasteClassLoader(gameDir, proxyDir, clientJar, libs, allowPackages, allowClasses, deny, replacements, verbose);
        String realKey = String.join(",", allowPackages) + "|" + String.join(",", allowClasses) + "|" + String.join(",", deny);
        genSummary = Gen.generateIfStale(clientJar, conf, gameDir, proxyDir, n -> l.refusal(n) == null, realKey);
        long t1 = System.nanoTime();
        loader = l;
        Thread.currentThread().setContextClassLoader(l);
        Class<?> bridge = l.loadClass("haste.gui.Bridge");
        Object b = bridge.getMethod("create").invoke(null);
        long t2 = System.nanoTime();
        long bootstrap = bridge.getField("bootstrapMillis").getLong(null);
        genSummary += String.format(" | proxies %.0f ms, Bridge.create %.0f ms (vanilla bootstrap %d ms, canonical proxies)",
            (t1 - t0) / 1e6, (t2 - t1) / 1e6, bootstrap);
        return b;
    }

    /** Loads a class through the game loader (tier 3 / proxies / haste.gui), for Rust. */
    public static Class<?> loadClass(String name) throws ClassNotFoundException {
        return Class.forName(name, true, loader);
    }

    /** The class loader's log ({@code "<how>\t<name>"} lines). */
    public static String classLog() { return loader == null ? "" : loader.log(); }

    /** The proxy generator's summary line from this start. */
    public static String genSummary() { return genSummary; }

    /**
     * One full GC, for the settle diet ({@code JvmHost.request_gc}): with the tight
     * {@code -XX:MaxHeapFreeRatio} the JVM starts with, this uncommits the heap the join
     * spike grew. Called once per session, after the world has gone quiet.
     */
    public static void collectGarbage() { System.gc(); }

    /**
     * JVM memory diagnostics for the bench's memory line, as {@code key=value} lines:
     * heap committed/used/max, metaspace, code cache and the class data sharing archive (bytes).
     */
    public static String memoryStats() {
        Runtime rt = Runtime.getRuntime();
        long total = rt.totalMemory(), free = rt.freeMemory();
        StringBuilder sb = new StringBuilder(256);
        sb.append("heap_committed=").append(total).append('\n');
        sb.append("heap_used=").append(total - free).append('\n');
        sb.append("heap_max=").append(rt.maxMemory()).append('\n');
        try {
            java.lang.management.MemoryPoolMXBean metaspace = null, compressed = null;
            long code = 0;
            for (java.lang.management.MemoryPoolMXBean pool
                     : java.lang.management.ManagementFactory.getMemoryPoolMXBeans()) {
                String n = pool.getName();
                if (n.startsWith("CodeHeap") || n.equals("CodeCache")) code += pool.getUsage().getUsed();
                else if (n.equals("Metaspace")) metaspace = pool;
                else if (n.equals("Compressed Class Space")) compressed = pool;
            }
            if (metaspace != null) sb.append("metaspace_used=").append(metaspace.getUsage().getUsed()).append('\n');
            if (code > 0) sb.append("code_cache_used=").append(code).append('\n');
            if (compressed != null) sb.append("compressed_class_used=").append(compressed.getUsage().getUsed()).append('\n');
        } catch (Throwable t) {
            // non-fatal: the bench prints what it got
        }
        return sb.toString();
    }
}
