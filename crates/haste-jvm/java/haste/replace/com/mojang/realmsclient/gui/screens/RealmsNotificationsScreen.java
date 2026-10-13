package com.mojang.realmsclient.gui.screens;

import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.network.chat.Component;

/**
 * Haste's replacement for vanilla's Realms notification bell of the title screen (see
 * {@code crates/haste-jvm/src/loader.rs}, {@code REPLACEMENTS}): vanilla's class builds a realms client
 * to learn whether the player has notifications, and Haste has no Realms at all. Haste's class is served
 * under the vanilla name, so the title screen's code path stays untouched - and everything it calls here
 * is a no-op, so the bell never appears.
 */
public final class RealmsNotificationsScreen extends Screen {
    public RealmsNotificationsScreen() {
        super(Component.empty());
    }

    @Override
    protected void init() {}

    @Override
    public void added() {}

    @Override
    public void removed() {}

    @Override
    public void tick() {}

    @Override
    public void extractRenderState(final GuiGraphicsExtractor graphics, final int x, final int y, final float a) {}

    @Override
    public boolean mouseClicked(final MouseButtonEvent event, final boolean doubleClick) { return false; }
}
