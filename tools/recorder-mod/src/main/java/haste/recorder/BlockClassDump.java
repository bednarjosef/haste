package haste.recorder;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;

/**
 * Dumps {@code block_classes.json}: for every block id, the simple names of its Java class and
 * superclasses up to {@link Block} (exclusive). The client's face culling asks the block's
 * {@code skipRendering} override (Block.shouldRenderFace), and which override a block uses is a
 * property of its class (HalfTransparentBlock, IronBarsBlock, LeavesBlock, ...), so the mesher
 * maps class names to culling rules from this dump instead of a hand-written block list.
 */
public final class BlockClassDump {
    private BlockClassDump() {}

    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

    static int dump(Path outDir) throws IOException {
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("minecraft", "26.3");
        JsonObject blocks = new JsonObject();
        for (Block block : BuiltInRegistries.BLOCK) {
            JsonArray chain = new JsonArray();
            for (Class<?> c = block.getClass(); c != BlockBehaviour.class; c = c.getSuperclass()) {
                chain.add(c.getSimpleName());
            }
            blocks.add(BuiltInRegistries.BLOCK.getKey(block).toString(), chain);
        }
        root.add("blocks", blocks);
        Files.write(
                outDir.resolve("block_classes.json"),
                GSON.toJson(root).getBytes(StandardCharsets.UTF_8));
        return blocks.size();
    }
}
