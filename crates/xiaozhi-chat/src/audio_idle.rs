//! 对齐 Go `internal/data/client/audio_idle.go`

use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct AudioIdleClock {
    start_at: Option<Instant>,
    pause_at: Option<Instant>,
    paused: bool,
    timeout_pending: bool,
}

impl AudioIdleClock {
    pub fn start(&mut self, now: Instant) {
        self.start_at = Some(now);
        self.pause_at = None;
        self.paused = false;
        self.timeout_pending = false;
    }

    pub fn pause(&mut self, now: Instant) {
        if self.start_at.is_none() || self.paused {
            return;
        }
        self.pause_at = Some(now);
        self.paused = true;
    }

    pub fn resume(&mut self, now: Instant) {
        if self.start_at.is_none() || !self.paused {
            return;
        }
        let pause_at = self.pause_at.unwrap_or(now);
        let now = now.max(pause_at);
        if let Some(start) = self.start_at {
            self.start_at = Some(start + now.duration_since(pause_at));
        }
        self.pause_at = None;
        self.paused = false;
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        let Some(start) = self.start_at else {
            return Duration::ZERO;
        };
        let end = if self.paused {
            self.pause_at.unwrap_or(now)
        } else {
            now
        };
        if end < start {
            Duration::ZERO
        } else {
            end - start
        }
    }

    pub fn reset(&mut self) {
        self.start_at = None;
        self.pause_at = None;
        self.paused = false;
        self.timeout_pending = false;
    }

    pub fn started(&self) -> bool {
        self.start_at.is_some()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn mark_timeout_pending(&mut self) -> bool {
        if self.timeout_pending {
            return false;
        }
        self.timeout_pending = true;
        true
    }

    pub fn clear_timeout_pending(&mut self) {
        self.timeout_pending = false;
    }

    pub fn timeout_pending(&self) -> bool {
        self.timeout_pending
    }
}
