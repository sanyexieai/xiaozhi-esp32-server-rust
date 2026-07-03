//! Minimax T2A v2 语音合成

use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

pub struct MinimaxTtsProvider {
    api_key: String,
    api_url: String,
    model: String,
    voice: Arc<RwLock<String>>,
    speed: f64,
    vol: f64,
    pitch: i32,
    sample_rate: u32,
    bitrate: u32,
    format: String,
    channel: u32,
    client: Client,
}

impl MinimaxTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let api_url = config
            .get("api_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.minimaxi.com/v1/t2a_v2")
            .to_string();
        Ok(Self {
            api_key: xiaozhi_core::trimmed_config_string(config, "api_key"),
            api_url: api_url.clone(),
            model: config
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("speech-2.8-hd")
                .to_string(),
            voice: Arc::new(RwLock::new(
                config
                    .get("voice")
                    .and_then(|v| v.as_str())
                    .unwrap_or("male-qn-qingse")
                    .to_string(),
            )),
            speed: config
                .get("speed")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0),
            vol: config
                .get("vol")
                .or_else(|| config.get("volume"))
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0),
            pitch: config
                .get("pitch")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(32000) as u32,
            bitrate: config
                .get("bitrate")
                .and_then(|v| v.as_u64())
                .unwrap_or(128000) as u32,
            format: config
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("mp3")
                .to_string(),
            channel: config
                .get("channel")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u32,
            client: crate::http_client::build_http_client(&api_url),
        })
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        if self.api_key.is_empty() {
            return Err(Error::Config("Minimax TTS api_key 未配置".into()));
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let voice = self.voice.read().await.clone();
        let body = serde_json::json!({
            "model": self.model,
            "text": text,
            "stream": false,
            "voice_setting": {
                "voice_id": voice,
                "speed": self.speed,
                "vol": self.vol,
                "pitch": self.pitch,
            },
            "audio_setting": {
                "sample_rate": self.sample_rate,
                "bitrate": self.bitrate,
                "format": self.format,
                "channel": self.channel,
            }
        });

        let resp = self
            .client
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("Minimax TTS 请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("Minimax TTS HTTP {status}: {err}")));
        }

        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Http(format!("Minimax TTS 解析失败: {e}")))?;

        if let Some(msg) = v.pointer("/base_resp/status_msg").and_then(|x| x.as_str()) {
            if v.pointer("/base_resp/status_code")
                .and_then(|c| c.as_i64())
                .unwrap_or(0)
                != 0
            {
                return Err(Error::Http(format!("Minimax TTS 错误: {msg}")));
            }
        }

        let audio_hex = v
            .pointer("/data/audio")
            .or_else(|| v.get("audio"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| Error::Http("Minimax TTS 响应中未找到音频".into()))?;

        decode_minimax_audio(audio_hex)
    }
}

fn decode_minimax_audio(raw: &str) -> Result<Vec<u8>> {
    use base64::Engine;

    if let Ok(bytes) = hex::decode(raw.trim()) {
        if !bytes.is_empty() {
            return Ok(bytes);
        }
    }
    base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|e| Error::Http(format!("Minimax 音频解码失败: {e}")))
}

#[async_trait]
impl TtsProvider for MinimaxTtsProvider {
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
            &self.format,
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
