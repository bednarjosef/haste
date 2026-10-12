package haste.gui;

import haste.rt.Native;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.VarHandle;
import net.minecraft.world.item.ItemStack;

/**
 * Bodies of the {@code ItemStack} proxy: state lives in Rust (inventory slot = handle, -1 = EMPTY) and is read
 * through natives.
 */
public final class StackImpl {
    private StackImpl() {}

    private static final VarHandle HANDLE;
    static {
        try {
            HANDLE = MethodHandles.privateLookupIn(ItemStack.class, MethodHandles.lookup()).findVarHandle(ItemStack.class, "haste$handle", long.class);
        } catch (ReflectiveOperationException e) { throw new ExceptionInInitializerError(e); }
    }

    static long handle(ItemStack s) { return (long) HANDLE.get(s); }
    static void setHandle(ItemStack s, long h) { HANDLE.set(s, h); }

    /** Item registry index, -1 when empty. */
    static int itemId(ItemStack s) { long h = handle(s); return h < 0 ? -1 : Native.stackItem(h); }
    static String displayName(ItemStack s) { int id = itemId(s); return id < 0 ? null : Native.itemName(id); }

    public static int getCount(ItemStack s) { long h = handle(s); return h < 0 ? 0 : Native.stackCount(h); }
    public static void setCount(ItemStack s, int c) { long h = handle(s); if (h >= 0) Native.stackSetCount(h, c); }
    public static boolean isEmpty(ItemStack s) { return itemId(s) < 0 || getCount(s) <= 0; }
    /**
     * Tooltip: one line with the item's registry name from Rust (no translation, components, enchantments or
     * advanced info yet: item data is tier 1, see NOTES.md).
     */
    public static java.util.List<net.minecraft.network.chat.Component> getTooltipLines(ItemStack s, net.minecraft.world.item.Item.TooltipContext ctx,
            net.minecraft.world.entity.player.Player p, net.minecraft.world.item.TooltipFlag flag) {
        String name = displayName(s);
        return name == null ? java.util.List.of() : java.util.List.of(net.minecraft.network.chat.Component.literal(name));
    }
    public static java.util.Optional<net.minecraft.world.inventory.tooltip.TooltipComponent> getTooltipImage(ItemStack s) { return java.util.Optional.empty(); }

    /** The item's holder (identity by registry name; no tags yet). Empty stacks: the holder of item -1 (no key). */
    public static net.minecraft.core.Holder<net.minecraft.world.item.Item> typeHolder(ItemStack s) { return ItemHolder.of(itemId(s)); }

    /** Item components are not bridged from Rust yet: every stack has none. */
    public static net.minecraft.core.component.DataComponentMap getComponents(ItemStack s) { return net.minecraft.core.component.DataComponentMap.EMPTY; }

    public static String toString(ItemStack s) { return isEmpty(s) ? "0 air" : getCount(s) + " " + displayName(s); }
}
