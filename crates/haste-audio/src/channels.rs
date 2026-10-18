//! The sound channel pools, vanilla's `Library.ChannelPool` split and its fullness rule.
//!
//! `Library.init` sizes two `CountingChannelPool`s from the device's `ALC_MONO_SOURCES`
//! (attribute 4112): streaming = `clamp(sqrt(total), 2, 8)`, static =
//! `clamp(total - streaming, 8, 255)`. OpenAL Soft reports 255 mono sources on an ordinary
//! device, which gives vanilla its 247 static + 8 streaming channels. When a pool is full,
//! `CountingChannelPool.acquire` answers null (and warns, in an IDE) and `SoundEngine.play`
//! gives up on the sound entirely — it is not queued and never plays.
//!
//! Haste has no OpenAL device to ask, so the pools are sized from the mono-source count
//! OpenAL Soft would report; the split and the fullness rule are the ported part.

/// The `ALC_MONO_SOURCES` OpenAL Soft reports on an ordinary device (255), the total the
/// pools are split from.
pub const DEVICE_MONO_SOURCES: u32 = 255;

/// Which pool a sound takes its channel from, vanilla's `Library.Pool`: streamed sounds
/// (`stream: true`, music) stream, everything else plays a static buffer
/// (`SoundEngine.play`: `shouldStream() ? STREAMING : STATIC`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pool {
    Static,
    Streaming,
}

/// The two counting pools, vanilla's `Library.ChannelPool`s: a fixed limit per pool and how
/// many channels are in use. A full pool refuses to hand out a channel.
#[derive(Debug)]
pub struct ChannelPools {
    static_limit: usize,
    streaming_limit: usize,
    static_used: usize,
    streaming_used: usize,
    /// Whether each pool already logged that it is full (warn once per full stretch, so a
    /// burst of plays in one tick cannot spam).
    full_logged: [bool; 2],
}

impl Default for ChannelPools {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelPools {
    /// The pools as vanilla fills them from an ordinary OpenAL Soft device (255 mono
    /// sources): 247 static + 8 streaming.
    pub fn new() -> Self {
        Self::with_total_sources(DEVICE_MONO_SOURCES)
    }

    /// The pools for a device reporting `total` mono sources, exactly `Library.init`'s
    /// split: streaming = `clamp(sqrt(total), 2, 8)`, static =
    /// `clamp(total - streaming, 8, 255)`.
    pub fn with_total_sources(total: u32) -> Self {
        let streaming = ((total as f64).sqrt() as u32).clamp(2, 8);
        let static_ = total.saturating_sub(streaming).clamp(8, 255);
        Self {
            static_limit: static_ as usize,
            streaming_limit: streaming as usize,
            static_used: 0,
            streaming_used: 0,
            full_logged: [false; 2],
        }
    }

    /// (limit, used) of one pool.
    fn counts(&self, pool: Pool) -> (usize, usize) {
        match pool {
            Pool::Static => (self.static_limit, self.static_used),
            Pool::Streaming => (self.streaming_limit, self.streaming_used),
        }
    }

    fn full_logged_index(pool: Pool) -> usize {
        match pool {
            Pool::Static => 0,
            Pool::Streaming => 1,
        }
    }

    /// Take a channel from the pool, `CountingChannelPool.acquire`: `false` (the pool is
    /// full, logged once per full stretch) means the asking sound is not started.
    pub fn acquire(&mut self, pool: Pool) -> bool {
        let (limit, used) = self.counts(pool);
        if used >= limit {
            let logged = &mut self.full_logged[Self::full_logged_index(pool)];
            if !*logged {
                tracing::warn!("maximum sound pool size {limit} reached");
                *logged = true;
            }
            return false;
        }
        match pool {
            Pool::Static => self.static_used += 1,
            Pool::Streaming => self.streaming_used += 1,
        }
        true
    }

    /// Give a channel back, `ChannelHandle.release`. Only sounds that hold one may call
    /// this (the engine releases exactly what it acquired).
    pub fn release(&mut self, pool: Pool) {
        match pool {
            Pool::Static => self.static_used = self.static_used.saturating_sub(1),
            Pool::Streaming => self.streaming_used = self.streaming_used.saturating_sub(1),
        }
        let (limit, used) = self.counts(pool);
        if used < limit {
            self.full_logged[Self::full_logged_index(pool)] = false;
        }
    }

    /// Channels currently in use in one pool (diagnostics, tests).
    pub fn used(&self, pool: Pool) -> usize {
        match pool {
            Pool::Static => self.static_used,
            Pool::Streaming => self.streaming_used,
        }
    }

    /// One pool's limit (tests).
    pub fn limit(&self, pool: Pool) -> usize {
        match pool {
            Pool::Static => self.static_limit,
            Pool::Streaming => self.streaming_limit,
        }
    }

    /// `Library.getChannelDebugString`: `Sounds: used/limit + used/limit`.
    pub fn debug_string(&self) -> String {
        format!(
            "Sounds: {}/{} + {}/{}",
            self.static_used, self.static_limit, self.streaming_used, self.streaming_limit
        )
    }
}

#[cfg(test)]
mod tests;
