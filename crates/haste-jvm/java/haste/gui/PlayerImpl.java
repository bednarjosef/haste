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
    /** Health: the player's from Rust's set_health state, the mount's from its living-entity data. */
    public static float getHealth(net.minecraft.world.entity.LivingEntity e) {
        return e == Proxies.player ? Mirror.health() : Mirror.mountHealth();
    }

    /** Max health: the player's own attribute map; the mount's synced max_health attribute. */
    public static float getMaxHealth(net.minecraft.world.entity.LivingEntity e) {
        return e == Proxies.player
            ? (float) Proxies.player.getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.MAX_HEALTH)
            : Mirror.mountMaxHealth();
    }

    /** Counts the player's effect durations down once per tick, as the client does between server updates. */
    static void tickEffects() {
        for (net.minecraft.world.effect.MobEffectInstance e : Proxies.player.getActiveEffects()) e.tickClient();
    }

    /** Item use and sleeping are Rust's (player): not mirrored yet, so idle values. */
    public static net.minecraft.world.item.ItemStack getActiveItem(net.minecraft.world.entity.LivingEntity e) { return net.minecraft.world.item.ItemStack.EMPTY; }
    /** Vanilla {@code Player.getAttackStrengthScale}: (ticker + partial) / delay, clamped, from Rust's attack charge. */
    public static float getAttackStrengthScale(Player p, float partial) {
        return net.minecraft.util.Mth.clamp((Mirror.attackStrengthTicker() + partial) / Mirror.attackStrengthDelay(), 0f, 1f);
    }
    /** The mount's charged jump scale (vanilla `LocalPlayer.getJumpRidingScale`, Rust's charge state). */
    public static float getJumpRidingScale(net.minecraft.client.player.LocalPlayer p) { return Mirror.jumpRidingScale(); }
    public static int getSleepTimer(Player p) { return 0; }
    public static boolean isScoping(Player p) { return false; }
    /** Vanilla `LocalPlayer.jumpableVehicle`: the mount proxy while Rust steers a horse-family vehicle. */
    public static net.minecraft.world.entity.PlayerRideableJumping jumpableVehicle(net.minecraft.client.player.LocalPlayer p) {
        return Mirror.mountJumpable() ? Proxies.mount : null;
    }
    /** Vanilla `Entity.showVehicleHealth` (the HUD asks the vehicle): the mount is a living entity. */
    public static boolean showVehicleHealth(net.minecraft.world.entity.Entity e) {
        return e != Proxies.player && Mirror.mountLiving();
    }
    public static void playSound(net.minecraft.world.entity.Entity e, net.minecraft.sounds.SoundEvent s, float volume, float pitch) {}
    public static void playSound(Player p, net.minecraft.sounds.SoundEvent s, float volume, float pitch) {}

    // ---- Entity ----
    /** Eyes in water (the only fluid tag the HUD asks about): mirrored from Rust's physics. */
    public static boolean isEyeInFluid(net.minecraft.world.entity.Entity e, net.minecraft.tags.TagKey<net.minecraft.world.level.material.Fluid> tag) {
        return tag == net.minecraft.tags.FluidTags.WATER && Mirror.flag(Mirror.F_EYE_IN_WATER);
    }

    /**
     * Vanilla {@code Entity.getVehicle}: the mount proxy while riding a living vehicle (its health
     * feeds the HUD's vehicle hearts). Boats and other non-living vehicles have no proxy yet, so
     * they answer null like being on foot does.
     */
    public static net.minecraft.world.entity.Entity getVehicle(net.minecraft.world.entity.Entity e) {
        return e == Proxies.player && Mirror.mountLiving() ? Proxies.mount : null;
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
