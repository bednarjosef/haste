// The per-connection counters the HUD and the headless printout read: packet and chunk
// counts, teleports, respawns, the recent chat lines and the sound packet tally.

/// Per-connection counters for the HUD/headless printout.
#[derive(Clone, Debug, Default)]
pub struct NetCounters {
    /// Play packets drained by the driver.
    pub packets: u64,
    /// Teleports accepted.
    pub teleports: u64,
    /// Chunks loaded.
    pub chunks_loaded: u64,
    /// Chunk unloads.
    pub chunks_unloaded: u64,
    /// Chunks the world refused to load.
    pub chunk_errors: u64,
    /// Last chunk error message.
    pub last_chunk_error: Option<String>,
    /// Whether the play `login` packet was seen.
    pub got_login: bool,
    /// Respawns / dimension changes.
    pub respawns: u64,
    /// Last chat/system lines (most recent last, at most 20).
    pub chat: Vec<String>,
    /// `sound`, `sound_entity` and `stop_sound` packets seen.
    pub sound_packets: u64,
    /// Own block sounds the player produced (steps, mining, placing, landing).
    pub own_sounds: u64,
}
