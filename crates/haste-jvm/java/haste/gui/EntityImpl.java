package haste.gui;

import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.world.entity.Entity;

/**
 * Bodies of the {@code EntityRenderDispatcher} proxy. Entity rendering is Rust's (renderer/entities); screens that draw an
 * entity (the inventory's player model) get an empty vanilla render state, and the extractor records the draw
 * request (no GuiFrame command exists for it yet: interface request).
 */
public final class EntityImpl {
    private EntityImpl() {}

    /** One canonical renderer proxy for every entity: it only hands out empty render states. */
    public static net.minecraft.client.renderer.entity.EntityRenderer getRenderer(EntityRenderDispatcher d, Entity e) {
        return Proxies.service(net.minecraft.client.renderer.entity.player.AvatarRenderer.class);
    }

    public static EntityRenderState createRenderState(net.minecraft.client.renderer.entity.EntityRenderer r, Entity e, float partial) {
        return new EntityRenderState();
    }

    public static EntityRenderState extractEntity(EntityRenderDispatcher d, Entity e, float partial) {
        return new EntityRenderState();
    }
}
