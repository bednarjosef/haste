package haste.gui;

import com.mojang.blaze3d.platform.Window;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.util.HashMap;
import java.util.Map;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;

/**
 * The canonical tier-2 proxies. Exactly one Java object per Rust object for the JVM's lifetime (mods will keep
 * state on them): singletons for the client objects, one {@link ItemStack} per inventory slot handle.
 */
public final class Proxies {
    private Proxies() {}

    public static Minecraft minecraft;
    public static Gui gui;
    public static Font font;
    public static Options options;
    public static Window window;
    public static SoundManager sound;
    public static GuiGraphicsExtractor graphics;
    public static LocalPlayer player;
    public static Inventory inventory;
    public static ItemStack empty;
    private static final Map<Long, ItemStack> STACKS = new HashMap<>();

    static void create() {
        minecraft = make(Minecraft.class);
        gui = make(Gui.class);
        font = make(Font.class);
        options = make(Options.class);
        window = make(Window.class);
        sound = make(SoundManager.class);
        graphics = make(GuiGraphicsExtractor.class);
        player = make(LocalPlayer.class);
        inventory = make(Inventory.class);
        empty = make(ItemStack.class);
        StackImpl.setHandle(empty, -1);
        setStatic(ItemStack.class, "EMPTY", ItemStack.class, empty);

        set(font, "lineHeight", int.class, haste.rt.Native.lineHeight());
        set(minecraft, "font", Font.class, font);
        set(minecraft, "options", Options.class, options);
        set(minecraft, "gui", Gui.class, gui);
        set(minecraft, "mouseHandler", net.minecraft.client.MouseHandler.class, make(net.minecraft.client.MouseHandler.class));
        set(gui, "hud", net.minecraft.client.gui.Hud.class, make(net.minecraft.client.gui.Hud.class));
        set(minecraft, "player", LocalPlayer.class, player);
        set(player, "connection", net.minecraft.client.multiplayer.ClientPacketListener.class,
            service(net.minecraft.client.multiplayer.ClientPacketListener.class));
        KeyImpl.installDefaults(options);
    }

    private static final Map<Class<?>, Object> SERVICES = new HashMap<>();

    /** The canonical proxy of a client service Haste does not have (all methods no-ops), created on first use. */
    static <T> T service(Class<T> c) {
        return c.cast(SERVICES.computeIfAbsent(c, Proxies::make));
    }

    /** The canonical proxy for the stack in inventory slot {@code slot}. */
    public static ItemStack stack(long slot) {
        if (slot < 0) return empty;
        return STACKS.computeIfAbsent(slot, h -> { ItemStack s = make(ItemStack.class); StackImpl.setHandle(s, h); return s; });
    }

    /**
     * Instantiates a generated proxy through its generated no-arg constructor. Method handles, not core
     * reflection: reflection would resolve every constructor/field type of the class (most are refused).
     */
    static <T> T make(Class<T> c) {
        try {
            var l = MethodHandles.privateLookupIn(c, MethodHandles.lookup());
            return c.cast(l.findConstructor(c, MethodType.methodType(void.class)).invoke());
        } catch (Throwable e) {
            throw new IllegalStateException("cannot create proxy " + c.getName(), e);
        }
    }

    /** Sets a proxy instance field that is final in vanilla (the generator drops final), by name and type. */
    static void set(Object o, String field, Class<?> type, Object v) {
        for (Class<?> k = o.getClass(); k != null; k = k.getSuperclass()) {
            try {
                MethodHandles.privateLookupIn(k, MethodHandles.lookup()).findVarHandle(k, field, type).set(o, v);
                return;
            } catch (NoSuchFieldException e) {
                // try the superclass
            } catch (IllegalAccessException e) {
                throw new IllegalStateException(e);
            }
        }
        throw new IllegalStateException("no field " + field + " on " + o.getClass().getName());
    }

    static void setStatic(Class<?> c, String field, Class<?> type, Object v) {
        try {
            MethodHandles.privateLookupIn(c, MethodHandles.lookup()).findStaticVarHandle(c, field, type).set(v);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException("cannot set " + c.getName() + "." + field, e);
        }
    }
}
