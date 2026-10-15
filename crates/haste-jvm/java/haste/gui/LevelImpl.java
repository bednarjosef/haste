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
    /** The client scoreboard (scoreboard packets, see {@link Handlers}). */
    public static net.minecraft.world.scores.Scoreboard getScoreboard(Level l) { return Handlers.SCOREBOARD; }
    public static net.minecraft.world.scores.Scoreboard getScoreboard(net.minecraft.client.multiplayer.ClientLevel l) { return Handlers.SCOREBOARD; }

    public static void disconnect(net.minecraft.client.multiplayer.ClientLevel l, net.minecraft.network.chat.Component reason) { haste.rt.Native.requestDisconnect(); }

    /** The server's enabled feature flags (from the configuration phase; vanilla's defaults until then). */
    static net.minecraft.world.flag.FeatureFlagSet features = net.minecraft.world.flag.FeatureFlags.DEFAULT_FLAGS;

    /** World age in ticks (Rust's set_time state). */
    public static long getGameTime(Level l) { return (long) Mirror.D[Mirror.D_GAME_TIME]; }
    public static long getGameTime(net.minecraft.client.multiplayer.ClientLevel l) { return getGameTime((Level) l); }
    public static long getDayTime(Level l) { return (long) Mirror.D[Mirror.D_DAY_TIME]; }
    /** The world border is not forwarded yet: vanilla's default (no border in reach, no vignette). */
    static final net.minecraft.world.level.border.WorldBorder BORDER = new net.minecraft.world.level.border.WorldBorder();
    public static net.minecraft.world.level.border.WorldBorder getWorldBorder(Level l) { return BORDER; }
    public static net.minecraft.world.level.border.WorldBorder getWorldBorder(net.minecraft.client.multiplayer.ClientLevel l) { return BORDER; }
    static final net.minecraft.world.TickRateManager TICK_RATE = new net.minecraft.world.TickRateManager();
    public static net.minecraft.world.TickRateManager tickRateManager(Level l) { return TICK_RATE; }
    public static net.minecraft.world.TickRateManager tickRateManager(net.minecraft.client.multiplayer.ClientLevel l) { return TICK_RATE; }
    public static net.minecraft.world.entity.player.Player getPlayerByUUID(Level l, java.util.UUID id) {
        return id.equals(Proxies.player.getUUID()) ? Proxies.player : null;
    }

    /** The level's players (vanilla {@code ClientLevel.players}): only the local player is here. */
    public static java.util.List<net.minecraft.client.player.AbstractClientPlayer> players(net.minecraft.client.multiplayer.ClientLevel l) {
        return java.util.List.of(Proxies.player);
    }
    /** Difficulty and hardcore of the joined world (difficulty not forwarded yet: normal). */
    static net.minecraft.client.multiplayer.ClientLevel.ClientLevelData levelData;
    public static net.minecraft.client.multiplayer.ClientLevel.ClientLevelData getLevelData(net.minecraft.client.multiplayer.ClientLevel l) {
        boolean hardcore = Mirror.flag(Mirror.F_HARDCORE);
        if (levelData == null || levelData.isHardcore() != hardcore) {
            levelData = new net.minecraft.client.multiplayer.ClientLevel.ClientLevelData(net.minecraft.world.Difficulty.NORMAL, hardcore, false);
        }
        return levelData;
    }
    public static net.minecraft.world.level.storage.LevelData getLevelData(Level l) { return getLevelData((net.minecraft.client.multiplayer.ClientLevel) l); }
    /** The dimension type of the joined world, from the synchronised registry (Rust passes its id at login). */
    static int dimensionTypeId;
    public static net.minecraft.world.level.dimension.DimensionType dimensionType(Level l) {
        var reg = Registries.access().lookupOrThrow(net.minecraft.core.registries.Registries.DIMENSION_TYPE);
        var t = reg.byId(dimensionTypeId);
        return t != null ? t : reg.getValueOrThrow(net.minecraft.world.level.dimension.BuiltinDimensionTypes.OVERWORLD);
    }
    /** Blocks are Rust's (world-and-data): the HUD's camera checks see air in full light. */
    public static net.minecraft.world.level.block.state.BlockState getBlockState(Level l, net.minecraft.core.BlockPos p) {
        return net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
    }

    /**
     * The block entities the screens edit (see {@link BlockEntities}); the world's own block entities are
     * Rust's and not mirrored — the store only holds what a screen reached (signs, command blocks, ...).
     */
    public static net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(Level l, net.minecraft.core.BlockPos p) {
        return BlockEntities.get(p);
    }
    public static net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(net.minecraft.client.multiplayer.ClientLevel l, net.minecraft.core.BlockPos p) {
        return getBlockEntity((Level) l, p);
    }

    // The world's change propagation is Rust's/server's; the block entities Java holds are for the
    // screens (BlockEntities), so the updates a screen's edit marks are no-ops here.
    public static void sendBlockUpdated(Level l, net.minecraft.core.BlockPos p, net.minecraft.world.level.block.state.BlockState old,
                                         net.minecraft.world.level.block.state.BlockState cur, int flags) {}
    public static void blockEntityChanged(Level l, net.minecraft.core.BlockPos p) {}
    public static void updateNeighbourForOutputSignal(Level l, net.minecraft.core.BlockPos p, net.minecraft.world.level.block.Block b) {}
    public static int getMaxLocalRawBrightness(Level l, net.minecraft.core.BlockPos p) { return 15; }
    static final net.minecraft.world.level.lighting.LevelLightEngine LIGHT = Proxies.make(net.minecraft.world.level.lighting.LevelLightEngine.class);
    public static net.minecraft.world.level.lighting.LevelLightEngine getLightEngine(Level l) { return LIGHT; }
    public static int getRawBrightness(net.minecraft.world.level.lighting.LevelLightEngine e, net.minecraft.core.BlockPos p, int darken) {
        return Math.max(0, 15 - darken);
    }
    /** Sky darkening (night, weather) is not mirrored yet: daylight. */
    public static int getSkyDarken(Level l) { return 0; }

    public static FeatureFlagSet enabledFeatures(Level l) { return features; }
    public static FeatureFlagSet enabledFeatures(net.minecraft.client.multiplayer.ClientLevel l) { return enabledFeatures((Level) l); }
}
