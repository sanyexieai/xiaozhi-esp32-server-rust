//! Edge 离线 TTS WebSocket 服务

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream_with_source;
use crate::traits::TtsProvider;

pub struct EdgeOfflineTtsProvider {
    server_url: String,
    voice: RwLock<String>,
    sample_rate: u32,
    channels: u8,
    frame_duration: u32,
}

impl EdgeOfflineTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            server_url: config
                .get("server_url")
                .and_then(|v| v.as_str())
                .unwrap_or("ws://127.0.0.1:8080/tts")
                .to_string(),
            voice: RwLock::new(
                config
                    .get("voice")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ),
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(16000) as u32,
            channels: config
                .get("channels")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u8,
            frame_duration: config
                .get("frame_duration")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as u32,
        })
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        if self.server_url.is_empty() {
            return Err(Error::Config("Edge 离线 TTS server_url 未配置".into()));
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let (ws, _) = connect_async(&self.server_url)
            .await
            .map_err(|e| Error::Http(format!("Edge 离线 TTS 连接失败: {e}")))?;
        let (mut write, mut read) = ws.split();

        let req = {
            let mut payload = serde_json::json!({
                "text": text,
                "sample_rate": self.sample_rate,
                "channels": self.channels,
                "frame_duration": self.frame_duration,
            });
            let voice = self.voice.read().await.clone();
            if !voice.is_empty() {
                payload["voice"] = serde_json::json!(voice);
            }
            payload
        };
        write
            .send(Message::Text(req.to_string().into()))
            .await
            .map_err(|e| Error::Http(format!("Edge 离线 TTS 发送失败: {e}")))?;

        let mut audio = Vec::new();
        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| Error::Http(format!("Edge 离线 TTS 接收失败: {e}")))?;
            match msg {
                Message::Binary(data) => audio.extend(data),
                Message::Text(s) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if v.get("done").and_then(|d| d.as_bool()) == Some(true) {
                            break;
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        Ok(audio)
    }
}

#[async_trait]
impl TtsProvider for EdgeOfflineTtsProvider {
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
        let audio = self.synthesize(text).await?;
        if !audio.is_empty() {
            raw_tx.send(audio).await.ok();
        }
        Ok(wrap_tts_audio_stream_with_source(
            raw_rx,
            "pcm",
            sample_rate,
            self.sample_rate,
            channels,
            frame_duration,
        ))
    }

    async fn set_voice(&self, voice_config: &serde_json::Value) -> Result<()> {
        if let Some(voice) = voice_config.get("voice").and_then(|v| v.as_str()) {
            let voice = voice.trim();
            if !voice.is_empty() {
                *self.voice.write().await = voice.to_string();
            }
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.server_url.is_empty()
    }
}
