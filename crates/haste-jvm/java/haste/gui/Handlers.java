package haste.gui;

import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.game.*;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.scores.Objective;
import net.minecraft.world.scores.PlayerTeam;
import net.minecraft.world.scores.ScoreAccess;
import net.minecraft.world.scores.ScoreHolder;
import net.minecraft.world.scores.Scoreboard;

/**
 * Handlers of the {@code ClientPacketListener} proxy for the play packets Rust forwards (the ones whose state is
 * tier 3). Each one hands the decoded vanilla packet to the vanilla component that owns that state: the chat
 * pipeline, the HUD's title/boss/tab-list parts, the scoreboard, the player's menus, attributes and effects.
 * Packets about tier-1 state (world, entities, movement) never reach Java.
 */
public final class Handlers {
    private Handlers() {}

    /** The local player's entity id (set by Rust at login; effect/attribute/entity-data packets are filtered on it). */
    static int localEntityId = Integer.MIN_VALUE;
    /** The client-side scoreboard (vanilla's plain data structure, read by the HUD's sidebar and the tab list). */
    static final Scoreboard SCOREBOARD = new Scoreboard();
    /** The tab list, keyed by profile id, in arrival order. */
    static final Map<UUID, PlayerInfo> PLAYERS = new LinkedHashMap<>();
    /** Players shown in the tab list ("listed"). */
    static final Map<UUID, PlayerInfo> LISTED = new LinkedHashMap<>();
    /** The server's command tree (Brigadier), rebuilt from each commands packet. */
    static com.mojang.brigadier.CommandDispatcher<net.minecraft.client.multiplayer.ClientSuggestionProvider> commands =
        new com.mojang.brigadier.CommandDispatcher<>();

    // ---- chat ----
    public static void handleSystemChat(ClientPacketListener c, ClientboundSystemChatPacket p) {
        Proxies.chatListener.handleSystemMessage(p.content(), p.overlay());
    }

    /**
     * Signed player chat. Approximate: the message is shown through vanilla's disguised-chat path (decorated with
     * the bound chat type, e.g. "&lt;name&gt; text"), without signature verification or the secure/not-secure
     * indicator; unsigned replacement content is preferred when the server sends one, filtered text is not masked.
     */
    public static void handlePlayerChat(ClientPacketListener c, ClientboundPlayerChatPacket p) {
        Component content = p.unsignedContent().orElseGet(() -> Component.literal(p.body().content()));
        Proxies.chatListener.handleDisguisedChatMessage(content, p.chatType());
    }

    public static void handleDisguisedChat(ClientPacketListener c, ClientboundDisguisedChatPacket p) {
        Proxies.chatListener.handleDisguisedChatMessage(p.message(), p.chatType());
    }

    /** Deleting signed messages needs the signature cache of verified chat, which Haste does not keep yet: ignored. */
    public static void handleDeleteChat(ClientPacketListener c, ClientboundDeleteChatPacket p) {}

    public static void handleCustomChatCompletions(ClientPacketListener c, ClientboundCustomChatCompletionsPacket p) {
        ConnectionImpl.getSuggestionsProvider(c).modifyCustomCompletions(p.action(), p.entries());
    }

    // ---- titles, action bar, boss bars, tab list header ----
    public static void setTitleText(ClientPacketListener c, ClientboundSetTitleTextPacket p) { Proxies.hud.setTitle(p.text()); }
    public static void setSubtitleText(ClientPacketListener c, ClientboundSetSubtitleTextPacket p) { Proxies.hud.setSubtitle(p.text()); }
    public static void setTitlesAnimation(ClientPacketListener c, ClientboundSetTitlesAnimationPacket p) {
        Proxies.hud.setTimes(p.getFadeIn(), p.getStay(), p.getFadeOut());
    }
    public static void setActionBarText(ClientPacketListener c, ClientboundSetActionBarTextPacket p) {
        Proxies.hud.setOverlayMessage(p.text(), false);
    }
    public static void handleTitlesClear(ClientPacketListener c, ClientboundClearTitlesPacket p) {
        Proxies.hud.clearTitles();
        if (p.shouldResetTimes()) Proxies.hud.resetTitleTimes();
    }
    public static void handleBossUpdate(ClientPacketListener c, ClientboundBossEventPacket p) {
        Proxies.hud.getBossOverlay().update(p);
    }
    /** An empty header or footer is no header or footer. */
    public static void handleTabListCustomisation(ClientPacketListener c, ClientboundTabListPacket p) {
        Proxies.hud.getTabList().setHeader(p.header().getString().isEmpty() ? null : p.header());
        Proxies.hud.getTabList().setFooter(p.footer().getString().isEmpty() ? null : p.footer());
    }

    // ---- tab list players ----
    private static MethodHandle playerInfoSetter(String name, Class<?> type) {
        try {
            return MethodHandles.privateLookupIn(PlayerInfo.class, MethodHandles.lookup())
                .findVirtual(PlayerInfo.class, name, MethodType.methodType(void.class, type));
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }
    private static final MethodHandle SET_GAME_MODE = playerInfoSetter("setGameMode", net.minecraft.world.level.GameType.class);
    private static final MethodHandle SET_LATENCY = playerInfoSetter("setLatency", int.class);

    public static void handlePlayerInfoUpdate(ClientPacketListener c, ClientboundPlayerInfoUpdatePacket p) throws Throwable {
        for (ClientboundPlayerInfoUpdatePacket.Entry e : p.newEntries()) {
            PLAYERS.putIfAbsent(e.profileId(), new PlayerInfo(e.profile(), false));
        }
        for (ClientboundPlayerInfoUpdatePacket.Entry e : p.entries()) {
            PlayerInfo info = PLAYERS.get(e.profileId());
            if (info == null) continue;
            for (ClientboundPlayerInfoUpdatePacket.Action a : p.actions()) {
                switch (a) {
                    case UPDATE_GAME_MODE -> SET_GAME_MODE.invoke(info, e.gameMode());
                    case UPDATE_LATENCY -> SET_LATENCY.invoke(info, e.latency());
                    case UPDATE_DISPLAY_NAME -> info.setTabListDisplayName(e.displayName());
                    case UPDATE_HAT -> info.setShowHat(e.showHat());
                    case UPDATE_LIST_ORDER -> info.setTabListOrder(e.listOrder());
                    case UPDATE_LISTED -> { if (e.listed()) LISTED.put(e.profileId(), info); else LISTED.remove(e.profileId()); }
                    default -> {} // ADD_PLAYER handled above; INITIALIZE_CHAT: no chat verification yet
                }
            }
        }
    }

    public static void handlePlayerInfoRemove(ClientPacketListener c, ClientboundPlayerInfoRemovePacket p) {
        for (UUID id : p.profileIds()) { PLAYERS.remove(id); LISTED.remove(id); }
    }

    public static Collection<PlayerInfo> getOnlinePlayers(ClientPacketListener c) { return PLAYERS.values(); }
    public static Collection<PlayerInfo> getListedOnlinePlayers(ClientPacketListener c) { return LISTED.values(); }
    public static java.util.Collection<UUID> getOnlinePlayerIds(ClientPacketListener c) { return PLAYERS.keySet(); }
    public static PlayerInfo getPlayerInfo(ClientPacketListener c, UUID id) { return PLAYERS.get(id); }
    public static PlayerInfo getPlayerInfo(ClientPacketListener c, String name) {
        for (PlayerInfo i : PLAYERS.values()) if (i.getProfile().name().equals(name)) return i;
        return null;
    }

    // ---- scoreboard ----
    private static final int ADD = 0, REMOVE = 1, CHANGE = 2;

    public static void handleAddObjective(ClientPacketListener c, ClientboundSetObjectivePacket p) {
        String name = p.getObjectiveName();
        Objective o = SCOREBOARD.getObjective(name);
        switch (p.getMethod()) {
            case ADD -> {
                if (o == null) {
                    SCOREBOARD.addObjective(name, net.minecraft.world.scores.criteria.ObjectiveCriteria.DUMMY, p.getDisplayName(),
                        p.getRenderType(), false, p.getNumberFormat().orElse(null));
                }
            }
            case REMOVE -> { if (o != null) SCOREBOARD.removeObjective(o); }
            case CHANGE -> {
                if (o != null) {
                    o.setDisplayName(p.getDisplayName());
                    o.setRenderType(p.getRenderType());
                    o.setNumberFormat(p.getNumberFormat().orElse(null));
                }
            }
            default -> {}
        }
    }

    public static void handleSetScore(ClientPacketListener c, ClientboundSetScorePacket p) {
        Objective o = SCOREBOARD.getObjective(p.objectiveName());
        if (o == null) return;
        ScoreAccess s = SCOREBOARD.getOrCreatePlayerScore(ScoreHolder.forNameOnly(p.owner()), o, true);
        s.set(p.score());
        s.display(p.display().orElse(null));
        s.numberFormatOverride(p.numberFormat().orElse(null));
    }

    public static void handleResetScore(ClientPacketListener c, ClientboundResetScorePacket p) {
        ScoreHolder h = ScoreHolder.forNameOnly(p.owner());
        if (p.objectiveName() == null) {
            SCOREBOARD.resetAllPlayerScores(h);
        } else {
            Objective o = SCOREBOARD.getObjective(p.objectiveName());
            if (o != null) SCOREBOARD.resetSinglePlayerScore(h, o);
        }
    }

    public static void handleSetDisplayObjective(ClientPacketListener c, ClientboundSetDisplayObjectivePacket p) {
        String name = p.getObjectiveName();
        SCOREBOARD.setDisplayObjective(p.getSlot(), name == null ? null : SCOREBOARD.getObjective(name));
    }

    public static void handleSetPlayerTeamPacket(ClientPacketListener c, ClientboundSetPlayerTeamPacket p) {
        PlayerTeam team;
        if (p.getTeamAction() == ClientboundSetPlayerTeamPacket.Action.ADD) {
            team = SCOREBOARD.getPlayerTeam(p.getName());
            if (team == null) team = SCOREBOARD.addPlayerTeam(p.getName());
        } else {
            team = SCOREBOARD.getPlayerTeam(p.getName());
            if (team == null) return;
        }
        PlayerTeam t = team;
        p.getParameters().ifPresent(par -> {
            t.setDisplayName(par.displayName());
            t.setColor(par.color());
            t.unpackOptions(par.options());
            t.setNameTagVisibility(par.nameTagVisibility());
            t.setCollisionRule(par.collisionRule());
            t.setPlayerPrefix(par.playerPrefix());
            t.setPlayerSuffix(par.playerSuffix());
        });
        ClientboundSetPlayerTeamPacket.Action players = p.getPlayerAction();
        if (players == ClientboundSetPlayerTeamPacket.Action.ADD) {
            for (String n : p.getPlayers()) SCOREBOARD.addPlayerToTeam(n, t);
        } else if (players == ClientboundSetPlayerTeamPacket.Action.REMOVE) {
            for (String n : p.getPlayers()) SCOREBOARD.removePlayerFromTeam(n, t);
        }
        if (p.getTeamAction() == ClientboundSetPlayerTeamPacket.Action.REMOVE) SCOREBOARD.removePlayerTeam(t);
    }

    // ---- the local player's effects, attributes and entity data ----
    public static void handleUpdateMobEffect(ClientPacketListener c, ClientboundUpdateMobEffectPacket p) {
        if (p.getEntityId() != localEntityId) return;
        var inst = new net.minecraft.world.effect.MobEffectInstance(p.getEffect(), p.getEffectDurationTicks(), p.getEffectAmplifier(),
            p.isEffectAmbient(), p.isEffectVisible(), p.effectShowsIcon(), null);
        if (!p.shouldBlend()) inst.skipBlending();
        Proxies.player.getActiveEffectsMap().put(p.getEffect(), inst);
    }

    public static void handleRemoveMobEffect(ClientPacketListener c, ClientboundRemoveMobEffectPacket p) {
        if (p.entityId() != localEntityId) return;
        Proxies.player.getActiveEffectsMap().remove(p.effect());
    }

    public static void handleUpdateAttributes(ClientPacketListener c, ClientboundUpdateAttributesPacket p) {
        if (p.getEntityId() != localEntityId) return;
        for (ClientboundUpdateAttributesPacket.AttributeSnapshot s : p.getValues()) {
            AttributeInstance inst = Proxies.attributes.getInstance(s.attribute());
            if (inst == null) continue;
            inst.setBaseValue(s.base());
            inst.removeModifiers();
            for (var m : s.modifiers()) inst.addTransientModifier(m);
        }
    }

    public static void handleSetEntityData(ClientPacketListener c, ClientboundSetEntityDataPacket p) {
        if (p.id() != localEntityId) return;
        Proxies.player.getEntityData().assignValues(p.packedItems());
    }

    // ---- advancements ----

    /** The server's advancement tree and progress, like ClientPacketListener.handleUpdateAdvancementsPacket. */
    public static void handleUpdateAdvancementsPacket(ClientPacketListener c, ClientboundUpdateAdvancementsPacket p) {
        Proxies.advancements.update(p);
    }

    /** The server picked the tab to show (login and world change). */
    public static void handleSelectAdvancementsTab(ClientPacketListener c, ClientboundSelectAdvancementsTabPacket p) {
        Proxies.advancements.setSelectedTab(p.getTab() == null ? null : Proxies.advancements.get(p.getTab()), false);
    }

    // ---- commands ----
    public static void handleCommands(ClientPacketListener c, ClientboundCommandsPacket p) {
        var ctx = net.minecraft.commands.CommandBuildContext.simple(Registries.access(), LevelImpl.enabledFeatures(Proxies.level));
        commands = new com.mojang.brigadier.CommandDispatcher<>(p.getRoot(ctx, CommandNodes.BUILDER));
    }

    public static void handleCommandSuggestions(ClientPacketListener c, ClientboundCommandSuggestionsPacket p) {
        ConnectionImpl.getSuggestionsProvider(c).completeCustomSuggestions(p.id(), p.toSuggestions());
    }

    // ---- containers ----
    public static void handleOpenScreen(ClientPacketListener c, ClientboundOpenScreenPacket p) {
        net.minecraft.client.gui.screens.MenuScreens.create(p.getType(), Proxies.minecraft, p.getContainerId(), p.getTitle());
    }

    private static AbstractContainerMenu menu(int id) {
        if (id == 0) return Proxies.player.inventoryMenu;
        AbstractContainerMenu m = Proxies.player.containerMenu;
        return m != null && m.containerId == id ? m : null;
    }

    public static void handleContainerContent(ClientPacketListener c, ClientboundContainerSetContentPacket p) {
        AbstractContainerMenu m = menu(p.containerId());
        if (m != null) m.initializeContents(p.stateId(), p.items(), p.carriedItem());
    }

    public static void handleContainerSetSlot(ClientPacketListener c, ClientboundContainerSetSlotPacket p) {
        AbstractContainerMenu m = menu(p.getContainerId());
        if (m != null) m.setItem(p.getSlot(), p.getStateId(), p.getItem());
    }

    public static void handleSetCursorItem(ClientPacketListener c, ClientboundSetCursorItemPacket p) {
        AbstractContainerMenu m = Proxies.player.containerMenu;
        if (m != null) m.setCarried(p.contents());
    }

    public static void handleSetPlayerInventory(ClientPacketListener c, ClientboundSetPlayerInventoryPacket p) {
        Proxies.inventory.setItem(p.slot(), p.contents());
    }

    public static void handleContainerSetData(ClientPacketListener c, ClientboundContainerSetDataPacket p) {
        AbstractContainerMenu m = menu(p.getContainerId());
        if (m != null) m.setData(p.getId(), p.getValue());
    }

    /** The server closed the container: back to the player's own menu, screen closed. */
    public static void handleContainerClose(ClientPacketListener c, ClientboundContainerClosePacket p) {
        Proxies.player.clientSideCloseContainer();
    }

    public static void handleMerchantOffers(ClientPacketListener c, ClientboundMerchantOffersPacket p) {
        AbstractContainerMenu m = menu(p.getContainerId());
        if (m instanceof net.minecraft.world.inventory.MerchantMenu mm) {
            mm.setOffers(p.getOffers());
            mm.setXp(p.getVillagerXp());
            mm.setMerchantLevel(p.getVillagerLevel());
            mm.setShowProgressBar(p.showProgress());
            mm.setCanRestock(p.canRestock());
        }
    }

    // ---- death ----
    public static void handlePlayerCombatKill(ClientPacketListener c, ClientboundPlayerCombatKillPacket p) {
        if (p.playerId() != localEntityId || Mirror.flag(Mirror.F_IMMEDIATE_RESPAWN)) return;
        Proxies.gui.setScreen(new net.minecraft.client.gui.screens.DeathScreen(p.message(), Mirror.flag(Mirror.F_HARDCORE), Proxies.player));
    }
}
