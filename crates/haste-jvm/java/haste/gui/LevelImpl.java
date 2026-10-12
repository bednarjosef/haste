package haste.gui;

import net.minecraft.core.RegistryAccess;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.level.Level;

/** Bodies of the {@code ClientLevel}/{@code Level} proxy (the world is Rust's, world-and-data) and {@code Entity.level()}. */
public final class LevelImpl {
    private LevelImpl() {}

    public static Level level(Entity e) { return Proxies.level; }
    public static boolean isClientSide(Level l) { return true; }
    /** The connection's registries (see {@link Registries}). */
    public static RegistryAccess registryAccess(Level l) { return Registries.access(); }
    /** Vanilla's default feature flags (the server's enabled set is not forwarded yet). */
    public static FeatureFlagSet enabledFeatures(Level l) { return net.minecraft.world.flag.FeatureFlags.DEFAULT_FLAGS; }
    public static FeatureFlagSet enabledFeatures(net.minecraft.client.multiplayer.ClientLevel l) { return enabledFeatures((Level) l); }
}
