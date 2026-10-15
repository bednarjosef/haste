package net.minecraft.client.model.object.book;

import net.minecraft.client.model.geom.ModelPart;

/**
 * Haste's replacement for vanilla's BookModel (see {@code crates/haste-jvm/src/loader.rs},
 * {@code REPLACEMENTS}): vanilla's extends {@code Model}, whose constructor binds a
 * {@code RenderTypes::entitySolid} method reference — the renderer's own pipeline set, which is
 * deny-listed here. The enchanting table's screen ({@code EnchantmentScreen.init}) builds one
 * from the baked book layer and hands it to {@code GuiGraphicsExtractor.book}, which is a no-op
 * in Haste (the GUI batch draws no models yet), so the replacement keeps exactly that shape: a
 * model that exists and can be constructed, never rendered.
 */
public class BookModel {
    /** Vanilla keeps the baked root; nothing reads it here, but a mod's mixin might. */
    public final ModelPart root;

    public BookModel(final ModelPart root) {
        this.root = root;
    }
}
