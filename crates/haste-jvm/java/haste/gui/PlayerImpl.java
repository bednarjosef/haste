package haste.gui;

import haste.rt.Native;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;

/** Bodies of the {@code Player}/{@code LocalPlayer} proxy (state in Rust; the inventory is a real vanilla one). */
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
    /** The server's operator level for this player (entity events 24-28), as vanilla's level-based set. */
    public static net.minecraft.server.permissions.PermissionSet permissions(net.minecraft.client.player.LocalPlayer p) {
        return permissionSet(Native.permissionLevel());
    }

    static net.minecraft.server.permissions.PermissionSet permissionSet(int level) {
        return switch (level) {
            case 1 -> net.minecraft.server.permissions.LevelBasedPermissionSet.MODERATOR;
            case 2 -> net.minecraft.server.permissions.LevelBasedPermissionSet.GAMEMASTER;
            case 3 -> net.minecraft.server.permissions.LevelBasedPermissionSet.ADMIN;
            case 4 -> net.minecraft.server.permissions.LevelBasedPermissionSet.OWNER;
            default -> net.minecraft.server.permissions.LevelBasedPermissionSet.ALL;
        };
    }

    /** Haste has no account-level chat restrictions (yet): everything allowed. */
    public static net.minecraft.client.multiplayer.chat.ChatAbilities chatAbilities(net.minecraft.client.player.LocalPlayer p) {
        return net.minecraft.client.multiplayer.chat.ChatAbilities.NO_RESTRICTIONS;
    }

    /** The open container closes: Rust is told (it sends the close packet); the screen itself closes as vanilla does. */
    public static void closeContainer(net.minecraft.client.player.LocalPlayer p) {
        Native.containerInput(p.containerMenu == null ? 0 : p.containerMenu.containerId, -1, -1, -1);
    }

}
