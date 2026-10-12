package haste.gui;

import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.List;
import java.util.Optional;
import java.util.Set;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.metadata.MetadataSectionType;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.resources.IoSupplier;
import net.minecraft.server.packs.resources.Resource;
import net.minecraft.server.packs.resources.ResourceProvider;

/**
 * The vanilla data pack inside the player's client jar ({@code data/minecraft/...}), read-only. Used to build the
 * synchronised registries without a server (tests, and anything that needs registry data before joining): the
 * entries are announced without data and vanilla's registry loader reads them from here, the way a vanilla client
 * reads entries of packs it already knows.
 */
final class JarDataPack implements PackResources, ResourceProvider {
    private final ZipFile jar;
    private final PackLocationInfo location =
        new PackLocationInfo("vanilla", Component.literal("Vanilla (client jar)"), PackSource.BUILT_IN, Optional.empty());

    JarDataPack(String jarPath) throws IOException { this.jar = new ZipFile(jarPath); }

    /** Entry ids of a registry ({@code dialog} -> every {@code data/minecraft/dialog/**.json}), sorted. */
    List<Identifier> entries(String registryPath) {
        String prefix = "data/minecraft/" + registryPath + "/";
        List<Identifier> out = new ArrayList<>();
        for (Enumeration<? extends ZipEntry> e = jar.entries(); e.hasMoreElements(); ) {
            String n = e.nextElement().getName();
            if (n.startsWith(prefix) && n.endsWith(".json")) {
                out.add(Identifier.withDefaultNamespace(n.substring(prefix.length(), n.length() - 5)));
            }
        }
        out.sort(null);
        return out;
    }

    private static String path(PackType type, Identifier id) {
        return type.getDirectory() + "/" + id.getNamespace() + "/" + id.getPath();
    }

    // ---- ResourceProvider (data files by full identifier, e.g. minecraft:dialog/foo.json) ----
    @Override public Optional<Resource> getResource(Identifier id) {
        IoSupplier<InputStream> s = getResource(PackType.SERVER_DATA, id);
        return s == null ? Optional.empty() : Optional.of(new Resource(this, s));
    }

    // ---- PackResources ----
    @Override public IoSupplier<InputStream> getResource(PackType type, Identifier id) {
        ZipEntry e = jar.getEntry(path(type, id));
        return e == null ? null : () -> jar.getInputStream(e);
    }

    @Override public void listResources(PackType type, String namespace, String directory, ResourceOutput out) {
        String prefix = type.getDirectory() + "/" + namespace + "/" + directory + "/";
        for (Enumeration<? extends ZipEntry> en = jar.entries(); en.hasMoreElements(); ) {
            ZipEntry e = en.nextElement();
            String n = e.getName();
            if (n.startsWith(prefix) && !e.isDirectory()) {
                out.accept(Identifier.fromNamespaceAndPath(namespace, n.substring(type.getDirectory().length() + namespace.length() + 2)),
                    () -> jar.getInputStream(e));
            }
        }
    }

    @Override public Set<String> getNamespaces(PackType type) { return Set.of("minecraft"); }
    @Override public PackLocationInfo location() { return location; }
    @Override public IoSupplier<InputStream> getRootResource(String... path) {
        ZipEntry e = jar.getEntry(String.join("/", path));
        return e == null ? null : () -> jar.getInputStream(e);
    }
    @Override public <T> T getMetadataSection(MetadataSectionType<T> type) { return null; }
    @Override public void close() {
        try { jar.close(); } catch (IOException e) { /* read-only */ }
    }
}
