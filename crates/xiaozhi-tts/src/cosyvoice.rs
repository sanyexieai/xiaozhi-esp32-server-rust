//! CosyVoice HTTP TTS

use async_trait::async_trait;
use reqwest::Client;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

pub struct CosyVoiceTtsProvider {
    api_url: String,
    spk_id: RwLock<String>,
    target_sr: u32,
    audio_format: String,
    instruct_text: String,
    client: Client,
}

impl CosyVoiceTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let api_url = config
            .get("api_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(Self {
            api_url: api_url.clone(),
            spk_id: RwLock::new(
                config
                    .get("spk_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ),
            target_sr: config
                .get("target_sr")
                .and_then(|v| v.as_u64())
                .unwrap_or(24000) as u32,
            audio_format: config
                .get("audio_format")
                .and_then(|v| v.as_str())
                .unwrap_or("mp3")
                .to_string(),
            instruct_text: config
                .get("instruct_text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            client: crate::http_client::build_http_client(if api_url.is_empty() {
                "http://127.0.0.1"
            } else {
                &api_url
            }),
        })
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        if self.api_url.is_empty() {
            return Err(Error::Config("CosyVoice api_url 未配置".into()));
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let spk_id = self.spk_id.read().await.clone();
        let body = serde_json::json!({
            "text": text,
            "spk_id": spk_id,
            "target_sr": self.target_sr,
            "format": self.audio_format,
            "instruct_text": self.instruct_text,
        });

        let resp = self
            .client
            .post(&self.api_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("CosyVoice 请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("CosyVoice HTTP {status}: {err}")));
        }

        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if ct.contains("audio") || ct.contains("octet-stream") {
            return resp
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| Error::Http(format!("CosyVoice 读取失败: {e}")));
        }

        let text_body = resp
            .text()
            .await
            .map_err(|e| Error::Http(format!("CosyVoice 读取失败: {e}")))?;

        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text_body) {
            use base64::Engine;
            for key in ["audio", "data", "audio_data"] {
                if let Some(b64) = v.get(key).and_then(|x| x.as_str()) {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                        return Ok(bytes);
                    }
                }
            }
        }
        Err(Error::Http("CosyVoice 响应中未找到音频".into()))
    }
}

#[async_trait]
impl TtsProvider for CosyVoiceTtsProvider {
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
        Ok(wrap_tts_audio_stream(
            raw_rx,
            &self.audio_format,
            sample_rate,
            channels,
            frame_duration,
        ))
    }

    async fn set_voice(&self, voice_config: &serde_json::Value) -> Result<()> {
        let spk_id = voice_config
            .get("voice")
            .or_else(|| voice_config.get("spk_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(spk_id) = spk_id {
            *self.spk_id.write().await = spk_id.to_string();
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.api_url.is_empty()
    }
}
