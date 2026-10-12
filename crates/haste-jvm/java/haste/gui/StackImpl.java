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
    public static String toString(ItemStack s) { return isEmpty(s) ? "0 air" : getCount(s) + " " + displayName(s); }
}
