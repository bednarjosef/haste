package haste.gui;

import net.minecraft.client.renderer.texture.TextureManager;

/**
 * Bodies of the {@code TextureManager} proxy: the server list's favicons are the only caller, and
 * their pixels stay in Rust (Haste's renderer owns every GPU texture). Registration is recorded
 * per location so {@code getTexture} answers the questions vanilla's favicon code asks (whether an
 * icon is registered); nothing is ever uploaded.
 */
public final class TextureImpl {
    private TextureImpl() {}

    private static final java.util.Set<net.minecraft.resources.Identifier> REGISTERED = java.util.Collections.synchronizedSet(
        java.util.Collections.newSetFromMap(new java.util.HashMap<>()));

    /** The one texture manager (vanilla: one per client, Minecraft.getTextureManager). */
    private static final TextureManager TEXTURES = Proxies.make(TextureManager.class);

    /** The one texture manager (ClientImpl.getTextureManager hands it out). */
    static TextureManager textures() {
        return TEXTURES;
    }

    /** FaviconTexture.register (a dynamic texture): remember the location only. */
    public static void register(TextureManager t, net.minecraft.resources.Identifier location,
                                net.minecraft.client.renderer.texture.AbstractTexture texture) {
        REGISTERED.add(location);
    }

    /** FaviconTexture.clear (release): forget it. */
    public static void release(TextureManager t, net.minecraft.resources.Identifier location) {
        REGISTERED.remove(location);
    }

    /** A missing texture (vanilla's placeholder); registered icons never need uploading here. */
    public static net.minecraft.client.renderer.texture.AbstractTexture getTexture(TextureManager t,
        net.minecraft.resources.Identifier location) {
        return Proxies.make(net.minecraft.client.renderer.texture.AbstractTexture.class);
    }
}
