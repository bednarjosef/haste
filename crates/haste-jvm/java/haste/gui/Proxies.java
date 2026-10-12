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

/**
 * The canonical tier-2 proxies. Exactly one Java object per Rust object for the JVM's lifetime (mods will keep
 * state on them): singletons for the client objects. The player's {@link Inventory} is a real vanilla inventory
 * (with real item stacks) attached to the player proxy; see {@link InventorySync}.
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
    public static net.minecraft.world.entity.EntityEquipment equipment;
    public static net.minecraft.client.multiplayer.ClientLevel level;
    public static net.minecraft.client.multiplayer.ClientPacketListener connection;

    static void create() {
        minecraft = make(Minecraft.class);
        gui = make(Gui.class);
        font = make(Font.class);
        options = make(Options.class);
        window = make(Window.class);
        sound = make(SoundManager.class);
        graphics = make(GuiGraphicsExtractor.class);
        player = make(LocalPlayer.class);
        level = make(net.minecraft.client.multiplayer.ClientLevel.class);
        connection = service(net.minecraft.client.multiplayer.ClientPacketListener.class);
        // the player's inventory and equipment are real vanilla containers on the player proxy
        equipment = new net.minecraft.world.entity.EntityEquipment();
        set(player, "equipment", net.minecraft.world.entity.EntityEquipment.class, equipment);
        inventory = new Inventory(player, equipment);
        set(player, "inventory", Inventory.class, inventory);

        set(font, "lineHeight", int.class, haste.rt.Native.lineHeight());
        set(minecraft, "font", Font.class, font);
        // the chat input font ("filter fishy" hides suspicious glyphs): the same Font proxy
        set(minecraft, "fontFilterFishy", Font.class, font);
        set(minecraft, "options", Options.class, options);
        set(minecraft, "gui", Gui.class, gui);
        set(minecraft, "mouseHandler", net.minecraft.client.MouseHandler.class, make(net.minecraft.client.MouseHandler.class));
        set(gui, "hud", net.minecraft.client.gui.Hud.class, make(net.minecraft.client.gui.Hud.class));
        set(minecraft, "player", LocalPlayer.class, player);
        minecraft.level = level;
        set(minecraft, "gameMode", net.minecraft.client.multiplayer.MultiPlayerGameMode.class,
            make(net.minecraft.client.multiplayer.MultiPlayerGameMode.class));
        set(player, "connection", net.minecraft.client.multiplayer.ClientPacketListener.class, connection);
        KeyImpl.installDefaults(options);
        // the recipe book is a no-op proxy (no <clinit>), but InventoryScreen still shows its toggle button:
        // the sprites are the two that exist in the jar under textures/gui/sprites/recipe_book/
        setStatic(net.minecraft.client.gui.screens.recipebook.RecipeBookComponent.class, "RECIPE_BUTTON_SPRITES",
            net.minecraft.client.gui.components.WidgetSprites.class,
            new net.minecraft.client.gui.components.WidgetSprites(
                net.minecraft.resources.Identifier.withDefaultNamespace("recipe_book/button"),
                net.minecraft.resources.Identifier.withDefaultNamespace("recipe_book/button_highlighted")));
        // the player's own menu is real vanilla Java over the Rust-backed Inventory proxy
        var menu = new net.minecraft.world.inventory.InventoryMenu(inventory, true, player);
        set(player, "inventoryMenu", net.minecraft.world.inventory.InventoryMenu.class, menu);
        // no other container open: the player's container menu is its inventory menu
        player.containerMenu = menu;
    }

    private static final Map<Class<?>, Object> SERVICES = new HashMap<>();

    /** The canonical proxy of a client service Haste does not have (all methods no-ops), created on first use. */
    static <T> T service(Class<T> c) {
        return c.cast(SERVICES.computeIfAbsent(c, Proxies::make));
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
            } catch (NoSuchFieldException | IllegalAccessException e) {
                // declared in (and private to) a superclass: try there
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
