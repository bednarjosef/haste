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
    /** Creative mode (Rust game mode). */
    public static boolean hasInfiniteMaterials(Player p) { return Native.gameMode() == 1; }
    public static boolean isCreative(Player p) { return Native.gameMode() == 1; }
    public static boolean isSpectator(Player p) { return Native.gameMode() == 3; }

    // ---- LivingEntity ----
    /** Mob effects are Rust entity state not bridged yet: none (the inventory shows no effect panel). */
    public static java.util.Collection<net.minecraft.world.effect.MobEffectInstance> getActiveEffects(net.minecraft.world.entity.LivingEntity e) {
        return java.util.List.of();
    }

    /** Players can use every equipment slot (armour and both hands). */
    public static boolean canUseSlot(net.minecraft.world.entity.LivingEntity e, net.minecraft.world.entity.EquipmentSlot slot) {
        return e == Proxies.player;
    }

    // ---- LocalPlayer ----
    /** Haste has no account-level chat restrictions (yet): everything allowed. */
    public static net.minecraft.client.multiplayer.chat.ChatAbilities chatAbilities(net.minecraft.client.player.LocalPlayer p) {
        return net.minecraft.client.multiplayer.chat.ChatAbilities.NO_RESTRICTIONS;
    }

    /** The open container closes: Rust is told (it sends the close packet); the screen itself closes as vanilla does. */
    public static void closeContainer(net.minecraft.client.player.LocalPlayer p) {
        Native.containerInput(p.containerMenu == null ? 0 : p.containerMenu.containerId, -1, -1, -1);
    }

    // ---- Inventory ----
    public static ItemStack getItem(Inventory inv, int slot) {
        if (slot < 0 || slot >= Native.inventorySize()) return Proxies.empty;
        return Proxies.stack(slot);
    }
    /** The player inventory's name, vanilla's "container.inventory" translation. */
    public static net.minecraft.network.chat.Component getName(Inventory inv) {
        return net.minecraft.network.chat.Component.translatable("container.inventory");
    }
    public static int getContainerSize(Inventory inv) { return Native.inventorySize(); }
    public static int getSelectedSlot(Inventory inv) { return Native.selectedSlot(); }
    public static ItemStack getSelectedItem(Inventory inv) { return getItem(inv, Native.selectedSlot()); }
}
