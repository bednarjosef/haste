package haste.gui;

import haste.rt.Native;
import java.lang.invoke.MethodHandles;
import java.util.HashMap;
import java.util.Map;
import java.util.Set;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderOwner;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.item.Item;

/**
 * {@code Holder<Item>} for a Rust item (by registry index): a real vanilla stand-alone {@link Holder.Reference}
 * keyed by the registry name, with no tags and no components bound (item data and tags are tier 1, not bridged
 * yet), and no Item value. Identity checks by name work; tag checks are false. One holder per item.
 */
final class ItemHolder {
    private ItemHolder() {}

    private static final HolderOwner<Item> OWNER = new HolderOwner<>() {};
    private static final Map<Integer, Holder<Item>> BY_ID = new HashMap<>();

    static Holder<Item> of(int item) {
        return BY_ID.computeIfAbsent(item, i -> {
            String name = i < 0 ? "minecraft:air" : Native.itemName(i);
            if (name == null) name = "haste:unknown_" + i;
            Holder.Reference<Item> h = Holder.Reference.createStandAlone(OWNER, ResourceKey.create(Registries.ITEM, Identifier.parse(name)));
            try {
                var l = MethodHandles.privateLookupIn(Holder.Reference.class, MethodHandles.lookup());
                l.findVarHandle(Holder.Reference.class, "tags", Set.class).set(h, Set.of());
                l.findVarHandle(Holder.Reference.class, "components", DataComponentMap.class).set(h, DataComponentMap.EMPTY);
            } catch (ReflectiveOperationException e) {
                throw new IllegalStateException(e);
            }
            return h;
        });
    }
}
