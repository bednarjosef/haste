package net.minecraft.client.gui.screens;

import haste.rt.Native;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.layouts.FrameLayout;
import net.minecraft.client.gui.layouts.LinearLayout;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.TransferState;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.network.chat.CommonComponents;
import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/**
 * Haste's replacement for vanilla's ConnectScreen (see {@code crates/haste-jvm/src/loader.rs},
 * {@code REPLACEMENTS}): vanilla's opens its own netty connection on a Java thread, but in Haste
 * the connection is tier 1, owned by Rust's protocol crate. The replacement keeps vanilla's
 * screen flow — startConnecting is what JoinMultiplayerScreen.join and the direct-connect screen
 * call — shows vanilla's connecting screen, and hands the address to Rust
 * ({@link Native#requestJoin}), which connects and takes over the window.
 *
 * <p>Vanilla's class is denied (see {@code DENY}) because a generated proxy cannot stand in for
 * it: a {@code class} line in proxies.conf regenerates the whole super chain and would shadow the
 * real {@link Screen} every other screen extends.
 */
public class ConnectScreen extends Screen {
    /** The screen to return to on failure or cancel (vanilla's parent). */
    public final Screen parent;
    private final Component connectFailedTitle;

    public ConnectScreen(final Screen parent, final Component connectFailedTitle) {
        super(Component.translatable("connect.title"));
        this.parent = parent;
        this.connectFailedTitle = connectFailedTitle;
    }

    /** Vanilla's entry point (JoinMultiplayerScreen.join): connect to the server in Rust. */
    public static void startConnecting(
        final Screen parent, final Minecraft minecraft, final ServerAddress hostAndPort,
        final ServerData data, final boolean isQuickPlay, final @Nullable TransferState transferState
    ) {
        ConnectScreen screen = new ConnectScreen(parent, CommonComponents.CONNECT_FAILED);
        minecraft.gui.setScreen(screen);
        Native.requestJoin(data.ip);
    }

    @Override
    protected void init() {
        LinearLayout layout = LinearLayout.vertical().spacing(6);
        layout.defaultCellSetting().alignHorizontallyCenter();
        layout.addChild(new StringWidget(Component.translatable("connect.connecting"), this.font));
        layout.addChild(Button.builder(CommonComponents.GUI_CANCEL, button -> this.onCancel())
            .bounds(0, 0, 120, 20).build());
        layout.arrangeElements();
        layout.visitWidgets(this::addRenderableWidget);
        FrameLayout.alignInRectangle(layout, 0, 0, this.width, this.height, 0.5F, 0.25F);
    }

    @Override
    public void onClose() {
        this.onCancel();
    }

    private void onCancel() {
        Native.cancelJoin();
        this.minecraft.gui.setScreen(this.parent);
    }

    @Override
    public void extractRenderState(final GuiGraphicsExtractor graphics, final int mouseX, final int mouseY, final float a) {
        this.extractBackground(graphics, mouseX, mouseY, a);
        super.extractRenderState(graphics, mouseX, mouseY, a);
    }

    @Override
    public boolean mouseClicked(final MouseButtonEvent event, final boolean doubleClick) {
        return super.mouseClicked(event, doubleClick);
    }

    /** The failure title, for Rust to show on the disconnect screen after a failed join. */
    Component connectFailedTitle() {
        return this.connectFailedTitle;
    }
}
