package haste.recorder;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Dumps {@code block_entity_types.json}: which block carries which {@code
 * minecraft:block_entity_type}. Two views of the same registry:
 *
 * <ul>
 *   <li>{@code types}: every block entity type with the blocks its valid-block set names
 *       ({@code BlockEntityType}'s own view, through {@code isValid}).
 *   <li>{@code blocks} (authoritative, what the client loads): for every block state where
 *       {@code BlockState.hasBlockEntity()} is true, the type {@code EntityBlock
 *       .newBlockEntity(BlockPos.ZERO, state)} returned — the same call {@code LevelChunk
 *       .setBlockState} makes when a block update places the block, so a plain chest placement
 *       creates its entity client-side. States of one block share the type; a block whose
 *       states disagree, or that returns no entity for some of them (vanilla's do not, but a
 *       mod's {@code newBlockEntity} may vary), is emitted per state under {@code states}.
 * </ul>
 */
public final class BlockEntityTypeDump {
    private BlockEntityTypeDump() {}

    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

    /** Writes {@code block_entity_types.json} into {@code outDir}; returns the block count. */
    static int dump(Path outDir) throws IOException {
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("minecraft", "26.3");
        root.add("types", typesJson());
        JsonObject blocks = new JsonObject();
        JsonObject states = new JsonObject();
        int perStateBlocks = 0;
        for (Block block : BuiltInRegistries.BLOCK) {
            // state -> type id (absent when newBlockEntity returned null or threw)
            Map<Integer, String> stateTypes = new LinkedHashMap<>();
            int entityStates = 0;
            for (BlockState state : block.getStateDefinition().getPossibleStates()) {
                if (!state.hasBlockEntity()) continue;
                entityStates++;
                String type = newBlockEntityType(state);
                if (type != null) stateTypes.put(Block.getId(state), type);
            }
            if (stateTypes.isEmpty()) continue;
            String blockKey = BuiltInRegistries.BLOCK.getKey(block).toString();
            String only = null;
            boolean mixed = stateTypes.size() != entityStates;
            for (String type : stateTypes.values()) {
                if (only == null) only = type;
                else if (!only.equals(type)) mixed = true;
            }
            if (mixed) {
                // The states disagree (or some returned no entity): emit per state.
                perStateBlocks++;
                for (Map.Entry<Integer, String> e : stateTypes.entrySet()) {
                    states.addProperty(Integer.toString(e.getKey()), e.getValue());
                }
            } else {
                blocks.addProperty(blockKey, only);
            }
        }
        root.add("blocks", blocks);
        root.add("states", states);
        if (perStateBlocks > 0) {
            HasteRecorderMod.LOGGER.warn(
                    "block_entity_types.json: {} blocks have per-state types", perStateBlocks);
        }
        writeJson(outDir.resolve("block_entity_types.json"), root);
        return blocks.size();
    }

    /** The type id {@code newBlockEntity} returns for {@code state}, or null when it has none. */
    private static String newBlockEntityType(BlockState state) {
        try {
            BlockEntity entity = ((EntityBlock) state.getBlock())
                    .newBlockEntity(BlockPos.ZERO, state);
            if (entity == null) return null;
            BlockEntityType<?> type = entity.getType();
            return BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(type).toString();
        } catch (Throwable t) {
            HasteRecorderMod.LOGGER.error(
                    "newBlockEntity failed for {}", BuiltInRegistries.BLOCK.getKey(state.getBlock()), t);
            return null;
        }
    }

    /** Every block entity type with its registry id and the blocks its valid-block set names. */
    private static JsonArray typesJson() {
        JsonArray types = new JsonArray();
        for (BlockEntityType<?> type : BuiltInRegistries.BLOCK_ENTITY_TYPE) {
            JsonObject o = new JsonObject();
            o.addProperty("id", BuiltInRegistries.BLOCK_ENTITY_TYPE.getId(type));
            o.addProperty("type", BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(type).toString());
            JsonArray valid = new JsonArray();
            for (Block block : BuiltInRegistries.BLOCK) {
                if (type.isValid(block.defaultBlockState())) {
                    valid.add(BuiltInRegistries.BLOCK.getKey(block).toString());
                }
            }
            o.add("valid_blocks", valid);
            types.add(o);
        }
        return types;
    }

    private static void writeJson(Path path, JsonObject root) throws IOException {
        Files.write(path, GSON.toJson(root).getBytes(StandardCharsets.UTF_8));
    }
}
