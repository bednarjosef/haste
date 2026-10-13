package haste.recorder.client;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.Set;
import net.minecraft.client.Minecraft;
import net.minecraft.client.model.geom.ModelLayerLocation;
import net.minecraft.client.model.geom.builders.CubeDefinition;
import net.minecraft.client.model.geom.builders.CubeDeformation;
import net.minecraft.client.model.geom.builders.CubeListBuilder;
import net.minecraft.client.model.geom.builders.LayerDefinition;
import net.minecraft.client.model.geom.builders.PartDefinition;
import net.minecraft.client.model.geom.builders.UVPair;
import net.minecraft.core.Direction;
import net.minecraft.resources.Identifier;

/**
 * Dumps the geometry of every model layer the client has baked, to
 * {@code cache/oracle/26.3/entity_models.json} (FORMATS.md).
 *
 * The client's {@link net.minecraft.client.model.geom.EntityModelSet} holds the registered
 * {@link LayerDefinition}s (one per {@link ModelLayerLocation}); walking their builder tree gives
 * us everything a renderer needs: texture size, per-part pivots/rotations and per-cube origins,
 * sizes, UV offsets, mirror and inflate. The definitions themselves are kept private by the
 * EntityModelSet, so this reads its {@code roots} map reflectively (signatures checked with
 * javap; no decompilation) — a plain observer's read, nothing is modified.
 */
public final class ModelDumps {
    private ModelDumps() {}

    /** Writes the dump; returns the number of layers written, or -1 on failure. */
    static int dumpModels() throws IOException, ReflectiveOperationException {
        Minecraft mc = Minecraft.getInstance();
        Map<ModelLayerLocation, LayerDefinition> roots = modelSetRoots(mc);
        JsonArray layers = new JsonArray();
        // Sorted by layer id so two runs of the same version diff cleanly.
        java.util.List<Map.Entry<ModelLayerLocation, LayerDefinition>> sorted =
            new java.util.ArrayList<>(roots.entrySet());
        java.util.Comparator<Map.Entry<ModelLayerLocation, LayerDefinition>> byId =
            java.util.Comparator.comparing((Map.Entry<ModelLayerLocation, LayerDefinition> e) ->
                e.getKey().toString());
        sorted.sort(byId);
        for (Map.Entry<ModelLayerLocation, LayerDefinition> e : sorted) {
            layers.add(layerJson(e.getKey(), e.getValue()));
        }
        JsonObject root = new JsonObject();
        root.addProperty("version", 1);
        root.addProperty("minecraft", "26.3");
        root.addProperty("source", "client EntityModelSet (baked model layers)");
        root.add("layers", layers);
        Path out = RecorderIO.oracleDir().resolve("entity_models.json");
        Files.createDirectories(out.getParent());
        Files.write(out, RecorderIO.GSON.toJson(root).getBytes(StandardCharsets.UTF_8));
        return layers.size();
    }

    /** EntityModelSet keeps its LayerDefinitions in a private `roots` map; read it reflectively. */
    @SuppressWarnings("unchecked")
    private static Map<ModelLayerLocation, LayerDefinition> modelSetRoots(Minecraft mc)
            throws ReflectiveOperationException {
        Object modelSet = mc.getEntityModels();
        Field rootsField = modelSet.getClass().getDeclaredField("roots");
        rootsField.setAccessible(true);
        return (Map<ModelLayerLocation, LayerDefinition>) rootsField.get(modelSet);
    }

    private static JsonObject layerJson(ModelLayerLocation location, LayerDefinition layer)
            throws ReflectiveOperationException {
        JsonObject o = new JsonObject();
        o.addProperty("layer", location.model().getNamespace() + ":" + location.model().getPath()
            + "#" + location.layer());
        int[] texSize = layerTexSize(layer);
        o.addProperty("texture_width", texSize[0]);
        o.addProperty("texture_height", texSize[1]);
        PartDefinition rootPart = layerRoot(layer);
        // The root part has no name and no cubes of its own; only its children exist.
        o.add("parts", partsJson(rootPart));
        return o;
    }

    /** LayerDefinition -> MeshDefinition -> root PartDefinition, and MaterialDefinition -> tex size. */
    private static PartDefinition layerRoot(LayerDefinition layer) throws ReflectiveOperationException {
        Field meshField = LayerDefinition.class.getDeclaredField("mesh");
        meshField.setAccessible(true);
        Object mesh = meshField.get(layer);
        Field rootField = mesh.getClass().getDeclaredField("root");
        rootField.setAccessible(true);
        return (PartDefinition) rootField.get(mesh);
    }

    private static int[] layerTexSize(LayerDefinition layer) throws ReflectiveOperationException {
        Field materialField = LayerDefinition.class.getDeclaredField("material");
        materialField.setAccessible(true);
        Object material = materialField.get(layer);
        Field xField = material.getClass().getDeclaredField("xTexSize");
        Field yField = material.getClass().getDeclaredField("yTexSize");
        xField.setAccessible(true);
        yField.setAccessible(true);
        return new int[] {xField.getInt(material), yField.getInt(material)};
    }

    private static JsonArray partsJson(PartDefinition part) throws ReflectiveOperationException {
        JsonArray a = new JsonArray();
        for (Map.Entry<String, PartDefinition> child : part.getChildren()) {
            a.add(partJson(child.getKey(), child.getValue()));
        }
        return a;
    }

    private static JsonObject partJson(String name, PartDefinition part)
            throws ReflectiveOperationException {
        JsonObject o = new JsonObject();
        o.addProperty("name", name);
        Object pose = partPose(part);
        o.add("pivot", floats(poser(pose, "x"), poser(pose, "y"), poser(pose, "z")));
        o.add("rotation", floats(poser(pose, "xRot"), poser(pose, "yRot"), poser(pose, "zRot")));
        o.add("scale", floats(poser(pose, "xScale"), poser(pose, "yScale"), poser(pose, "zScale")));
        Field cubesField = PartDefinition.class.getDeclaredField("cubes");
        cubesField.setAccessible(true);
        @SuppressWarnings("unchecked")
        java.util.List<Object> cubes = (java.util.List<Object>) cubesField.get(part);
        JsonArray cubeArray = new JsonArray();
        for (Object cube : cubes) cubeArray.add(cubeJson(cube));
        o.add("cubes", cubeArray);
        o.add("children", partsJson(part));
        return o;
    }

    private static Object partPose(PartDefinition part) throws ReflectiveOperationException {
        Field poseField = PartDefinition.class.getDeclaredField("partPose");
        poseField.setAccessible(true);
        return poseField.get(part);
    }

    /** PartPose is a record with x/y/z, xRot/yRot/zRot, xScale/yScale/zScale accessors. */
    private static float poser(Object pose, String accessor) throws ReflectiveOperationException {
        Method m = pose.getClass().getMethod(accessor);
        return (Float) m.invoke(pose);
    }

    /** One cube of a part: origin, size, UV offset, mirror, inflate and hidden faces. */
    private static JsonObject cubeJson(Object cube) throws ReflectiveOperationException {
        JsonObject o = new JsonObject();
        Object origin = field(cube, "origin");
        Object dimensions = field(cube, "dimensions");
        UVPair texCoord = (UVPair) field(cube, "texCoord");
        o.add("origin", floats(vecX(origin), vecY(origin), vecZ(origin)));
        o.add("size", floats(vecX(dimensions), vecY(dimensions), vecZ(dimensions)));
        o.addProperty("uv", (int) texCoord.u() + " " + (int) texCoord.v());
        o.addProperty("u", texCoord.u());
        o.addProperty("v", texCoord.v());
        Object mirror = field(cube, "mirror");
        o.addProperty("mirror", (Boolean) mirror);
        Object grow = field(cube, "grow");
        float gx = (Float) field(grow, "growX"), gy = (Float) field(grow, "growY"),
              gz = (Float) field(grow, "growZ");
        o.add("inflate", floats(gx, gy, gz));
        @SuppressWarnings("unchecked")
        Set<Direction> faces = (Set<Direction>) field(cube, "visibleFaces");
        JsonArray faceArray = new JsonArray();
        for (Direction d : faces) faceArray.add(d.getName());
        o.add("visible_faces", faceArray);
        Object comment = field(cube, "comment");
        if (comment != null) o.addProperty("comment", (String) comment);
        return o;
    }

    private static Object field(Object target, String name) throws ReflectiveOperationException {
        Field f = target.getClass().getDeclaredField(name);
        f.setAccessible(true);
        return f.get(target);
    }

    // org.joml.Vector3fc reads (origin/dimensions), via its x()/y()/z() accessors.
    private static float vecX(Object v) throws ReflectiveOperationException {
        return (Float) v.getClass().getMethod("x").invoke(v);
    }

    private static float vecY(Object v) throws ReflectiveOperationException {
        return (Float) v.getClass().getMethod("y").invoke(v);
    }

    private static float vecZ(Object v) throws ReflectiveOperationException {
        return (Float) v.getClass().getMethod("z").invoke(v);
    }

    private static JsonArray floats(float... values) {
        JsonArray a = new JsonArray();
        for (float v : values) a.add(v);
        return a;
    }
}
