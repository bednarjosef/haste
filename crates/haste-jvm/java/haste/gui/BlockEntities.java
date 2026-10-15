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

    /** Number of entities in the store (tests). */
    static int count() {
        return BY_POS.size();
    }

    /**
     * The block entity at a position, created empty when the store does not have it: vanilla's
     * client builds it when the block state arrives (`LevelChunk.setBlockState` calls
     * {@code EntityBlock.newBlockEntity(pos, state)} with the real state), which Haste mirrors
     * through Rust's world ({@code haste.rt.Native.blockState}). The state's block must be one
     * the type accepts; otherwise the type's default ({@code BlockEntityType.create} would
     * produce a mismatched entity vanilla's own check drops, e.g. a standing sign where the
     * block is a hanging one).
     */
    static BlockEntity getOrCreate(BlockPos pos, BlockEntityType<?> type) {
        BlockEntity be = BY_POS.get(pos);
        if (be == null || be.getType() != type) {
            be = type.create(pos, stateFor(pos, type));
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
     * The server forgot a chunk (vanilla {@code handleForgetLevelChunk} -> {@code ClientLevel.unload} ->
     * {@code LevelChunk.removeBlockEntity}): its block entities leave the store, so it does not grow
     * without bound over a session of moving around.
     */
    static void dropChunk(int chunkX, int chunkZ) {
        BY_POS.keySet().removeIf(pos -> (pos.getX() >> 4) == chunkX && (pos.getZ() >> 4) == chunkZ);
    }

    /**
     * The state at {@code pos} when it is one of the type's blocks (vanilla's own rule,
     * {@code BlockEntityType.isValid}): the hanging signs draw the hanging-sign art, the wall
     * signs sit sideways, the rotation is the real block's. Falls back to a default state of a
     * block the type accepts ({@code BlockEntityType.validBlocks} is private) when Rust does not
     * know the position (no chunk there, or a state the type does not accept).
     */
    private static BlockState stateFor(BlockPos pos, BlockEntityType<?> type) {
        int id = haste.rt.Native.blockState(pos.getX(), pos.getY(), pos.getZ());
        if (id >= 0) {
            BlockState state = Block.stateById(id);
            if (type.isValid(state)) return state;
        }
        return defaultState(type);
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
     * The entity is created empty when the store does not have it, from the block at the position like
     * vanilla's chunk builds it ({@code LevelChunk.createBlockEntity} -> {@code EntityBlock.newBlockEntity}):
     * a hanging sign block makes a {@code HangingSignBlockEntity} (the hanging-sign editor), a standing
     * or wall sign a plain {@code SignBlockEntity}. Without a state ({@code blockState} -1) the standing
     * sign's editor is the fallback, like before the state was recorded.
     */
    static void openSignEditor(BlockPos pos, SignTextSlot slot) {
        BlockEntity be = BY_POS.get(pos);
        if (be == null) {
            int id = haste.rt.Native.blockState(pos.getX(), pos.getY(), pos.getZ());
            if (id >= 0) {
                BlockState state = Block.stateById(id);
                if (state.getBlock() instanceof net.minecraft.world.level.block.EntityBlock entityBlock) {
                    be = entityBlock.newBlockEntity(pos, state);
                }
            }
            if (be == null) {
                be = net.minecraft.world.level.block.entity.BlockEntityTypes.SIGN.create(pos,
                    defaultState(net.minecraft.world.level.block.entity.BlockEntityTypes.SIGN));
            }
            if (be == null) return;
            be.setLevel(Proxies.level);
            BY_POS.put(pos, be);
        }
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
