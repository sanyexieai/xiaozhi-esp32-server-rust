//! 会话 LLM/TTS 运行时上下文（供 LlmManager / TtsManager 读取，对齐 Go ClientState + pool provider）

use std::sync::Arc;

use xiaozhi_llm::LlmProvider;
use xiaozhi_protocol::audio::AudioParams;
use xiaozhi_tts::TtsProvider;

use crate::state::ClientState;

#[derive(Clone)]
pub struct SessionMedia {
    pub tts: Arc<dyn TtsProvider>,
    pub llm: Arc<dyn LlmProvider>,
    pub audio_params: AudioParams,
    pub session_id: String,
    pub device_id: String,
    pub agent_id: String,
}

impl SessionMedia {
    pub fn from_session(
        tts: Arc<dyn TtsProvider>,
        llm: Arc<dyn LlmProvider>,
        state: &ClientState,
        audio_params: AudioParams,
    ) -> Self {
        Self {
            tts,
            llm,
            audio_params,
            session_id: state.session_id.clone(),
            device_id: state.device_id.clone(),
            agent_id: state.agent_id.clone(),
        }
    }
}
