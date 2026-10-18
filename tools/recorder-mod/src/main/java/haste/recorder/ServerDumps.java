package haste.recorder;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.authlib.GameProfile;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Locale;
import java.util.UUID;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateHolder;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;

/**
 * Dumps the server-side oracle data to {@code cache/oracle/26.3/} (resolved relative to the server
 * directory, see the dump script). Everything here reads the running server's own registries and
 * calls the game's public shape/accessor methods with an empty level: observation, not translation.
 */
public final class ServerDumps {
    private ServerDumps() {}

    private static final BlockPos ORIGIN = BlockPos.ZERO;
    private static final CollisionContext EMPTY_CONTEXT = CollisionContext.empty();
    /** heuristically-marked "shape may depend on neighbours" flags; see NOTES in FORMATS.md */
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

    /** Called from the tick mixin once the server's first tick is done (registries booted, level loaded). */
    public static void dumpAll(MinecraftServer server) {
        Path outDir = outputDir(server);
        try {
            Files.createDirectories(outDir);
            long t0 = System.nanoTime();
            int states = dumpBlockShapes(outDir);
            HasteRecorderMod.LOGGER.info("block_shapes.json: {} states", states);
            int entities = dumpEntityDimensions(outDir, server);
            HasteRecorderMod.LOGGER.info("entity_dimensions.json: {} entity types", entities);
            int blocks = dumpBlockProperties(outDir);
            HasteRecorderMod.LOGGER.info("block_properties.json: {} blocks", blocks);
            int beBlocks = BlockEntityTypeDump.dump(outDir);
            HasteRecorderMod.LOGGER.info("block_entity_types.json: {} blocks", beBlocks);
            HasteRecorderMod.LOGGER.info("oracle dumps complete in {} ms", (System.nanoTime() - t0) / 1_000_000);
        } catch (Throwable t) {
            HasteRecorderMod.LOGGER.error("oracle dump failed", t);
        }
    }

    /**
     * Output dir: the {@code haste.oracle.out} system property when the dump script sets it,
     * otherwise {@code <server dir>/oracle-dumps/26.3} (the server dir's parent is unreliable:
     * getServerDirectory may return the launch working directory, not the install root).
     */
    static Path outputDir(MinecraftServer server) {
        String out = System.getProperty("haste.oracle.out");
        if (out != null && !out.isBlank()) return Path.of(out);
        return server.getServerDirectory().resolve("oracle-dumps").resolve("26.3");
    }

    // ---------------------------------------------------------------- block_shapes.json

    static int dumpBlockShapes(Path outDir) throws IOException {
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("minecraft", "26.3");
        JsonArray states = new JsonArray();
        int n = Block.BLOCK_STATE_REGISTRY.size();
        for (int id = 0; id < n; id++) {
            BlockState state = Block.stateById(id);
            if (state == null) continue;
            states.add(stateJson(id, state));
        }
        root.add("states", states);
        writeJson(outDir.resolve("block_shapes.json"), root);
        return states.size();
    }

    private static JsonObject stateJson(int id, BlockState state) {
        JsonObject o = new JsonObject();
        o.addProperty("id", id);
        Identifier key = BuiltInRegistries.BLOCK.getKey(state.getBlock());
        o.addProperty("block", key.toString());
        JsonObject props = new JsonObject();
        for (Property<?> p : state.getProperties()) {
            props.addProperty(p.getName(), propertyNameValue(state, p));
        }
        o.add("properties", props);

        // Shapes with an empty level at the origin. State-dependent blocks (fences, walls, ...)
        // return their no-neighbour shape here; we mark those with the neighbours flag.
        VoxelShape collision = state.getCollisionShape(EmptyBlockGetter.INSTANCE, ORIGIN, EMPTY_CONTEXT);
        o.add("collision", shapeBoxes(collision));
        VoxelShape outline = state.getShape(EmptyBlockGetter.INSTANCE, ORIGIN, EMPTY_CONTEXT);
        o.add("outline", shapeBoxes(outline));
        // BlockState.getOcclusionShape: what neighbour/fluid face culling tests against
        // (getShape by default, empty for the noOcclusion family, smaller for fences and
        // the other overrides). Absent in dumps from before this field.
        VoxelShape occlusion = state.getOcclusionShape();
        o.add("occlusion", shapeBoxes(occlusion));

        o.addProperty("light", state.getLightEmission());
        // BlockState.getLightDampening / propagatesSkylightDown: what the client light engine
        // (LightEngine, SkyLightEngine) reads for every block it propagates through.
        o.addProperty("light_dampening", state.getLightDampening());
        o.addProperty("propagates_skylight_down", state.propagatesSkylightDown());
        // BlockState.canOcclude / useShapeForLightOcclusion: LightEngine's gates for
        // shape-based light occlusion (isEmptyShape, shapeOccludes). Absent in dumps from
        // before this field.
        o.addProperty("can_occlude", state.canOcclude());
        o.addProperty("use_shape_for_light_occlusion", state.useShapeForLightOcclusion());
        o.addProperty("air", state.isAir());
        o.addProperty("opaque", state.isSolidRender());
        // EntityRenderer.extractShadowPiece skips INVISIBLE render shapes; RenderShape has only
        // INVISIBLE and MODEL in 26.3, so the lowercase enum name is the whole domain.
        o.addProperty("render_shape", state.getRenderShape().name().toLowerCase(Locale.ROOT));
        // Level.collidesWithSuffocatingBlock -> BlockState.isSuffocating with an empty level: the
        // default predicate is tag + isCollisionShapeFullBlock (no-neighbour shape here); see
        // NOTES.md for the two predicates that read more than the state.
        o.addProperty("suffocating", state.isSuffocating(EmptyBlockGetter.INSTANCE, ORIGIN));
        if (state.liquid() || !state.getFluidState().isEmpty()) {
            Identifier fluid = BuiltInRegistries.FLUID.getKey(state.getFluidState().getType());
            o.addProperty("fluid", fluid.toString());
            o.addProperty("fluid_empty", state.getFluidState().isEmpty());
        }
        if (shapeDependsOnNeighbours(state)) {
            o.addProperty("neighbours", true);
        }
        return o;
    }

    /** Name of the state's current value for one property. */
    private static <T extends Comparable<T>> String propertyNameValue(StateHolder<?, ?> state, Property<T> p) {
        return p.getName(state.getValue(p));
    }

    private static JsonArray shapeBoxes(VoxelShape shape) {
        JsonArray boxes = new JsonArray();
        if (shape == null || shape.isEmpty()) return boxes;
        List<AABB> aabbs = shape.toAabbs();
        for (AABB aabb : aabbs) {
            JsonArray box = new JsonArray();
            box.add(round(aabb.minX)); box.add(round(aabb.minY)); box.add(round(aabb.minZ));
            box.add(round(aabb.maxX)); box.add(round(aabb.maxY)); box.add(round(aabb.maxZ));
            boxes.add(box);
        }
        return boxes;
    }

    private static double round(double v) {
        return Math.round(v * 1e6) / 1e6;
    }

    /**
     * Conservative marker: blocks whose getShape/getCollisionShape implementation is known (by
     * class list, from observation of which vanilla blocks change shape with neighbours) to read
     * neighbouring blocks. With an empty level they still return their no-neighbour shape, so the
     * collision/outline arrays stay meaningful; the flag tells consumers they may differ in world.
     */
    private static boolean shapeDependsOnNeighbours(BlockState state) {
        String name = BuiltInRegistries.BLOCK.getKey(state.getBlock()).getPath();
        switch (name) {
            case "wall":
            case "mushroom_stem":
                return true;
            default:
                return name.endsWith("_wall") || name.endsWith("_fence") || name.endsWith("_fence_gate")
                        || name.endsWith("_pane") || name.endsWith("_iron_bars") || name.endsWith("_tripwire_hook")
                        || name.endsWith("_glass_pane") || name.endsWith("_stained_glass_pane")
                        || name.endsWith("_door") || name.endsWith("_trapdoor") || name.endsWith("_gate");
        }
    }

    // ---------------------------------------------------------------- entity_dimensions.json

    static int dumpEntityDimensions(Path outDir, MinecraftServer server) throws IOException {
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("minecraft", "26.3");
        JsonArray types = new JsonArray();
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            JsonObject o = new JsonObject();
            o.addProperty("id", BuiltInRegistries.ENTITY_TYPE.getId(type));
            o.addProperty("type", BuiltInRegistries.ENTITY_TYPE.getKey(type).toString());
            EntityDimensions dims = type.getDimensions();
            o.addProperty("width", dims.width());
            o.addProperty("height", dims.height());
            o.addProperty("eye_height", dims.eyeHeight());
            o.addProperty("fixed", dims.fixed());
            o.addProperty("fire_immune", type.fireImmune());
            types.add(o);
        }
        root.add("types", types);

        // Per-pose player dimensions from a real ServerPlayer instance in the overworld.
        ServerLevel overworld = server.overworld();
        GameProfile profile = new GameProfile(UUID.randomUUID(), "haste_oracle");
        ServerPlayer player = new ServerPlayer(server, overworld, profile,
                net.minecraft.server.level.ClientInformation.createDefault());
        JsonObject playerPoses = new JsonObject();
        for (Pose pose : Pose.values()) {
            EntityDimensions d = player.getDimensions(pose);
            JsonObject p = new JsonObject();
            p.addProperty("width", d.width());
            p.addProperty("height", d.height());
            p.addProperty("eye_height", player.getEyeHeight(pose));
            playerPoses.add(pose.getSerializedName(), p);
        }
        root.add("player", playerPoses);
        writeJson(outDir.resolve("entity_dimensions.json"), root);
        return types.size();
    }

    // ---------------------------------------------------------------- block_properties.json

    static int dumpBlockProperties(Path outDir) throws IOException {
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("minecraft", "26.3");
        JsonObject blocks = new JsonObject();
        for (Block block : BuiltInRegistries.BLOCK) {
            JsonObject o = new JsonObject();
            o.addProperty("friction", block.getFriction());
            o.addProperty("speed_factor", block.getSpeedFactor());
            o.addProperty("jump_factor", block.getJumpFactor());
            // Breaking: hardness (-1 = unbreakable) and whether drops need the correct tool, from
            // the default state; states that differ are listed under "states" by state id.
            BlockState def = block.defaultBlockState();
            float destroy = def.getDestroySpeed(EmptyBlockGetter.INSTANCE, ORIGIN);
            boolean correctTool = def.requiresCorrectToolForDrops();
            o.addProperty("destroy_time", destroy);
            o.addProperty("requires_correct_tool_for_drops", correctTool);
            JsonObject differing = new JsonObject();
            for (BlockState st : block.getStateDefinition().getPossibleStates()) {
                float d = st.getDestroySpeed(EmptyBlockGetter.INSTANCE, ORIGIN);
                boolean c = st.requiresCorrectToolForDrops();
                if (d != destroy || c != correctTool) {
                    JsonObject so = new JsonObject();
                    so.addProperty("destroy_time", d);
                    so.addProperty("requires_correct_tool_for_drops", c);
                    differing.add(Integer.toString(Block.getId(st)), so);
                }
            }
            if (differing.size() > 0) o.add("states", differing);
            blocks.add(BuiltInRegistries.BLOCK.getKey(block).toString(), o);
        }
        root.add("blocks", blocks);
        writeJson(outDir.resolve("block_properties.json"), root);
        return blocks.size();
    }

    private static void writeJson(Path path, JsonObject root) throws IOException {
        Files.write(path, GSON.toJson(root).getBytes(StandardCharsets.UTF_8));
    }
}
