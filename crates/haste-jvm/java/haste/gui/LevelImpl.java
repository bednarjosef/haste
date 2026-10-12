package haste.gui;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;

/** Bodies of the {@code ClientLevel}/{@code Level} proxy (the world is Rust's, WS2) and {@code Entity.level()}. */
public final class LevelImpl {
    private LevelImpl() {}

    public static Level level(Entity e) { return Proxies.level; }
    public static boolean isClientSide(Level l) { return true; }
}
