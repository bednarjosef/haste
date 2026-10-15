package haste.gui;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.IOException;
import java.io.InputStream;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Enumeration;
import java.util.HashMap;
import java.util.Map;
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

/**
 * The player's client resources as one pack, read from the two places vanilla's built-in pack reads
 * from ({@code VanillaPackResourcesBuilder.pushJarResources} + {@code pushAssetPath}): the client
 * jar's own {@code assets/} tree and the launcher's downloaded assets (the index's hash-named
 * objects under {@code assets/objects}). Haste loads no resource packs of its own, so this is the
 * pack the {@code LanguageManager}'s reload reads (the {@code pack.mcmeta} language list and
 * {@code assets/minecraft/lang/*.json}); the client jar alone carries only en_us.
 */
final class AssetsPack implements PackResources {
    private final ZipFile jar;
    private final Path objectsDir;
    /** Asset index keys ({@code <namespace>/<path>}, plus {@code pack.mcmeta}) to their object hash. */
    private final Map<String, String> index;
    private final PackLocationInfo location = new PackLocationInfo(
        "vanilla", Component.literal("Vanilla (client resources)"), PackSource.BUILT_IN, Optional.empty());

    AssetsPack(String jarPath, String assetsDir, String indexName) throws IOException {
        this.jar = new ZipFile(jarPath);
        Path assets = Path.of(assetsDir);
        this.objectsDir = assets.resolve("objects");
        this.index = loadIndex(assets, indexName);
    }

    /** The launcher's asset index ({@code indexes/<name>.json}, or the single index present). */
    private static Map<String, String> loadIndex(Path assets, String indexName) {
        Path indexPath = assets.resolve("indexes").resolve(indexName + ".json");
        if (!Files.isRegularFile(indexPath)) {
            try (var entries = Files.list(assets.resolve("indexes"))) {
                var found = entries
                    .filter(p -> p.getFileName().toString().endsWith(".json"))
                    .toList();
                if (found.size() == 1) indexPath = found.get(0);
            } catch (IOException e) {
                // no index: the jar's own assets tree is all there is
            }
        }
        Map<String, String> map = new HashMap<>();
        try (Reader reader = Files.newBufferedReader(indexPath, StandardCharsets.UTF_8)) {
            JsonObject root = JsonParser.parseReader(reader).getAsJsonObject();
            root.getAsJsonObject("objects").entrySet().forEach(
                e -> map.put(e.getKey(), e.getValue().getAsJsonObject().get("hash").getAsString()));
        } catch (IOException e) {
            // like Haste's assets reader: soft-fail so the jar alone can serve the pack
        }
        return map;
    }

    /** The downloaded object of an index key, or null. */
    private IoSupplier<InputStream> object(String key) {
        String hash = index.get(key);
        if (hash == null || hash.length() < 2) return null;
        Path path = objectsDir.resolve(hash.substring(0, 2)).resolve(hash);
        return Files.isRegularFile(path) ? () -> Files.newInputStream(path) : null;
    }

    /** The jar's {@code assets/<key>} entry, or null. */
    private IoSupplier<InputStream> jarAsset(String key) {
        ZipEntry e = jar.getEntry("assets/" + key);
        return e == null ? null : () -> jar.getInputStream(e);
    }

    // ---- PackResources ----
    @Override public IoSupplier<InputStream> getResource(PackType type, Identifier id) {
        if (type != PackType.CLIENT_RESOURCES) return null;
        String key = id.getNamespace() + "/" + id.getPath();
        IoSupplier<InputStream> fromJar = jarAsset(key);
        return fromJar != null ? fromJar : object(key);
    }

    @Override public void listResources(PackType type, String namespace, String directory, ResourceOutput out) {
        if (type != PackType.CLIENT_RESOURCES) return;
        String prefix = namespace + "/" + directory + "/";
        for (Map.Entry<String, String> e : index.entrySet()) {
            if (!e.getKey().startsWith(prefix)) continue;
            IoSupplier<InputStream> s = object(e.getKey());
            if (s != null) {
                out.accept(Identifier.fromNamespaceAndPath(namespace, e.getKey().substring(namespace.length() + 1)), s);
            }
        }
        String jarPrefix = "assets/" + prefix;
        for (Enumeration<? extends ZipEntry> en = jar.entries(); en.hasMoreElements(); ) {
            ZipEntry e = en.nextElement();
            String n = e.getName();
            if (n.startsWith(jarPrefix) && !e.isDirectory()) {
                out.accept(
                    Identifier.fromNamespaceAndPath(namespace, n.substring("assets/".length() + namespace.length() + 1)),
                    () -> jar.getInputStream(e));
            }
        }
    }

    @Override public Set<String> getNamespaces(PackType type) { return Set.of("minecraft"); }
    @Override public PackLocationInfo location() { return location; }

    @Override public IoSupplier<InputStream> getRootResource(String... path) {
        String key = String.join("/", path);
        IoSupplier<InputStream> fromObjects = object(key);
        if (fromObjects != null) return fromObjects;
        ZipEntry e = jar.getEntry(key);
        return e == null ? null : () -> jar.getInputStream(e);
    }

    /**
     * The metadata sections of {@code pack.mcmeta} (the launcher downloads it as an asset object;
     * the language list the LanguageManager reads is one), loaded once like
     * {@code AbstractPackMetadataResources.loadMetadata}.
     */
    private net.minecraft.server.packs.resources.ResourceMetadata metadata;
    @Override public <T> T getMetadataSection(MetadataSectionType<T> type) throws IOException {
        if (metadata == null) {
            IoSupplier<InputStream> meta = getRootResource(PACK_META);
            metadata = meta == null
                ? net.minecraft.server.packs.resources.ResourceMetadata.EMPTY
                : net.minecraft.server.packs.resources.ResourceMetadata.fromJsonStream(meta.get());
        }
        return metadata.getSection(type).orElse(null);
    }

    @Override public void close() {
        try { jar.close(); } catch (IOException e) { /* read-only */ }
    }
}
