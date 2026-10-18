//! The screen management of [`Ui`] (jvm-host): opening and closing the real vanilla screens,
//! the menu screens' requests (join, quit, world list, options, resource packs), their option
//! and inventory edits, and the disconnect. See the module documentation of [`crate::ui`].

use super::*;

impl Ui {
    /// Records the world's block state at a position (Rust's `BlockStateId`), for the block
    /// entities the screens edit (see [`Ui::handle`]); the game thread calls it while it can
    /// still read the world.
    pub fn record_block_state(&self, pos: BlockPos, state: BlockStateId) {
        self.gui().set_block_state(pos, state.0 as i32);
    }

    /// Opens a real vanilla screen (menu mode's title screen, tests).
    pub fn open_screen(&self, kind: ScreenKind) -> anyhow::Result<()> {
        self.gui().open_screen(kind)
    }

    /// The multiplayer screen's Join: the address to connect to, if any was clicked since the
    /// last call (menu mode; the connection itself is Rust's).
    pub fn take_join_request(&self) -> Option<String> {
        self.gui().take_join_request()
    }

    /// The connecting screen's Cancel since the last call: drop a join still in progress.
    pub fn take_join_cancelled(&self) -> bool {
        self.gui().take_join_cancelled()
    }

    /// The title screen's Quit Game since the last call: close the client.
    pub fn take_quit_request(&self) -> bool {
        self.gui().take_quit_request()
    }

    /// The world list's "Play Selected World" since the last call: the world folder to start
    /// the local server for and join.
    pub fn take_open_world_request(&self) -> Option<String> {
        self.gui().take_open_world_request()
    }

    /// The create screen's "Create New World" since the last call: the world to make.
    pub fn take_create_world_request(&self) -> Option<haste_jvm::CreateWorldRequest> {
        self.gui().take_create_world_request()
    }

    /// One 20 Hz client tick (game thread).
    pub fn tick(&self) {
        self.gui().tick();
        if let Some(e) = self.gui().take_last_error() {
            tracing::warn!("ui tick: {e}");
        }
    }

    /// The option changes vanilla screens made since the last call, as options.txt key/value
    /// pairs, oldest first. The client applies each to its live options (FOV, render distance,
    /// volumes, key binds, ...); `haste.gui.OptionBridge` reports them from vanilla's own
    /// `OptionInstance.onValueUpdate` callbacks.
    pub fn drain_option_changes(&self) -> Vec<(String, String)> {
        self.state(|s| s.take_option_changes())
    }

    /// The inventory slots vanilla Java code changed since the last call (vanilla
    /// `Inventory` numbering, oldest first): creative slot sets, menu clicks, pick block —
    /// edits vanilla predicts client-side, which the server does not echo back (vanilla's
    /// `handleSetCreativeModeSlot` marks the slot remote before `broadcastChanges`, so no
    /// `container_set_slot`/`set_player_inventory` arrives for it).
    pub fn drain_inventory_changes(&self) -> Vec<(i32, haste_core::entity::ItemStackData)> {
        self.state(|s| s.take_inventory_changes())
    }

    /// The resource pack selection the pack screen committed since the last call (vanilla
    /// `Minecraft.reloadResourcePacks` after Done): repository ids, bottom first.
    pub fn take_resource_pack_selection(&self) -> Option<Vec<String>> {
        self.gui().take_resource_pack_ids()
    }

    /// Whether a screen is open (the client releases the mouse and stops game input).
    pub fn screen_open(&self) -> bool {
        self.gui().is_open()
    }

    /// Whether vanilla code wants the mouse grabbed (e.g. after "Back to Game").
    pub fn wants_mouse_grabbed(&self) -> bool {
        self.gui().mouse_grabbed()
    }

    /// The client grabbed or released the mouse itself.
    pub fn set_mouse_grabbed(&self, grabbed: bool) {
        self.gui().set_mouse_grabbed(grabbed);
    }

    /// Whether vanilla code asked to leave the server (pause screen "Disconnect"); clears it.
    pub fn take_disconnect_request(&self) -> bool {
        self.state(|s| std::mem::take(&mut s.disconnect_requested))
    }

    /// Whether the player may use game-master blocks (vanilla
    /// `Player.canUseGameMasterBlocks`: creative and operator level >= 2).
    pub fn game_master_state(&self) -> (bool, u8) {
        self.state(|s| (s.game_mode == GameMode::Creative, s.permission_level))
    }

    /// A use opened the command block editor (vanilla `LocalPlayer.openCommandBlock`).
    pub fn open_used_command_block(&self, pos: haste_protocol::haste_core::BlockPos) {
        if let Err(e) = self.gui().open_command_block(pos) {
            tracing::warn!("opening the command block editor: {e:#}");
        }
    }

    /// A use opened the writable book's editor (vanilla `LocalPlayer.openItemGui`); `hand` is
    /// `InteractionHand.values()` numbering (0 main, 1 off).
    pub fn open_item_gui(&self, hand: i32) {
        if let Err(e) = self.gui().open_item_gui(hand) {
            tracing::warn!("opening the book editor: {e:#}");
        }
    }

    /// The connection ended: vanilla's disconnect screen with the reason; the HUD stops.
    pub fn disconnected(&self, reason: &Component) {
        self.state(|s| s.in_world = false);
        if let Err(e) = self.gui().disconnected(reason) {
            tracing::warn!("disconnect screen: {e:#}");
        }
    }
}
