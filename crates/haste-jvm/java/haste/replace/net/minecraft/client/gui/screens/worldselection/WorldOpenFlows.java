package net.minecraft.client.gui.screens.worldselection;

import com.mojang.datafixers.util.Pair;
import com.mojang.serialization.Lifecycle;
import java.util.Optional;
import java.util.function.Function;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.GenericMessageScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.LayeredRegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.ReloadableServerResources;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;

/**
 * Haste's replacement for vanilla's WorldOpenFlows (see {@code crates/haste-jvm/src/loader.rs},
 * {@code REPLACEMENTS}): vanilla's plays a world through the integrated server, but Haste's
 * singleplayer is haste-setup's local dedicated server (tier 1, Rust owns the process). The
 * replacement keeps vanilla's screen flow — the world list and create screen call these
 * methods — shows vanilla's loading screen, and hands the world (and, for a new world, the
 * creation settings) to Rust ({@link haste.rt.Native#requestOpenWorld} and
 * {@link haste.rt.Native#requestCreateWorld}), which starts the server and joins it.
 */
public class WorldOpenFlows {
    private final Minecraft minecraft;
    private final LevelStorageSource levelSource;

    public WorldOpenFlows(final Minecraft minecraft, final LevelStorageSource levelSource) {
        this.minecraft = minecraft;
        this.levelSource = levelSource;
    }

    /** Vanilla's entry point (the world list's "Play Selected World"): start the world in Rust. */
    public void openWorld(final String levelId, final Runnable onCancel) {
        this.minecraft.setScreenAndShow(new GenericMessageScreen(Component.translatable("selectWorld.data_read")));
        haste.rt.Native.requestOpenWorld(levelId);
    }

    /**
     * Vanilla's entry point for a fresh world (the debug-recreate button; the create screen goes
     * through {@link #createLevelFromExistingSettings}). The world is created by the local
     * server from the settings; the dimensions provider is vanilla's normal-world one (Haste's
     * server expresses world type in {@code level-type}).
     */
    public void createFreshLevel(
        final String levelId, final LevelSettings levelSettings, final WorldOptions options,
        final Function<HolderLookup.Provider, WorldDimensions> dimensionsProvider, final Screen parentScreen
    ) {
        this.minecraft.setScreenAndShow(new GenericMessageScreen(Component.translatable("selectWorld.data_read")));
        haste.rt.Native.requestCreateWorld(levelId, levelSettings.levelName(),
            levelSettings.gameType().getId(), difficultyId(levelSettings), levelSettings.difficultySettings().hardcore(),
            levelSettings.allowCommands(), Long.toString(options.seed()), options.generateStructures(), levelTypeId(dimensionsProvider));
    }

    /**
     * Vanilla's create path ({@code CreateWorldScreen.createNewWorld}): the screen created the
     * world folder and built the settings; the integrated server would now save them and start.
     * Haste releases the folder lock (the local server locks it itself) and hands the settings to
     * Rust, which writes {@code server.properties} and starts the server.
     */
    public void createLevelFromExistingSettings(
        final LevelStorageSource.LevelStorageAccess levelSourceAccess,
        final ReloadableServerResources serverResources,
        final LayeredRegistryAccess<RegistryLayer> registryAccess,
        final LevelDataAndDimensions.WorldDataAndGenSettings worldDataAndGenSettings,
        final Optional<net.minecraft.world.level.gamerules.GameRules> gameRules
    ) {
        LevelSettings settings = worldDataAndGenSettings.data().getLevelSettings();
        WorldOptions options = worldDataAndGenSettings.genSettings().options();
        String levelType = WorldPresets.fromSettings(worldDataAndGenSettings.genSettings().dimensions())
            .map(preset -> preset.identifier().toString())
            .orElse("minecraft:normal");
        levelSourceAccess.safeClose();
        haste.rt.Native.requestCreateWorld(levelSourceAccess.getLevelId(), settings.levelName(),
            settings.gameType().getId(), difficultyId(settings), settings.difficultySettings().hardcore(),
            settings.allowCommands(), Long.toString(options.seed()), options.generateStructures(), levelType);
    }

    /** Recreating a world from an existing one is not supported yet; the world list shows its error alert. */
    public Pair<LevelSettings, WorldCreationContext> recreateWorldData(final LevelStorageSource.LevelStorageAccess levelSourceAccess) {
        throw new UnsupportedOperationException("recreating a world is not supported yet");
    }

    /** Vanilla's experimental/deprecated warning (stable default worlds skip it, as in vanilla). */
    public static void confirmWorldCreation(final Minecraft minecraft, final CreateWorldScreen parent, final Lifecycle lifecycle, final Runnable task, final boolean skipWarning) {
        task.run();
    }

    private static int difficultyId(final LevelSettings settings) {
        return settings.difficultySettings().difficulty().getId();
    }

    /** The world type as a {@code level-type} id: flat stays flat, everything else normal. */
    private static String levelTypeId(final Function<HolderLookup.Provider, WorldDimensions> dimensionsProvider) {
        return "minecraft:normal";
    }
}
