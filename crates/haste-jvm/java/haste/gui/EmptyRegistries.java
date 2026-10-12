package haste.gui;

import com.mojang.serialization.Lifecycle;
import java.util.HashMap;
import java.util.Map;
import java.util.Optional;
import java.util.stream.Stream;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.resources.ResourceKey;

/**
 * The registry access Java sees until Rust's registry data (tier 1) is bridged: every registry exists and is
 * empty (a frozen vanilla {@link MappedRegistry}), so lookups and tag queries come back empty instead of
 * throwing. Approximate by design; see NOTES.md.
 */
final class EmptyRegistries implements RegistryAccess.Frozen {
    static final EmptyRegistries INSTANCE = new EmptyRegistries();
    private final Map<ResourceKey<?>, Registry<?>> registries = new HashMap<>();

    @Override
    @SuppressWarnings({"unchecked", "rawtypes"})
    public <E> Optional<Registry<E>> lookup(ResourceKey<? extends Registry<? extends E>> key) {
        return Optional.of((Registry<E>) registries.computeIfAbsent(key, k -> {
            MappedRegistry r = new MappedRegistry((ResourceKey) k, Lifecycle.stable());
            r.freeze();
            return r;
        }));
    }

    @Override
    public Stream<RegistryEntry<?>> registries() { return Stream.empty(); }
}
