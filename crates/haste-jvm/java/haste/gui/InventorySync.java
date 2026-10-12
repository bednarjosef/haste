package haste.gui;

import haste.rt.Native;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;

/**
 * Keeps Rust's mirror of the player's inventory current. The inventory itself is vanilla's (real {@link ItemStack}s
 * in a real {@link Inventory}), because vanilla's menus predict clicks by mutating stacks in place and the server's
 * container packets are applied by vanilla's menus. After every bridge call that can change it, the changed slots
 * (and the carried stack) are encoded with vanilla's item stack codec and handed to Rust, which keeps them as
 * {@code ItemStackData} (item, count, raw component patch) for the renderer and the game logic.
 */
final class InventorySync {
    private InventorySync() {}

    /** What Rust last received: inventory slots (vanilla numbering), then the carried stack. */
    private static ItemStack[] last;

    /** Sends every slot that differs from what Rust last received. Cheap when nothing changed (41 compares). */
    static void sync() {
        Inventory inv = Proxies.inventory;
        int n = inv.getContainerSize();
        if (last == null || last.length != n + 1) {
            last = new ItemStack[n + 1];
        }
        for (int i = 0; i < n; i++) check(i, i, inv.getItem(i));
        net.minecraft.world.inventory.AbstractContainerMenu menu = Proxies.player.containerMenu;
        check(n, -1, menu == null ? ItemStack.EMPTY : menu.getCarried());
    }

    private static void check(int index, int slot, ItemStack now) {
        ItemStack before = last[index];
        if (before != null && ItemStack.matches(before, now)) return;
        last[index] = now.copy();
        Native.inventorySlot(slot, encode(now));
    }

    /** Vanilla's network encoding of an optional stack: count, item id, component patch. */
    static byte[] encode(ItemStack s) {
        RegistryFriendlyByteBuf buf = Registries.buffer();
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, s);
        byte[] b = new byte[buf.readableBytes()];
        buf.readBytes(b);
        return b;
    }

    static ItemStack decode(byte[] b) {
        return ItemStack.OPTIONAL_STREAM_CODEC.decode(Registries.buffer(b));
    }

    /** Forgets what Rust has (a new connection): the next sync sends every slot. */
    static void reset() { last = null; }
}
