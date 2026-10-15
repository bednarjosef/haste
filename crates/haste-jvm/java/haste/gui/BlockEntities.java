package haste.gui;

import java.lang.invoke.MethodHandles;
import java.util.HashMap;
import java.util.Map;
import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.entity.SignBlockEntity;
import net.minecraft.world.level.block.entity.SignTextSlot;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.nbt.CompoundTag;

/**
 * The block entities the screens edit: one canonical vanilla {@link BlockEntity} per position, created and
 * updated from the forwarded {@code block_entity_data} packets (the world itself is Rust's, world-and-data,
 * so this store is the tier-3 shadow of {@code ClientLevel}'s per-chunk block entities). Vanilla
 * {@code ClientPacketListener.handleOpenSignEditor}/{@code handleBlockEntityData} reach these through
 * {@code LevelImpl.getBlockEntity}.
 */
public final class BlockEntities {
    private BlockEntities() {}

    /** Position -> the canonical block entity (one per position, like a chunk's map). */
    static final Map<BlockPos, BlockEntity> BY_POS = new HashMap<>();

    /** The block entity at a position, or null (vanilla {@code Level.getBlockEntity}). */
    static BlockEntity get(BlockPos pos) {
        return BY_POS.get(pos);
    }

    /**
     * The block entity at a position, created empty when the store does not have it: vanilla's
     * client builds it when the block state arrives (`LevelChunk.setBlockState`, and the chunk's
     * own entity list), which Haste only mirrors for the entities a screen reached — the use of
     * a game-master block or a freshly placed sign usually arrives before any
     * {@code block_entity_data} named it.
     */
    static BlockEntity getOrCreate(BlockPos pos, BlockEntityType<?> type) {
        BlockEntity be = BY_POS.get(pos);
        if (be == null || be.getType() != type) {
            be = type.create(pos, defaultState(type));
            if (be == null) return null;
            be.setLevel(Proxies.level);
            BY_POS.put(pos, be);
        }
        return be;
    }

    /** Called when the world is left (vanilla unloads the chunks' block entities with them). */
    static void clear() {
        BY_POS.clear();
    }

    /**
     * A default state of a block the type accepts ({@code BlockEntityType.validBlocks} is private): the sign
     * and command-block editors only read the block family off it (wood type, wall vs standing) and the
     * rotation lives in Rust's world, not in the screen.
     */
    private static BlockState defaultState(BlockEntityType<?> type) {
        try {
            Set<Block> blocks = (Set<Block>) MethodHandles.privateLookupIn(BlockEntityType.class,
                MethodHandles.lookup()).findVarHandle(BlockEntityType.class, "validBlocks", Set.class).get(type);
            if (!blocks.isEmpty()) return blocks.iterator().next().defaultBlockState();
        } catch (Throwable e) {
            throw new IllegalStateException("cannot read the valid blocks of " + type, e);
        }
        throw new IllegalStateException("block entity type " + type + " has no valid blocks");
    }

    /**
     * The block entity at {@code pos}, of the type the packet names: created from the type's first valid
     * block's default state when nothing is there yet (vanilla's {@code LevelChunk.setBlockState} creates it
     * when the block arrives), then loaded from the packet's tag like vanilla
     * {@code ClientPacketListener.handleBlockEntityData} does. Returns the entity (null when the type
     * cannot create one).
     */
    static BlockEntity load(BlockPos pos, BlockEntityType<?> type, CompoundTag tag) {
        BlockEntity be = getOrCreate(pos, type);
        if (be == null) return null;
        try (var reporter = new net.minecraft.util.ProblemReporter.ScopedCollector(
            be.problemPath(), org.slf4j.LoggerFactory.getLogger(BlockEntities.class))) {
            be.loadWithComponents(net.minecraft.world.level.storage.TagValueInput.create(
                reporter, Registries.access(), tag));
        }
        return be;
    }

    /**
     * {@code ClientPacketListener.handleOpenSignEditor}: the sign at the position goes to vanilla's text
     * editor, standing or hanging as the entity's class says (vanilla {@code LocalPlayer.openTextEdit}).
     * The entity is created empty when the store does not have it: vanilla's client builds it when the
     * sign's block state arrives (`LevelChunk.setBlockState`), which Haste does not mirror — the first
     * packet naming a freshly placed sign is usually this one, not {@code block_entity_data}.
     */
    static void openSignEditor(BlockPos pos, SignTextSlot slot) {
        BlockEntity be = getOrCreate(pos, net.minecraft.world.level.block.entity.BlockEntityTypes.SIGN);
        if (be instanceof SignBlockEntity sign) {
            if (sign instanceof net.minecraft.world.level.block.entity.HangingSignBlockEntity hanging) {
                Proxies.gui.setScreen(new net.minecraft.client.gui.screens.inventory.HangingSignEditScreen(
                    hanging, slot, ClientImpl.isTextFilteringEnabled(Proxies.minecraft)));
            } else {
                Proxies.gui.setScreen(new net.minecraft.client.gui.screens.inventory.SignEditScreen(
                    sign, slot, ClientImpl.isTextFilteringEnabled(Proxies.minecraft)));
            }
        } else {
            org.slf4j.LoggerFactory.getLogger(BlockEntities.class)
                .warn("Ignoring openTextEdit on an invalid entity: {} at pos {}", be, pos);
        }
    }
}
