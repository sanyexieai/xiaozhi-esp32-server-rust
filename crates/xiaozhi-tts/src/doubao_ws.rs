//! 豆包 TTS WebSocket 流式（V3 二进制协议）

use async_trait::async_trait;
use tokio::sync::mpsc;
use xiaozhi_core::Result;

use crate::doubao::DoubaoV3Config;
use crate::doubao_v3_ws::DoubaoV3WsClient;
use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

pub struct DoubaoWsTtsProvider {
    client: DoubaoV3WsClient,
}

impl DoubaoWsTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            client: DoubaoV3WsClient::new(DoubaoV3Config::from_config(config)),
        })
    }
}

#[async_trait]
impl TtsProvider for DoubaoWsTtsProvider {
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
        let raw_rx = self.client.synthesize_stream(text).await?;
        let audio_format = self.client.audio_format().await;
        Ok(wrap_tts_audio_stream(
            raw_rx,
            &audio_format,
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
