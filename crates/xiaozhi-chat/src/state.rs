use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use xiaozhi_config::user::UConfig;
use xiaozhi_llm::ChatMessage;

use crate::audio_idle::AudioIdleClock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeMode {
    VadInterrupt = 1,
    AsrInterrupt = 2,
    SpeakerInterrupt = 3,
    AsrFinalInterrupt = 4,
}

impl From<u8> for RealtimeMode {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::VadInterrupt,
            2 => Self::AsrInterrupt,
            3 => Self::SpeakerInterrupt,
            _ => Self::AsrFinalInterrupt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenPhase {
    Idle,
    Listening,
    Processing,
    Speaking,
}

pub struct ClientState {
    pub device_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub device_config: UConfig,
    pub dialogue: Vec<ChatMessage>,
    pub abort: Arc<AtomicBool>,
    pub listen_mode: String,
    pub listen_phase: ListenPhase,
    pub transport_type: String,
    pub welcome_speaking: bool,
    pub welcome_playing: bool,
    pub audio_idle: AudioIdleClock,
}

impl ClientState {
    pub fn new(device_id: String, session_id: String, config: UConfig) -> Self {
        let agent_id = config.agent_id.clone();
        Self {
            device_id,
            agent_id,
            session_id,
            device_config: config,
            dialogue: Vec::new(),
            abort: Arc::new(AtomicBool::new(false)),
            listen_mode: "auto".to_string(),
            listen_phase: ListenPhase::Idle,
            transport_type: "websocket".to_string(),
            welcome_speaking: false,
            welcome_playing: false,
            audio_idle: AudioIdleClock::default(),
        }
    }

    pub fn uses_audio_idle_clock(&self) -> bool {
        self.listen_mode == "auto" || self.is_realtime()
    }

    pub fn start_audio_idle_window(&mut self) {
        if !self.uses_audio_idle_clock() {
            return;
        }
        self.audio_idle.start(Instant::now());
    }

    pub fn pause_audio_idle_window(&mut self) {
        if !self.uses_audio_idle_clock() {
            return;
        }
        self.audio_idle.pause(Instant::now());
    }

    pub fn resume_audio_idle_window(&mut self) {
        if !self.uses_audio_idle_clock() {
            return;
        }
        self.audio_idle.resume(Instant::now());
    }

    pub fn reset_audio_idle_window(&mut self) {
        self.audio_idle.reset();
    }

    pub fn audio_idle_started(&self) -> bool {
        self.audio_idle.started()
    }

    pub fn audio_idle_paused(&self) -> bool {
        self.audio_idle.paused()
    }

    pub fn audio_idle_elapsed_ms(&self) -> u64 {
        self.audio_idle.elapsed(Instant::now()).as_millis() as u64
    }

    pub fn mark_audio_idle_timeout_pending(&mut self) -> bool {
        self.audio_idle.mark_timeout_pending()
    }

    pub fn clear_audio_idle_timeout_pending(&mut self) {
        self.audio_idle.clear_timeout_pending();
    }

    pub fn audio_idle_timeout_pending(&self) -> bool {
        self.audio_idle.timeout_pending()
    }

    pub fn is_realtime(&self) -> bool {
        self.listen_mode == "realtime"
    }

    pub fn trigger_abort(&self) {
        self.abort.store(true, Ordering::SeqCst);
    }

    pub fn clear_abort(&self) {
        self.abort.store(false, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.abort.load(Ordering::SeqCst)
    }
}
