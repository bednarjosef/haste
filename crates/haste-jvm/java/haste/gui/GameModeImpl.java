package haste.gui;

import net.minecraft.client.multiplayer.MultiPlayerGameMode;

/**
 * Bodies of the {@code MultiPlayerGameMode} proxy: the game mode is Rust's. Container clicks keep vanilla's bodies
 * (proxies.conf): vanilla predicts the click on the real menu and sends container_click with the hashed stacks.
 */
public final class GameModeImpl {
    private GameModeImpl() {}

    /** The local player's game mode (Rust's state: login, respawn, game_event). */
    public static net.minecraft.world.level.GameType getPlayerMode(MultiPlayerGameMode m) {
        return net.minecraft.world.level.GameType.byId(PlayerImpl.gameMode());
    }
    public static boolean canHurtPlayer(MultiPlayerGameMode m) { return getPlayerMode(m).isSurvival(); }
    public static boolean hasExperience(MultiPlayerGameMode m) { return getPlayerMode(m).isSurvival(); }

}
