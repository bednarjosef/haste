package haste.gui;

import haste.rt.Native;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.level.entity.EntityInLevelCallback;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.food.FoodData;

/**
 * Rust's primitive player state, copied into the player proxy once per frame and per tick ("mirrored once per
 * landmark"): one native call fills two arrays, then the values go where vanilla code reads them: public fields
 * ({@code experienceLevel}), real vanilla value objects held by the proxy ({@link FoodData}, {@link Abilities}, the
 * inventory's selected slot) or fields the proxy's getters return ({@code getHealth}). The array layout is
 * {@code haste_jvm::proxies::mirror} in Rust.
 */
final class Mirror {
    private Mirror() {}

    static final int I_SELECTED = 0, I_GAME_MODE = 1, I_PERMISSION = 2, I_FOOD = 3, I_XP_LEVEL = 4, I_XP_TOTAL = 5,
        I_FLAGS = 6, I_FPS = 7, I_CHUNKS = 8, I_SECTIONS_DRAWN = 9, I_SECTIONS_TOTAL = 10, I_ENTITIES = 11, INTS = 12;
    static final int D_HEALTH = 0, D_SATURATION = 1, D_XP_PROGRESS = 2, D_FLY_SPEED = 3, D_WALK_SPEED = 4, D_X = 5,
        D_Y = 6, D_Z = 7, D_YAW = 8, D_PITCH = 9, D_GAME_TIME = 10, D_DAY_TIME = 11, DOUBLES = 12;
    static final int F_HARDCORE = 1, F_EYE_IN_WATER = 2, F_INVULNERABLE = 4, F_FLYING = 8, F_MAY_FLY = 16,
        F_INSTABUILD = 32, F_IN_WORLD = 64, F_IMMEDIATE_RESPAWN = 128;

    static final int[] I = new int[INTS];
    static final double[] D = new double[DOUBLES];

    /** The vanilla value objects the player proxy hands out (canonical, updated in place). */
    static final FoodData FOOD = new FoodData();
    static final Abilities ABILITIES = new Abilities();

    static boolean flag(int f) { return (I[I_FLAGS] & f) != 0; }
    static boolean inWorld() { return flag(F_IN_WORLD); }
    static float health() { return (float) D[D_HEALTH]; }

    private static boolean callbackSet;

    /** Pulls Rust's state (one native call) and writes it into the proxies. */
    static void update() {
        Native.mirror(I, D);
        LocalPlayer p = Proxies.player;
        p.experienceLevel = I[I_XP_LEVEL];
        p.totalExperience = I[I_XP_TOTAL];
        p.experienceProgress = (float) D[D_XP_PROGRESS];
        FOOD.setFoodLevel(I[I_FOOD]);
        FOOD.setSaturation((float) D[D_SATURATION]);
        ABILITIES.invulnerable = flag(F_INVULNERABLE);
        ABILITIES.flying = flag(F_FLYING);
        ABILITIES.mayfly = flag(F_MAY_FLY);
        ABILITIES.instabuild = flag(F_INSTABUILD);
        ABILITIES.setFlyingSpeed((float) D[D_FLY_SPEED]);
        ABILITIES.setWalkingSpeed((float) D[D_WALK_SPEED]);
        // position and rotation into the entity's own fields (vanilla's setters keep the derived fields current).
        // The proxy is allocated without vanilla's constructor, so Entity.levelCallback (normally initialised
        // to EntityInLevelCallback.NULL) is null, and setPosRaw calls levelCallback.onMove() whenever the block
        // position changes: that threw on every frame the player moved and the whole HUD frame was lost
        // (the flashing vignette and hotbar).
        if (!callbackSet) {
            p.setLevelCallback(EntityInLevelCallback.NULL);
            callbackSet = true;
        }
        p.setPosRaw(D[D_X], D[D_Y], D[D_Z]);
        p.setYRot((float) D[D_YAW]);
        p.setXRot((float) D[D_PITCH]);
        int sel = I[I_SELECTED];
        if (sel >= 0 && sel < 9 && Proxies.inventory.getSelectedSlot() != sel) Proxies.inventory.setSelectedSlot(sel);
    }
}
