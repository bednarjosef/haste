package haste.gui;

import haste.rt.Native;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;

/** Bodies of the {@code Player}/{@code LocalPlayer} and {@code Inventory} proxies (state in Rust). */
public final class PlayerImpl {
    private PlayerImpl() {}

    // ---- Player ----
    public static Inventory getInventory(Player p) { return Proxies.inventory; }

    // ---- LocalPlayer ----
    /** Haste has no account-level chat restrictions (yet): everything allowed. */
    public static net.minecraft.client.multiplayer.chat.ChatAbilities chatAbilities(net.minecraft.client.player.LocalPlayer p) {
        return net.minecraft.client.multiplayer.chat.ChatAbilities.NO_RESTRICTIONS;
    }

    // ---- Inventory ----
    public static ItemStack getItem(Inventory inv, int slot) {
        if (slot < 0 || slot >= Native.inventorySize()) return Proxies.empty;
        return Proxies.stack(slot);
    }
    public static int getContainerSize(Inventory inv) { return Native.inventorySize(); }
    public static int getSelectedSlot(Inventory inv) { return Native.selectedSlot(); }
    public static ItemStack getSelectedItem(Inventory inv) { return getItem(inv, Native.selectedSlot()); }
}
