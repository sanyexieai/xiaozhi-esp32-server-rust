use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use xiaozhi_core::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingResult {
    pub text: String,
    pub is_final: bool,
    pub confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[async_trait]
pub trait AsrProvider: Send + Sync {
    /// 一次性识别整段 PCM (float32, -1.0 ~ 1.0)
    async fn process(&self, pcm_data: &[f32]) -> Result<String>;

    /// 流式识别
    async fn streaming_recognize(
        &self,
        audio_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<mpsc::Receiver<StreamingResult>>;

    async fn close(&self) -> Result<()>;

    fn is_valid(&self) -> bool;
}
