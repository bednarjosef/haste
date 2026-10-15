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
    /** The real vanilla HUD, chat pipeline and the player's attribute map. */
    public static net.minecraft.client.gui.Hud hud;
    public static net.minecraft.client.gui.components.toasts.ToastManager toasts;
    public static net.minecraft.client.Camera camera;
    public static net.minecraft.client.tutorial.Tutorial tutorial;
    public static net.minecraft.client.multiplayer.chat.ChatListener chatListener;
    public static net.minecraft.world.entity.ai.attributes.AttributeMap attributes;
    /** The server's advancements (real vanilla Java over the forwarded packets). */
    public static net.minecraft.client.multiplayer.ClientAdvancements advancements;

    static void create() {
        minecraft = make(Minecraft.class);
        gui = make(Gui.class);
        font = make(Font.class);
        window = make(Window.class);
        sound = make(SoundManager.class);
        graphics = make(GuiGraphicsExtractor.class);
        player = make(LocalPlayer.class);
        level = make(net.minecraft.client.multiplayer.ClientLevel.class);
        connection = service(net.minecraft.client.multiplayer.ClientPacketListener.class);
        // the player's inventory and equipment are real vanilla containers on the player proxy
        // (PlayerEquipment, like Player.createEquipment: the main hand slot is the selected hotbar stack)
        equipment = new net.minecraft.world.entity.player.PlayerEquipment(player);
        set(player, "equipment", net.minecraft.world.entity.EntityEquipment.class, equipment);
        inventory = new Inventory(player, equipment);
        set(player, "inventory", Inventory.class, inventory);
        // the statistics counter the Stats screen reads and award_stats fills (vanilla's is built
        // with the LocalPlayer; the proxy never runs that constructor)
        set(player, "stats", net.minecraft.stats.StatsCounter.class, new net.minecraft.stats.StatsCounter());

        set(font, "lineHeight", int.class, haste.rt.Native.lineHeight());
        set(minecraft, "font", Font.class, font);
        // the chat input font ("filter fishy" hides suspicious glyphs): the same Font proxy
        set(minecraft, "fontFilterFishy", Font.class, font);
        java.io.File gameDir = new java.io.File(System.getProperty("haste.minecraftDir", "."));
        gameDir.mkdirs();
        set(minecraft, "gameDirectory", java.io.File.class, gameDir);
        // Options is real vanilla Java: every option with its vanilla default, key mappings with their default keys
        // (options.txt in Haste's JVM game directory is read if present; Haste's own config does not feed it yet)
        options = new Options(minecraft, gameDir);
        // no tutorial hints (vanilla starts new installs at the movement step; Haste's players are not new to the game)
        options.tutorialStep = net.minecraft.client.tutorial.TutorialSteps.NONE;
        // Haste plays no music: never show the now-playing toast
        options.musicToast().set(net.minecraft.client.MusicToastDisplayState.NEVER);
        set(minecraft, "options", Options.class, options);
        // every option change (and key binding, polled per tick) reaches Rust live
        OptionBridge.install();
        // which F3 entries are on (vanilla's list; the HUD's crosshair asks it)
        set(minecraft, "debugEntries", net.minecraft.client.gui.components.debug.DebugScreenEntryList.class,
            new net.minecraft.client.gui.components.debug.DebugScreenEntryList(gameDir, net.minecraft.util.datafix.DataFixers.getDataFixer()));
        set(minecraft, "gui", Gui.class, gui);
        set(minecraft, "mouseHandler", net.minecraft.client.MouseHandler.class, make(net.minecraft.client.MouseHandler.class));
        // the player's attributes and effects are real vanilla objects on the proxy (forwarded packets update them)
        attributes = new net.minecraft.world.entity.ai.attributes.AttributeMap(net.minecraft.world.entity.player.Player.createAttributes().build());
        set(player, "attributes", net.minecraft.world.entity.ai.attributes.AttributeMap.class, attributes);
        set(player, "activeEffects", java.util.Map.class, new java.util.HashMap<>());
        // the vanilla value objects Rust's state is mirrored into (see Mirror)
        set(player, "foodData", net.minecraft.world.food.FoodData.class, Mirror.FOOD);
        set(player, "abilities", net.minecraft.world.entity.player.Abilities.class, Mirror.ABILITIES);
        // what Entity's constructor would set: the type, its dimensions and eye height, a position
        var playerType = net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getValue(
            net.minecraft.resources.Identifier.withDefaultNamespace("player"));
        set(player, "type", net.minecraft.world.entity.EntityType.class, playerType);
        set(player, "dimensions", net.minecraft.world.entity.EntityDimensions.class, playerType.getDimensions());
        set(player, "eyeHeight", float.class, playerType.getDimensions().eyeHeight());
        set(player, "position", net.minecraft.world.phys.Vec3.class, net.minecraft.world.phys.Vec3.ZERO);
        set(player, "blockPosition", net.minecraft.core.BlockPos.class, net.minecraft.core.BlockPos.ZERO);
        set(player, "chunkPosition", net.minecraft.world.level.ChunkPos.class, new net.minecraft.world.level.ChunkPos(0, 0));
        set(player, "deltaMovement", net.minecraft.world.phys.Vec3.class, net.minecraft.world.phys.Vec3.ZERO);
        // the player's synched entity data, defined by vanilla's own defineSynchedData chain (kept bodies)
        try {
            var b = new net.minecraft.network.syncher.SynchedEntityData.Builder(player);
            defineEntityBase(b);
            MethodHandles.privateLookupIn(net.minecraft.world.entity.Entity.class, MethodHandles.lookup())
                .findVirtual(net.minecraft.world.entity.Entity.class, "defineSynchedData",
                    MethodType.methodType(void.class, net.minecraft.network.syncher.SynchedEntityData.Builder.class))
                .invoke(player, b);
            set(player, "entityData", net.minecraft.network.syncher.SynchedEntityData.class, b.build());
        } catch (Throwable e) {
            throw new IllegalStateException("defining the player's entity data", e);
        }
        set(minecraft, "player", LocalPlayer.class, player);
        camera = make(net.minecraft.client.Camera.class);
        set(minecraft, "gameRenderer", net.minecraft.client.renderer.GameRenderer.class, make(net.minecraft.client.renderer.GameRenderer.class));
        minecraft.level = level;
        var gameMode = make(net.minecraft.client.multiplayer.MultiPlayerGameMode.class);
        set(gameMode, "minecraft", Minecraft.class, minecraft);
        set(gameMode, "connection", net.minecraft.client.multiplayer.ClientPacketListener.class, connection);
        set(minecraft, "gameMode", net.minecraft.client.multiplayer.MultiPlayerGameMode.class, gameMode);
        set(player, "minecraft", Minecraft.class, minecraft);
        set(player, "connection", net.minecraft.client.multiplayer.ClientPacketListener.class, connection);
        // the recipe book is real vanilla Java; the proxy never runs LocalPlayer's constructor, which is
        // where vanilla builds the book, so the player's own ClientRecipeBook is set here (the kept
        // recipe-book handlers read it through Player.getRecipeBook)
        set(player, "recipeBook", net.minecraft.client.ClientRecipeBook.class, new net.minecraft.client.ClientRecipeBook());
        // LocalPlayer.dropSpamThrottler (vanilla: new TickThrottler(20, 1280) in its constructor,
        // which a proxy never runs): the kept canDropItems body the creative screen's slot clicks
        // go through reads it
        set(player, "dropSpamThrottler", net.minecraft.util.TickThrottler.class,
            new net.minecraft.util.TickThrottler(20, 1280));
        // LivingEntity.swingState (vanilla: new SwingState() in its constructor, which a proxy
        // never runs): the kept drop/swing bodies the creative screen's slot clicks go through
        // start the swing animation on it
        set(player, "swingState", net.minecraft.world.entity.LivingEntity.SwingState.class,
            new net.minecraft.world.entity.LivingEntity.SwingState());
        // the connection's own fields its kept handlers touch (vanilla's constructor sets both): the
        // search trees (the same object searchTrees() returns) and the level for SlotDisplayContext
        set(connection, "searchTrees", net.minecraft.client.multiplayer.SessionSearchTrees.class,
            ConnectionImpl.searchTrees(connection));
        set(connection, "level", net.minecraft.client.multiplayer.ClientLevel.class, level);
        // ClientCommonPacketListenerImpl.minecraft: the kept handlers' thread check reads its packetProcessor
        set(connection, "minecraft", Minecraft.class, minecraft);
        // the HUD and the chat pipeline are real vanilla Java (tier 3) over the proxies
        toasts = new net.minecraft.client.gui.components.toasts.ToastManager(minecraft, options);
        tutorial = new net.minecraft.client.tutorial.Tutorial(minecraft, options);
        hud = new net.minecraft.client.gui.Hud(minecraft);
        set(gui, "hud", net.minecraft.client.gui.Hud.class, hud);
        chatListener = new net.minecraft.client.multiplayer.chat.ChatListener(minecraft);
        // the server's advancements; telemetry is disabled (Haste sends none), like vanilla with
        // telemetry off: only the toast path would use it
        advancements = new net.minecraft.client.multiplayer.ClientAdvancements(minecraft,
            new net.minecraft.client.telemetry.WorldSessionTelemetryManager(
                net.minecraft.client.telemetry.TelemetryEventSender.DISABLED, false, null, null,
                java.util.UUID.randomUUID()));
        // the player's own menu is real vanilla Java over the real Inventory
        var menu = new net.minecraft.world.inventory.InventoryMenu(inventory, true, player);
        set(player, "inventoryMenu", net.minecraft.world.inventory.InventoryMenu.class, menu);
        // no other container open: the player's container menu is its inventory menu
        player.containerMenu = menu;
    }

    /**
     * The entity data every entity has (vanilla defines it in Entity's constructor, which proxies do not run):
     * each {@code EntityDataAccessor} constant of {@code Entity} with its type's empty value; the air supply starts
     * full (300 ticks, the documented default of entity metadata index 1).
     */
    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void defineEntityBase(net.minecraft.network.syncher.SynchedEntityData.Builder b) throws ReflectiveOperationException {
        for (java.lang.reflect.Field f : net.minecraft.world.entity.Entity.class.getDeclaredFields()) {
            if (f.getType() != net.minecraft.network.syncher.EntityDataAccessor.class || !java.lang.reflect.Modifier.isStatic(f.getModifiers())) continue;
            f.setAccessible(true);
            var acc = (net.minecraft.network.syncher.EntityDataAccessor) f.get(null);
            var ser = acc.serializer();
            Object v;
            if (ser == net.minecraft.network.syncher.EntityDataSerializers.BYTE) v = (byte) 0;
            else if (ser == net.minecraft.network.syncher.EntityDataSerializers.INT) v = f.getName().contains("AIR") ? 300 : 0;
            else if (ser == net.minecraft.network.syncher.EntityDataSerializers.BOOLEAN) v = false;
            else if (ser == net.minecraft.network.syncher.EntityDataSerializers.OPTIONAL_COMPONENT) v = java.util.Optional.empty();
            else if (ser == net.minecraft.network.syncher.EntityDataSerializers.POSE) v = net.minecraft.world.entity.Pose.STANDING;
            else throw new IllegalStateException("no default for entity data " + f.getName());
            b.define(acc, v);
        }
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
