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
     *               and repeated lib, allowPackage, allowClass, deny.
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
        boolean verbose = Boolean.parseBoolean(one.getOrDefault("verbose", "false"));

        HasteClassLoader l = new HasteClassLoader(gameDir, proxyDir, clientJar, libs, allowPackages, allowClasses, deny, verbose);
        String realKey = String.join(",", allowPackages) + "|" + String.join(",", allowClasses) + "|" + String.join(",", deny);
        genSummary = Gen.generateIfStale(clientJar, conf, gameDir, proxyDir, n -> l.refusal(n) == null, realKey);
        loader = l;
        Thread.currentThread().setContextClassLoader(l);
        Class<?> bridge = l.loadClass("haste.gui.Bridge");
        return bridge.getMethod("create").invoke(null);
    }

    /** Loads a class through the game loader (tier 3 / proxies / haste.gui), for Rust. */
    public static Class<?> loadClass(String name) throws ClassNotFoundException {
        return Class.forName(name, true, loader);
    }

    /** The class loader's log ({@code "<how>\t<name>"} lines). */
    public static String classLog() { return loader == null ? "" : loader.log(); }

    /** The proxy generator's summary line from this start. */
    public static String genSummary() { return genSummary; }
}
