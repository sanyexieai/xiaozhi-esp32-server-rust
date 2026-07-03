//! 智谱 GLM-TTS（OpenAI 兼容接口）

use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

pub struct ZhipuTtsProvider {
    api_key: String,
    api_url: String,
    model: String,
    voice: Arc<RwLock<String>>,
    response_format: String,
    speed: f64,
    volume: f64,
    client: Client,
}

impl ZhipuTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let api_url = config
            .get("api_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://open.bigmodel.cn/api/paas/v4/audio/speech")
            .to_string();
        Ok(Self {
            api_key: xiaozhi_core::trimmed_config_string(config, "api_key"),
            api_url: api_url.clone(),
            model: config
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("glm-tts")
                .to_string(),
            voice: Arc::new(RwLock::new(
                config
                    .get("voice")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tongtong")
                    .to_string(),
            )),
            response_format: config
                .get("response_format")
                .and_then(|v| v.as_str())
                .unwrap_or("mp3")
                .to_string(),
            speed: config
                .get("speed")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0),
            volume: config
                .get("volume")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0),
            client: crate::http_client::build_http_client(&api_url),
        })
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        if self.api_key.is_empty() {
            return Err(Error::Config("智谱 TTS api_key 未配置".into()));
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": self.voice.read().await.clone(),
            "response_format": self.response_format,
            "speed": self.speed,
            "volume": self.volume,
        });

        let resp = self
            .client
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("智谱 TTS 请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("智谱 TTS HTTP {status}: {err}")));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| Error::Http(format!("智谱 TTS 读取失败: {e}")))
    }
}

#[async_trait]
impl TtsProvider for ZhipuTtsProvider {
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
            &self.response_format,
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
                return Ok(());
            }
        }
        Err(Error::Config("无效的音色配置: 缺少 voice".into()))
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.api_key.is_empty()
    }
}
