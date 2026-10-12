package haste.gui;

import io.netty.buffer.Unpooled;
import net.minecraft.client.multiplayer.ClientRegistryLayer;
import net.minecraft.client.multiplayer.RegistryDataCollector;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.common.ClientboundUpdateTagsPacket;
import net.minecraft.network.protocol.configuration.ClientboundRegistryDataPacket;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.packs.resources.ResourceProvider;

/**
 * The registries tier-3 code sees. Built-in registries are vanilla's own, filled by vanilla's bootstrap from the
 * player's jar ({@link Bridge#create}); synchronised registries and tags arrive from the server during the
 * configuration phase and are built by vanilla's {@link RegistryDataCollector} from the packets Rust re-encodes
 * (the exact wire bytes), as the vanilla client does. Until then {@link #access} is the static layer alone.
 */
public final class Registries {
    private Registries() {}

    private static RegistryAccess.Frozen access = ClientRegistryLayer.createRegistryAccess().compositeAccess();
    private static RegistryDataCollector collector;
    private static boolean synced;

    /** The registry access of the current connection (static layer until the server's registries arrived). */
    public static RegistryAccess.Frozen access() { return access; }

    /** Whether the server's registries have been applied. */
    public static boolean synced() { return synced; }

    private static RegistryDataCollector collector() {
        if (collector == null) collector = new RegistryDataCollector();
        return collector;
    }

    /** A {@code registry_data} configuration packet body (without id). */
    static void registryData(byte[] body) {
        FriendlyByteBuf buf = new FriendlyByteBuf(Unpooled.wrappedBuffer(body));
        ClientboundRegistryDataPacket p = ClientboundRegistryDataPacket.STREAM_CODEC.decode(buf);
        collector().appendContents(p.registry(), p.entries());
    }

    /**
     * An {@code update_tags} packet body (configuration or play). In configuration the tags are collected with the
     * registries; in play they are applied to the current registries at once.
     */
    static void tags(byte[] body, boolean play) {
        ClientboundUpdateTagsPacket p = ClientboundUpdateTagsPacket.STREAM_CODEC.decode(Unpooled.wrappedBuffer(body));
        collector().appendTags(p.tags());
        if (play) finish();
    }

    /**
     * End of configuration: builds the registry access from everything collected (vanilla's collector: synced
     * entries decoded with their codecs, tags bound, item components initialised).
     */
    static void finish() {
        if (collector == null) return;
        // ResourceProvider.EMPTY: Haste announces no known packs, so every entry arrives with its data
        access = collector.collectGameRegistries(ResourceProvider.EMPTY, access, false);
        collector = null;
        synced = true;
    }

    /**
     * Builds the synchronised registries from the vanilla data pack in the player's client jar instead of a server:
     * every entry is announced without data and vanilla's loader reads it from the jar (like entries of a pack the
     * client knows), and the tags of every registry are resolved from the jar's tag files by vanilla's
     * {@link net.minecraft.tags.TagLoader} into the same network form a server sends. Entry ids are the sorted
     * order of the files (a server may number them differently). For tests and offline use; a connection's
     * registries replace these.
     */
    static void loadFromJar() throws java.io.IOException {
        JarDataPack pack = new JarDataPack(System.getProperty("haste.clientJar"));
        var rm = new net.minecraft.server.packs.resources.MultiPackResourceManager(
            net.minecraft.server.packs.PackType.SERVER_DATA, java.util.List.of(pack));
        RegistryDataCollector c = new RegistryDataCollector();
        java.util.Map<ResourceKey<? extends Registry<?>>, net.minecraft.tags.TagNetworkSerialization.NetworkPayload> tags = new java.util.HashMap<>();
        for (net.minecraft.resources.RegistryDataLoader.RegistryData<?> d : net.minecraft.resources.RegistryDataLoader.SYNCHRONIZED_REGISTRIES) {
            java.util.List<net.minecraft.core.RegistrySynchronization.PackedRegistryEntry> entries = new java.util.ArrayList<>();
            java.util.Map<net.minecraft.resources.Identifier, Integer> index = new java.util.HashMap<>();
            for (net.minecraft.resources.Identifier id : pack.entries(net.minecraft.core.registries.Registries.elementsDirPath(d.key()))) {
                index.put(id, entries.size());
                entries.add(new net.minecraft.core.RegistrySynchronization.PackedRegistryEntry(id, java.util.Optional.empty()));
            }
            c.appendContents(d.key(), entries);
            putTags(tags, rm, d.key(), id -> java.util.Optional.ofNullable(index.get(id)));
        }
        net.minecraft.core.registries.BuiltInRegistries.REGISTRY.forEach(r ->
            putTags(tags, rm, r.key(), id -> r.getOptional(id).map(v -> ((Registry) r).getId(v))));
        c.appendTags(tags);
        access = c.collectGameRegistries(pack, ClientRegistryLayer.createRegistryAccess().compositeAccess(), false);
        synced = true;
    }

    /** Resolves a registry's tag files (nested tags included) to entry ids with vanilla's tag loader. */
    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void putTags(java.util.Map<ResourceKey<? extends Registry<?>>, net.minecraft.tags.TagNetworkSerialization.NetworkPayload> out,
                                net.minecraft.server.packs.resources.ResourceManager rm, ResourceKey<? extends Registry<?>> key,
                                java.util.function.Function<net.minecraft.resources.Identifier, java.util.Optional<Integer>> ids) {
        var loader = new net.minecraft.tags.TagLoader<Integer>((id, required) -> ids.apply(id),
            net.minecraft.core.registries.Registries.tagsDirPath(key));
        java.util.Map<net.minecraft.resources.Identifier, java.util.List<Integer>> built = loader.build(loader.load(rm));
        if (built.isEmpty()) return;
        java.util.Map<net.minecraft.resources.Identifier, it.unimi.dsi.fastutil.ints.IntList> m = new java.util.HashMap<>();
        built.forEach((tag, list) -> m.put(tag, new it.unimi.dsi.fastutil.ints.IntArrayList(list)));
        out.put(key, new net.minecraft.tags.TagNetworkSerialization.NetworkPayload(m));
    }

    /** A buffer for decoding/encoding registry-dependent data (item stacks, packets) with the current access. */
    static RegistryFriendlyByteBuf buffer(byte[] body) {
        return new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(body), access);
    }

    static RegistryFriendlyByteBuf buffer() {
        return new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
    }

    /** Registry sizes as "name=size" lines (tests compare them with Rust's registries from the data generator). */
    public static String sizes() {
        StringBuilder sb = new StringBuilder();
        access.registries().sorted(java.util.Comparator.comparing(e -> e.key().identifier().toString())).forEach(e ->
            sb.append(e.key().identifier()).append('=').append(e.value().size()).append('\n'));
        return sb.toString();
    }

    /** Name of entry {@code id} in registry {@code registry} (tests), or null. */
    public static String name(String registry, int id) {
        ResourceKey<? extends Registry<?>> key = ResourceKey.createRegistryKey(net.minecraft.resources.Identifier.parse(registry));
        return access.lookup(key).map(r -> {
            Object v = r.byId(id);
            return v == null ? null : String.valueOf(((Registry) r).getKey(v));
        }).orElse(null);
    }

    /** Number of tags bound in a registry (tests). */
    public static int tagCount(String registry) {
        ResourceKey<? extends Registry<?>> key = ResourceKey.createRegistryKey(net.minecraft.resources.Identifier.parse(registry));
        return access.lookup(key).map(r -> (int) r.getTags().count()).orElse(-1);
    }
}
