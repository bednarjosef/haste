package haste.gui;

import haste.rt.GuiBatch;
import org.joml.Matrix3x2fStack;

/**
 * The pose stack {@code GuiGraphicsExtractor.pose()} returns. Screens push/translate/pop it directly (joml), so
 * translation is synchronised lazily: before each draw command {@link #sync()} emits a TRANSLATE_PUSH for any
 * translation not yet sent, and {@link #popMatrix()} emits the matching TRANSLATE_POPs. Scale and rotation are
 * not represented in GuiFrame (see NOTES.md).
 */
public final class Pose extends Matrix3x2fStack {
    private final float[] sentX = new float[64], sentY = new float[64];
    private final int[] pushes = new int[64];
    private int depth;

    public Pose() { super(64); }

    /** Resets to identity at the start of a frame. */
    public void reset() {
        clear();
        identity();
        depth = 0;
        sentX[0] = 0; sentY[0] = 0; pushes[0] = 0;
    }

    @Override
    public Matrix3x2fStack pushMatrix() {
        super.pushMatrix();
        if (depth + 1 < pushes.length) {
            depth++;
            sentX[depth] = sentX[depth - 1];
            sentY[depth] = sentY[depth - 1];
            pushes[depth] = 0;
        }
        return this;
    }

    @Override
    public Matrix3x2fStack popMatrix() {
        super.popMatrix();
        if (depth > 0) {
            for (int i = 0; i < pushes[depth]; i++) GuiBatch.op(GuiBatch.TRANSLATE_POP);
            depth--;
        }
        return this;
    }

    /** Emits the translation not yet sent to Rust (call before every draw command). */
    public void sync() {
        float tx = m20(), ty = m21();
        float dx = tx - sentX[depth], dy = ty - sentY[depth];
        if (dx != 0 || dy != 0) {
            GuiBatch.ops(GuiBatch.TRANSLATE_PUSH, Float.floatToRawIntBits(dx), Float.floatToRawIntBits(dy), Float.floatToRawIntBits(0f));
            pushes[depth]++;
            sentX[depth] = tx;
            sentY[depth] = ty;
        }
    }

    /** Pops every translation still open at the end of a frame. */
    public void finish() {
        while (depth > 0) popMatrix();
        for (int i = 0; i < pushes[0]; i++) GuiBatch.op(GuiBatch.TRANSLATE_POP);
        pushes[0] = 0;
    }

    public float tx() { return m20(); }
    public float ty() { return m21(); }
}
