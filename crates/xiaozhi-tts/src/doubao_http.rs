//! 豆包 TTS HTTP 方式

use async_trait::async_trait;
use tokio::sync::mpsc;
use xiaozhi_core::Result;

use crate::doubao::{DoubaoV3Client, DoubaoV3Config};
use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

pub struct DoubaoTtsProvider {
    client: DoubaoV3Client,
}

impl DoubaoTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let mut cfg = DoubaoV3Config::from_config(config);
        if cfg.api_url.contains("/stream") || cfg.api_url.contains("/sse") {
            cfg.api_url = "https://openspeech.bytedance.com/api/v3/tts/unidirectional".to_string();
        }
        Ok(Self {
            client: DoubaoV3Client::new(cfg),
        })
    }
}

#[async_trait]
impl TtsProvider for DoubaoTtsProvider {
    async fn text_to_speech(
        &self,
        text: &str,
        sample_rate: u32,
        channels: u8,
        frame_duration: u32,
    ) -> Result<Vec<Vec<u8>>> {
        let mut rx = self
            .text_to_speech_stream(text, sample_rate, channels, frame_duration)
            .await?;
        let mut frames = Vec::new();
        while let Some(f) = rx.recv().await {
            frames.push(f);
        }
        Ok(frames)
    }

    async fn text_to_speech_stream(
        &self,
        text: &str,
        sample_rate: u32,
        channels: u8,
        frame_duration: u32,
    ) -> Result<mpsc::Receiver<Vec<u8>>> {
        let (raw_tx, raw_rx) = mpsc::channel(8);
        let audio = self.client.synthesize(text).await?;
        if !audio.is_empty() {
            raw_tx.send(audio).await.ok();
        }
        Ok(wrap_tts_audio_stream(
            raw_rx,
            "mp3",
            sample_rate,
            channels,
            frame_duration,
        ))
    }

    async fn set_voice(&self, voice_config: &serde_json::Value) -> Result<()> {
        self.client.apply_voice_config(voice_config).await;
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        self.client.is_valid()
    }
}
