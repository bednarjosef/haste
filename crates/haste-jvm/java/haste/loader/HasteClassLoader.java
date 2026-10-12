package haste.loader;

import java.io.IOException;
import java.io.InputStream;
import java.net.MalformedURLException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/**
 * Haste's game class loader (tiers 2 and 3). For a class name it serves, in this order:
 * <ol>
 *   <li>JDK classes and Haste's boot classes ({@code haste.loader/proxy/rt}) from the parent;</li>
 *   <li>Haste's game-side classes ({@code haste.gui.*}, compiled against the client jar) from {@code gameDir};</li>
 *   <li>a generated tier-2 proxy from {@code proxyDir}, if one exists for the name (proxies win over everything);</li>
 *   <li>vanilla classes ({@code net.minecraft.*}, {@code com.mojang.*} that are in the client jar): from the
 *       player's client jar, but only when the allow-list permits and the deny-list does not;</li>
 *   <li>everything else (fastutil, guava, DFU's {@code com.mojang.serialization}, ...) from vanilla's
 *       third-party library jars.</li>
 * </ol>
 * Every decision is logged in {@link #log()}: "jar", "proxy", "game", "lib" or "refused: reason". The refused
 * entries are the to-do list (a missing proxy or allow-list entry).
 *
 * <p>The whole client jar is NEVER on the class path: it is only reachable through {@link #findClass}, one
 * allow-listed class at a time.
 */
public final class HasteClassLoader extends ClassLoader {
    static { registerAsParallelCapable(); }

    private final Path gameDir;
    private final Path proxyDir;
    private final ZipFile clientJar;
    private final URLClassLoader libs;
    private final String[] allowPackages, allowClasses, deny;
    /** class name -> how it was served or why it was refused (sorted, for stable output). */
    private final Map<String, String> log = new TreeMap<>();
    private final boolean verbose;

    public HasteClassLoader(Path gameDir, Path proxyDir, Path clientJar, Path[] libraryJars,
                            String[] allowPackages, String[] allowClasses, String[] deny, boolean verbose) throws IOException {
        super("haste-game", HasteClassLoader.class.getClassLoader());
        this.gameDir = gameDir;
        this.proxyDir = proxyDir;
        this.clientJar = new ZipFile(clientJar.toFile());
        List<URL> urls = new ArrayList<>();
        for (Path p : libraryJars) {
            try { urls.add(p.toUri().toURL()); } catch (MalformedURLException e) { throw new IllegalArgumentException(e); }
        }
        // Libraries see the JDK (platform loader) only: they never need vanilla or haste classes.
        this.libs = new URLClassLoader("haste-libs", urls.toArray(new URL[0]), ClassLoader.getPlatformClassLoader());
        this.allowPackages = allowPackages;
        this.allowClasses = allowClasses;
        this.deny = deny;
        this.verbose = verbose;
    }

    private static boolean parentFirst(String name) {
        return name.startsWith("java.") || name.startsWith("javax.") || name.startsWith("jdk.") || name.startsWith("sun.")
            || name.startsWith("haste.loader.") || name.startsWith("haste.proxy.") || name.startsWith("haste.rt.")
            || name.startsWith("org.objectweb.asm.");
    }

    @Override
    protected Class<?> loadClass(String name, boolean resolve) throws ClassNotFoundException {
        if (parentFirst(name)) return super.loadClass(name, resolve);
        synchronized (getClassLoadingLock(name)) {
            Class<?> c = findLoadedClass(name);
            if (c == null) c = findClass(name);
            if (resolve) resolveClass(c);
            return c;
        }
    }

    @Override
    protected Class<?> findClass(String name) throws ClassNotFoundException {
        String internal = name.replace('.', '/');
        if (name.startsWith("haste.")) {
            Class<?> c = defineFromDir(gameDir, name, internal);
            if (c == null) throw refuse(name, "no such haste class");
            record(name, "game");
            return c;
        }
        Class<?> proxy = defineFromDir(proxyDir, name, internal);
        if (proxy != null) { record(name, "proxy"); return proxy; }
        boolean vanilla = name.startsWith("net.minecraft.") || name.startsWith("com.mojang.");
        ZipEntry e = vanilla ? clientJar.getEntry(internal + ".class") : null;
        if (e != null) {
            String why = refusal(internal);
            if (why != null) throw refuse(name, why);
            byte[] b;
            try (InputStream in = clientJar.getInputStream(e)) { b = in.readAllBytes(); }
            catch (IOException ex) { throw new ClassNotFoundException(name, ex); }
            Class<?> c = defineClass(name, b, 0, b.length);
            record(name, "jar");
            return c;
        }
        if (name.startsWith("net.minecraft.")) throw refuse(name, "not in the client jar");
        try {
            Class<?> c = libs.loadClass(name);
            record(name, "lib");
            return c;
        } catch (ClassNotFoundException ex) {
            throw refuse(name, "not in the client jar, proxies or libraries");
        }
    }

    /**
     * Null if the policy permits {@code internal} from the jar, else the reason. The most specific matching
     * pattern wins (an exact class beats any package; a longer package prefix beats a shorter one); on a tie
     * deny wins. So {@code world/level/} can be denied while {@code world/level/storage/DataVersion} is allowed.
     */
    public String refusal(String internal) {
        int allowLen = Math.max(longest(internal, allowClasses), longest(internal, allowPackages));
        int denyLen = longest(internal, deny);
        if (denyLen >= 0 && denyLen >= allowLen) return "deny-listed (tier 1: needs a proxy)";
        if (allowLen >= 0) return null;
        return "not in the tier-3 allow-list";
    }

    /**
     * Specificity of the best matching pattern: a class pattern beats every package pattern, and a longer pattern
     * beats a shorter one (so {@code Outer$Nested} beats {@code Outer}); none = -1.
     */
    private static int longest(String internal, String[] patterns) {
        int best = -1;
        for (String p : patterns) {
            if (!matches(internal, p)) continue;
            best = Math.max(best, p.endsWith("/") ? p.length() : 100_000 + p.length());
        }
        return best;
    }

    /** A pattern ending in '/' is a package prefix (with subpackages), otherwise an exact class (with nested classes). */
    private static boolean matches(String internal, String pattern) {
        if (pattern.endsWith("/")) return internal.startsWith(pattern);
        return internal.equals(pattern) || internal.startsWith(pattern + "$");
    }

    private Class<?> defineFromDir(Path dir, String name, String internal) throws ClassNotFoundException {
        if (dir == null) return null;
        Path f = dir.resolve(internal + ".class");
        if (!Files.isRegularFile(f)) return null;
        try {
            byte[] b = Files.readAllBytes(f);
            return defineClass(name, b, 0, b.length);
        } catch (IOException ex) {
            throw new ClassNotFoundException(name, ex);
        }
    }

    private void record(String name, String how) {
        synchronized (log) { log.put(name, how); }
    }

    private ClassNotFoundException refuse(String name, String why) {
        String prev;
        synchronized (log) { prev = log.put(name, "refused: " + why); }
        if (prev == null && verbose) System.err.println("[haste] class refused: " + name + " (" + why + ")");
        return new ClassNotFoundException(name + " (haste: " + why + ")");
    }

    /** Libraries first, then non-class resources of the client jar (e.g. {@code assets/minecraft/lang/en_us.json}). */
    @Override
    protected URL findResource(String name) {
        URL u = libs.getResource(name);
        if (u != null || name.endsWith(".class")) return u;
        String n = name.startsWith("/") ? name.substring(1) : name;
        if (clientJar.getEntry(n) == null) return null;
        try { return new URL("jar:" + Path.of(clientJar.getName()).toUri() + "!/" + n); }
        catch (MalformedURLException e) { return null; }
    }

    @Override
    protected Enumeration<URL> findResources(String name) throws IOException { return libs.getResources(name); }

    /** The class log as lines {@code "<how>\t<class name>"}, sorted by class name. */
    public String log() {
        StringBuilder sb = new StringBuilder();
        synchronized (log) { for (var en : log.entrySet()) sb.append(en.getValue()).append('\t').append(en.getKey()).append('\n'); }
        return sb.toString();
    }
}
