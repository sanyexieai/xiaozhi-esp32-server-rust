use async_trait::async_trait;
use tokio::sync::mpsc;
use xiaozhi_core::Result;

#[derive(Debug, Clone)]
pub enum SynthesisEvent {
    AudioFrame(Vec<u8>),
    SentenceStart(String),
    SentenceEnd(String),
    Done,
    Error(String),
}

#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn text_to_speech(
        &self,
        text: &str,
        sample_rate: u32,
        channels: u8,
        frame_duration: u32,
    ) -> Result<Vec<Vec<u8>>>;

    async fn text_to_speech_stream(
        &self,
        text: &str,
        sample_rate: u32,
        channels: u8,
        frame_duration: u32,
    ) -> Result<mpsc::Receiver<Vec<u8>>>;

    async fn set_voice(&self, voice_config: &serde_json::Value) -> Result<()>;

    async fn close(&self) -> Result<()>;

    fn is_valid(&self) -> bool;
}

#[async_trait]
pub trait DualStreamTtsProvider: TtsProvider {
    async fn streaming_synthesize(
        &self,
        text_rx: mpsc::Receiver<String>,
        sample_rate: u32,
        channels: u8,
        frame_duration: u32,
    ) -> Result<mpsc::Receiver<SynthesisEvent>>;
}
