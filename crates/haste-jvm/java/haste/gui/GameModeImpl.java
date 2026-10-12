package haste.gui;

import haste.rt.Native;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.ContainerInput;

/**
 * Bodies of the {@code MultiPlayerGameMode} proxy: container clicks go to Rust (which owns the inventory and
 * talks to the server). No client-side prediction yet: the slots change when Rust's state changes.
 */
public final class GameModeImpl {
    private GameModeImpl() {}

    public static void handleContainerInput(MultiPlayerGameMode m, int containerId, int slot, int button, ContainerInput input, Player p) {
        Native.containerInput(containerId, slot, button, input.ordinal());
    }
}
