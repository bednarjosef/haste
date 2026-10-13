package haste.gui;

import haste.rt.Native;
import net.minecraft.world.entity.player.Player;

/**
 * Impls of the player proxy for state Rust owns (health, game mode, item use, operator level, eyes in water). Every
 * other method of the player keeps vanilla's body (`keepall` in proxies.conf) over the proxy's real vanilla parts.
 */
public final class PlayerImpl {
    private PlayerImpl() {}

    // ---- Player ----
    /** The local player's profile (name and id from the login; no skin properties yet). */
    static com.mojang.authlib.GameProfile profile = new com.mojang.authlib.GameProfile(new java.util.UUID(0, 0), "Player");
    public static com.mojang.authlib.GameProfile getGameProfile(Player p) { return profile; }
    /** Creative mode (Rust game mode). */
    public static boolean hasInfiniteMaterials(Player p) { return gameMode() == 1; }
    public static boolean isCreative(Player p) { return gameMode() == 1; }
    public static boolean isSpectator(Player p) { return gameMode() == 3; }
    static int gameMode() { return Mirror.I[Mirror.I_GAME_MODE]; }


    // ---- LivingEntity ----
    /** Health from Rust's set_health state. */
    public static float getHealth(net.minecraft.world.entity.LivingEntity e) { return e == Proxies.player ? Mirror.health() : 0f; }

    /** Counts the player's effect durations down once per tick, as the client does between server updates. */
    static void tickEffects() {
        for (net.minecraft.world.effect.MobEffectInstance e : Proxies.player.getActiveEffects()) e.tickClient();
    }

    /** Item use, attack cooldown, riding and sleeping are Rust's (player): not mirrored yet, so idle values. */
    public static net.minecraft.world.item.ItemStack getActiveItem(net.minecraft.world.entity.LivingEntity e) { return net.minecraft.world.item.ItemStack.EMPTY; }
    public static float getAttackStrengthScale(Player p, float partial) { return 1f; }
    public static float getJumpRidingScale(net.minecraft.client.player.LocalPlayer p) { return 0f; }
    public static int getSleepTimer(Player p) { return 0; }
    public static boolean isScoping(Player p) { return false; }
    public static net.minecraft.world.entity.PlayerRideableJumping jumpableVehicle(net.minecraft.client.player.LocalPlayer p) { return null; }
    public static boolean showVehicleHealth(net.minecraft.world.entity.LivingEntity e) { return false; }
    public static void playSound(net.minecraft.world.entity.Entity e, net.minecraft.sounds.SoundEvent s, float volume, float pitch) {}
    public static void playSound(Player p, net.minecraft.sounds.SoundEvent s, float volume, float pitch) {}

    // ---- Entity ----
    /** Eyes in water (the only fluid tag the HUD asks about): mirrored from Rust's physics. */
    public static boolean isEyeInFluid(net.minecraft.world.entity.Entity e, net.minecraft.tags.TagKey<net.minecraft.world.level.material.Fluid> tag) {
        return tag == net.minecraft.tags.FluidTags.WATER && Mirror.flag(Mirror.F_EYE_IN_WATER);
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


}
