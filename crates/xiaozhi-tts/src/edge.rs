use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use xiaozhi_core::Result;

use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

/// Edge TTS - 通过 Microsoft Edge Read Aloud API
pub struct EdgeTtsProvider {
    voice: Arc<RwLock<String>>,
    rate: Arc<RwLock<String>>,
    volume: Arc<RwLock<String>>,
    pitch: Arc<RwLock<String>>,
    client: Client,
}

impl EdgeTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            voice: Arc::new(RwLock::new(
                config
                    .get("voice")
                    .and_then(|v| v.as_str())
                    .unwrap_or("zh-CN-XiaoxiaoNeural")
                    .to_string(),
            )),
            rate: Arc::new(RwLock::new(
                config
                    .get("rate")
                    .and_then(|v| v.as_str())
                    .unwrap_or("+0%")
                    .to_string(),
            )),
            volume: Arc::new(RwLock::new(
                config
                    .get("volume")
                    .and_then(|v| v.as_str())
                    .unwrap_or("+0%")
                    .to_string(),
            )),
            pitch: Arc::new(RwLock::new(
                config
                    .get("pitch")
                    .and_then(|v| v.as_str())
                    .unwrap_or("+0Hz")
                    .to_string(),
            )),
            client: crate::http_client::build_http_client(
                "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1",
            ),
        })
    }

    fn build_ssml(text: &str, voice: &str, rate: &str, volume: &str, pitch: &str) -> String {
        format!(
            r#"<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='zh-CN'>
<voice name='{voice}'><prosody rate='{rate}' volume='{volume}' pitch='{pitch}'>{text}</prosody></voice>
</speak>"#
        )
    }
}

#[async_trait]
impl TtsProvider for EdgeTtsProvider {
    async fn text_to_speech(
        &self,
        text: &str,
        _sample_rate: u32,
        _channels: u8,
        _frame_duration: u32,
    ) -> Result<Vec<Vec<u8>>> {
        let mut rx = self
            .text_to_speech_stream(text, 16000, 1, 60)
            .await?;
        let mut frames = Vec::new();
        while let Some(frame) = rx.recv().await {
            frames.push(frame);
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
        let (raw_tx, raw_rx) = mpsc::channel(64);
        let voice = self.voice.read().await.clone();
        let rate = self.rate.read().await.clone();
        let volume = self.volume.read().await.clone();
        let pitch = self.pitch.read().await.clone();
        let ssml = Self::build_ssml(text, &voice, &rate, &volume, &pitch);
        let client = self.client.clone();

        tokio::spawn(async move {
            // Edge TTS WebSocket endpoint
            let url = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
            let token_url = "https://www.bing.com/opala/generate/token?key=6A5AA1D4EAFF4E9FB37E23D68491D6F4";

            let token_resp = match client.get(token_url).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("Edge TTS token 获取失败: {e}");
                    return;
                }
            };
            let token: serde_json::Value = match token_resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Edge TTS token 解析失败: {e}");
                    return;
                }
            };
            let token_str = token["token"].as_str().unwrap_or("");

            let resp = client
                .post(url)
                .header("Authorization", format!("Bearer {token_str}"))
                .header("Content-Type", "application/ssml+xml")
                .header("X-Microsoft-OutputFormat", "audio-16khz-32kbitrate-mono-mp3")
                .header("User-Agent", "Mozilla/5.0")
                .body(ssml)
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    if let Ok(bytes) = r.bytes().await {
                        let _ = raw_tx.send(bytes.to_vec()).await;
                    }
                }
                Ok(r) => tracing::error!("Edge TTS HTTP {} voice={}", r.status(), voice),
                Err(e) => tracing::error!("Edge TTS 请求失败: {e}"),
            }
        });

        Ok(wrap_tts_audio_stream(
            raw_rx,
            "mp3",
            sample_rate,
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
        self.voice
            .try_read()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}
