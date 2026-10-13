package haste.gui;

import com.mojang.renderpearl.api.GpuFormat;
import com.mojang.renderpearl.api.pipeline.BlendFactor;
import com.mojang.renderpearl.api.pipeline.BlendFunction;
import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import haste.rt.GuiBlend;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import net.minecraft.client.renderer.RenderPipelines;

/**
 * Haste's tier-2 {@code RenderPipelines}: the static fields vanilla's GUI passes to a draw call. Vanilla's own
 * class never runs here (its {@code <clinit>} would build the whole renderer's pipeline set and ask the GPU
 * backend for it), so the generated proxy starts with null fields and this class fills in the ones the GUI
 * uses, each carrying vanilla's location and blend function (see {@code RenderPipelines.java} in 26.3's
 * source, where they are registered). {@link GfxImpl#blend} reads the blend function from there, which is what
 * vanilla itself does; the identifier rule is only the fallback for a pipeline that is still null.
 *
 * <p>A mod that registers its own pipeline with one of these blend functions gets the same mapping, because
 * the mapping is on the function, not on the name: {@code RenderPipeline} and its value types are real
 * vanilla classes served from the player's jar, and Haste builds nothing of its own into them.
 */
public final class Pipelines {
    private Pipelines() {}

    /** Vanilla's vignette blend (RenderPipelines.VIGNETTE): dst * (1 - src colour): a darkening multiply. */
    private static final BlendFunction MULTIPLY = new BlendFunction(
        BlendFactor.ZERO, BlendFactor.ONE_MINUS_SRC_COLOR, BlendFactor.ZERO, BlendFactor.ONE);
    /** Vanilla's crosshair blend (BlendFunction.INVERT): the source inverts the colour behind it. */
    private static final BlendFunction INVERT = BlendFunction.INVERT;
    /** Vanilla's text selection highlight blend (RenderPipelines.GUI_TEXT_HIGHLIGHT, BlendFunction.ADDITIVE). */
    private static final BlendFunction ADDITIVE = BlendFunction.ADDITIVE;

    /** Pipelines whose blend function Haste has no GUI mode for: each is logged once, drawn normally. */
    private static final Set<RenderPipeline> UNMAPPED = ConcurrentHashMap.newKeySet();

    /** Fills the GUI's pipeline fields in the {@code RenderPipelines} proxy (idempotent). */
    public static void install() {
        set("GUI", pipeline("pipeline/gui", BlendFunction.TRANSLUCENT));
        set("GUI_INVERT", pipeline("pipeline/gui_invert", INVERT));
        set("GUI_TEXT_HIGHLIGHT", pipeline("pipeline/gui_text_highlight", BlendFunction.ADDITIVE));
        set("GUI_TEXT", pipeline("pipeline/gui_text", BlendFunction.TRANSLUCENT, "core/text"));
        set("GUI_TEXT_GRAYSCALE", pipeline("pipeline/gui_text_grayscale", BlendFunction.TRANSLUCENT, "core/text"));
        set("GUI_TEXTURED", pipeline("pipeline/gui_textured", BlendFunction.TRANSLUCENT, "core/position_tex_color"));
        set("GUI_TEXTURED_PREMULTIPLIED_ALPHA",
            pipeline("pipeline/gui_textured_premultiplied_alpha",
                BlendFunction.TRANSLUCENT_PREMULTIPLIED_ALPHA, "core/position_tex_color"));
        set("GUI_OPAQUE_TEXTURED_BACKGROUND", pipeline("pipeline/gui_opaque_textured_background",
            new ColorTargetState(Optional.empty(), GpuFormat.RGBA8_UNORM, 15), "core/position_tex_color"));
        set("GUI_NAUSEA_OVERLAY",
            pipeline("pipeline/gui_nausea_overlay", BlendFunction.ADDITIVE, "core/position_tex_color"));
        set("VIGNETTE", pipeline("pipeline/vignette", new ColorTargetState(MULTIPLY), "core/position_tex_color"));
        set("CROSSHAIR", pipeline("pipeline/crosshair", new ColorTargetState(INVERT), "core/position_tex_color"));
        set("MOJANG_LOGO", pipeline("pipeline/mojang_logo",
            new BlendFunction(BlendFactor.SRC_ALPHA, BlendFactor.ONE), "core/position_tex_color"));
    }

    /**
     * The {@link GuiBlend} code of a blit drawn with {@code p}: the vignette multiplies, the crosshair inverts,
     * the text selection highlight adds, every other GUI blend (translucent, premultiplied, none) is normal.
     * Codes: 0 normal, 1 multiply, 2 invert, 3 additive.
     */
    public static int blendOf(RenderPipeline p) {
        for (ColorTargetState target : p.getColorTargetStates()) {
            if (target == null) continue;
            Optional<BlendFunction> blend = target.blendFunction();
            if (blend.isEmpty()) continue;
            BlendFunction f = blend.get();
            if (f.equals(MULTIPLY)) return GuiBlend.MULTIPLY;
            if (f.equals(INVERT)) return GuiBlend.INVERT;
            if (f.equals(ADDITIVE)) return GuiBlend.ADDITIVE;
            if (UNMAPPED.add(p)) {
                System.err.println("[haste] GUI pipeline " + p.getLocation() + " blends " + f
                    + ": Haste has no such mode, drawing it normally");
            }
        }
        return GuiBlend.NORMAL;
    }

    /** A pipeline on the shaders of the same-shaped vanilla one: only its data matters to Haste, never a GPU. */
    private static RenderPipeline pipeline(String location, ColorTargetState target, String shader) {
        return RenderPipeline.builder()
            .withLocation(location)
            .withVertexShader(shader)
            .withFragmentShader(shader)
            .withColorTargetState(target)
            .withPrimitiveTopology(PrimitiveTopology.QUADS)
            .build();
    }

    private static RenderPipeline pipeline(String location, BlendFunction blend) {
        return pipeline(location, new ColorTargetState(blend), "core/gui");
    }

    private static RenderPipeline pipeline(String location, BlendFunction blend, String shader) {
        return pipeline(location, new ColorTargetState(blend), shader);
    }

    /**
     * Installs one pipeline into the proxy's static field. The generator drops the fields' {@code final}
     * (they start null without a {@code <clinit>}), so a var handle is enough; there is no second {@code
     * RenderPipelines} class for a mod to see.
     */
    private static void set(String field, RenderPipeline p) {
        Proxies.setStatic(RenderPipelines.class, field, RenderPipeline.class, p);
    }
}
