use async_trait::async_trait;
use reqwest::Client;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

pub struct OpenAiTtsProvider {
    api_key: String,
    api_url: String,
    model: String,
    voice: RwLock<String>,
    response_format: String,
    speed: f64,
    _stream: bool,
    client: Client,
}

impl OpenAiTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let api_url = config
            .get("api_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.openai.com/v1/audio/speech")
            .to_string();
        let client = crate::http_client::build_http_client(&api_url);
        Ok(Self {
            api_key: xiaozhi_core::trimmed_config_string(config, "api_key"),
            api_url,
            model: config
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("tts-1")
                .to_string(),
            voice: RwLock::new(
                config
                    .get("voice")
                    .and_then(|v| v.as_str())
                    .unwrap_or("alloy")
                    .to_string(),
            ),
            response_format: config
                .get("response_format")
                .and_then(|v| v.as_str())
                .unwrap_or("mp3")
                .to_string(),
            speed: config
                .get("speed")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0),
            _stream: config
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            client,
        })
    }

    pub fn from_index_config(config: &serde_json::Value) -> Result<Self> {
        let mut cfg = config.clone();
        if !cfg.get("api_url").is_some() {
            cfg["api_url"] = serde_json::json!("http://127.0.0.1:7860/audio/speech");
        }
        if !cfg.get("model").is_some() {
            cfg["model"] = serde_json::json!("indextts-vllm");
        }
        Self::from_config(&cfg)
    }
}

#[async_trait]
impl TtsProvider for OpenAiTtsProvider {
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
        let (raw_tx, raw_rx) = mpsc::channel(32);
        let voice = self.voice.read().await.clone();
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": voice,
            "response_format": self.response_format,
            "speed": self.speed,
        });

        let resp = self
            .client
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("OpenAI TTS 失败: {e}")))?;

        if !resp.status().is_success() {
            return Err(Error::Http(format!("OpenAI TTS HTTP {}", resp.status())));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::Http(format!("OpenAI TTS 读取失败: {e}")))?
            .to_vec();
        raw_tx.send(bytes).await.ok();

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
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.api_key.is_empty()
    }
}
