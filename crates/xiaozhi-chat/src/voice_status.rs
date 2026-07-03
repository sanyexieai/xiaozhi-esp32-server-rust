//! 与 Go `internal/data/client/voice_status.go` / `vad.go` 对齐的语音状态

#[derive(Debug, Clone, Default)]
pub struct VoiceStatus {
    pub have_voice: bool,
    pub have_voice_last_time_ms: i64,
    pub client_voice_stop: bool,
    pub silence_threshold_ms: i64,
}

impl VoiceStatus {
    pub fn reset(&mut self) {
        self.have_voice = false;
        self.have_voice_last_time_ms = 0;
        self.client_voice_stop = false;
    }

    pub fn is_silence(&self, idle_ms: i64) -> bool {
        idle_ms > self.silence_threshold_ms
    }
}

#[derive(Debug, Clone, Default)]
pub struct VadTracker {
    pub idle_duration_ms: i64,
    pub voice_duration_in_session_ms: i64,
}

impl VadTracker {
    pub fn add_idle(&mut self, ms: i64) {
        self.idle_duration_ms = self.idle_duration_ms.saturating_add(ms);
    }

    pub fn reset_idle(&mut self) {
        self.idle_duration_ms = 0;
    }

    pub fn add_voice(&mut self, ms: i64) {
        self.voice_duration_in_session_ms = self.voice_duration_in_session_ms.saturating_add(ms);
    }

    pub fn reset_voice_in_session(&mut self) {
        self.voice_duration_in_session_ms = 0;
    }

    pub fn reset_all(&mut self) {
        self.reset_idle();
        self.reset_voice_in_session();
    }
}
