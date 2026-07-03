//! 阿里云 DashScope 千问 TTS（qwen3-tts-flash 等，与 Go `qwen_tts.go` 对齐）

use async_trait::async_trait;
use base64::Engine;
use futures::StreamExt;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream_with_source;
use crate::traits::TtsProvider;

pub struct QwenTtsProvider {
    api_key: String,
    api_url: String,
    model: String,
    voice: Arc<RwLock<String>>,
    language_type: String,
    stream: bool,
    client: Client,
}

impl QwenTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let region = config
            .get("region")
            .and_then(|v| v.as_str())
            .unwrap_or("beijing");
        let default_base = if region.eq_ignore_ascii_case("singapore") {
            "https://dashscope-intl.aliyuncs.com"
        } else {
            "https://dashscope.aliyuncs.com"
        };
        let api_url = config
            .get("api_url")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                format!("{default_base}/api/v1/services/aigc/multimodal-generation/generation")
            });

        let mut api_key = config
            .get("api_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if api_key.is_empty() {
            api_key = std::env::var("DASHSCOPE_API_KEY").unwrap_or_default();
        }

        Ok(Self {
            api_key,
            client: build_http_client(&api_url),
            api_url,
            model: config
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("qwen3-tts-flash")
                .to_string(),
            voice: Arc::new(RwLock::new(
                config
                    .get("voice")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Cherry")
                    .to_string(),
            )),
            language_type: config
                .get("language_type")
                .and_then(|v| v.as_str())
                .unwrap_or("Chinese")
                .to_string(),
            stream: config
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        })
    }

    async fn request_body(&self, text: &str) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "input": {
                "text": text,
                "voice": self.voice.read().await.clone(),
                "language_type": self.language_type,
            }
        })
    }

    async fn synthesize_non_stream(&self, text: &str) -> Result<Vec<u8>> {
        if self.api_key.is_empty() {
            return Err(Error::Config("千问 TTS api_key 未配置".into()));
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let resp = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&self.request_body(text).await)
            .send()
            .await
            .map_err(|e| Error::Http(format!("千问 TTS 请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("千问 TTS HTTP {status}: {err}")));
        }

        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Http(format!("千问 TTS 解析失败: {e}")))?;

        extract_qwen_audio(&v, &self.client).await
    }

    async fn synthesize_sse_stream(&self, text: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        if self.api_key.is_empty() {
            return Err(Error::Config("千问 TTS api_key 未配置".into()));
        }
        if text.trim().is_empty() {
            let (_tx, rx) = mpsc::channel(1);
            return Ok(rx);
        }

        let resp = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("X-DashScope-SSE", "enable")
            .json(&self.request_body(text).await)
            .send()
            .await
            .map_err(|e| Error::Http(format!("千问 TTS 流式请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("千问 TTS HTTP {status}: {err}")));
        }

        let (tx, rx) = mpsc::channel(32);
        let mut byte_stream = resp.bytes_stream();
        tokio::spawn(async move {
            let mut buffer = String::new();
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("千问 TTS SSE 读取失败: {e}");
                        break;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find("\n\n") {
                    let block: String = buffer.drain(..pos + 2).collect();
                    if let Err(e) = handle_sse_block(&block, &tx).await {
                        tracing::error!("千问 TTS SSE 解析失败: {e}");
                        return;
                    }
                }
            }
            if !buffer.trim().is_empty() {
                let _ = handle_sse_block(&buffer, &tx).await;
            }
        });

        Ok(rx)
    }
}

fn build_http_client(api_url: &str) -> Client {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60));
    if should_bypass_proxy(api_url) {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn should_bypass_proxy(url: &str) -> bool {
    let url = url.to_lowercase();
    [
        "dashscope.aliyuncs.com",
        "dashscope-intl.aliyuncs.com",
        "localhost",
        "127.0.0.1",
    ]
    .iter()
    .any(|host| url.contains(host))
}

async fn handle_sse_block(block: &str, tx: &mpsc::Sender<Vec<u8>>) -> Result<()> {
    for line in block.lines() {
        let data = line
            .strip_prefix("data:")
            .map(str::trim)
            .unwrap_or("");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(data)
            .map_err(|e| Error::Http(format!("千问 TTS SSE JSON 解析失败: {e}")))?;
        if let Some(code) = v.get("code").and_then(|x| x.as_str()) {
            if !code.is_empty() && code != "Success" {
                let msg = v
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or(code);
                return Err(Error::Http(format!("千问 TTS SSE 错误 [{code}]: {msg}")));
            }
        }
        if let Some(status) = v.get("status_code").and_then(|x| x.as_u64()) {
            if status != 0 && status != 200 {
                let msg = v
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown error");
                return Err(Error::Http(format!("千问 TTS SSE 错误: {msg}")));
            }
        }
        if let Some(bytes) = decode_inline_audio(&v)? {
            if !bytes.is_empty() {
                tx.send(bytes).await.ok();
            }
        }
    }
    Ok(())
}

fn decode_inline_audio(v: &serde_json::Value) -> Result<Option<Vec<u8>>> {
    if let Some(b64) = v
        .pointer("/output/audio/data")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(cleaned)
            .map_err(|e| Error::Http(format!("千问 TTS Base64 解码失败: {e}")))?;
        return Ok(Some(strip_leading_wav_if_needed(bytes)));
    }
    Ok(None)
}

fn strip_leading_wav_if_needed(data: Vec<u8>) -> Vec<u8> {
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        if let Some(offset) = wav_data_offset(&data) {
            if offset < data.len() {
                return data[offset..].to_vec();
            }
        }
    }
    data
}

fn wav_data_offset(data: &[u8]) -> Option<usize> {
    let mut offset = 12usize;
    while offset + 8 <= data.len() {
        let chunk_size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?) as usize;
        if &data[offset..offset + 4] == b"data" {
            return Some(offset + 8);
        }
        offset += 8 + chunk_size;
    }
    None
}

async fn extract_qwen_audio(v: &serde_json::Value, client: &Client) -> Result<Vec<u8>> {
    if let Some(bytes) = decode_inline_audio(v)? {
        return Ok(bytes);
    }

    if let Some(url) = v.pointer("/output/audio/url").and_then(|x| x.as_str()) {
        let bytes = client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Http(format!("千问 TTS 下载音频失败: {e}")))?
            .bytes()
            .await
            .map_err(|e| Error::Http(format!("千问 TTS 读取音频失败: {e}")))?;
        return Ok(bytes.to_vec());
    }

    Err(Error::Http("千问 TTS 响应中未找到音频".into()))
}

#[async_trait]
impl TtsProvider for QwenTtsProvider {
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
        let raw_rx = if self.stream {
            self.synthesize_sse_stream(text).await?
        } else {
            let (raw_tx, raw_rx) = mpsc::channel(8);
            let audio = self.synthesize_non_stream(text).await?;
            if !audio.is_empty() {
                raw_tx.send(audio).await.ok();
            }
            raw_rx
        };
        Ok(wrap_tts_audio_stream_with_source(
            raw_rx,
            "pcm",
            sample_rate,
            24000,
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
        } else {
            return Err(Error::Config("无效的音色配置: 缺少 voice".into()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_language_is_chinese() {
        let p = QwenTtsProvider::from_config(&serde_json::json!({
            "api_key": "sk-test",
        }))
        .unwrap();
        assert_eq!(p.language_type, "Chinese");
        assert!(p.stream);
    }
}
