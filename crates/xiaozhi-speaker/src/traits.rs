use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use xiaozhi_core::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentifyResult {
    pub identified: bool,
    #[serde(default)]
    pub speaker_id: String,
    #[serde(default)]
    pub speaker_name: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub threshold: f64,
}

/// 兼容旧字段名
pub type SpeakerResult = IdentifyResult;

#[async_trait]
pub trait SpeakerProvider: Send + Sync {
    async fn start_streaming(&self, sample_rate: u32, agent_id: &str) -> Result<()>;
    async fn send_audio_chunk(&self, pcm: &[f32]) -> Result<()>;
    async fn finish_and_identify(&self) -> Result<Option<IdentifyResult>>;
    async fn reset(&self) -> Result<()>;
    async fn close(&self) -> Result<()>;
}
