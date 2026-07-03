//! 火山引擎 / 豆包语音 V3 TTS（HTTP SSE 流式）

use std::sync::Arc;

use reqwest::Client;
use tokio::sync::RwLock;
use xiaozhi_core::{Error, Result};

use crate::doubao_model::{normalize_doubao_voice, resolve_doubao_tts_model};

#[derive(Clone)]
pub struct DoubaoV3Config {
    pub appid: String,
    pub access_token: String,
    pub resource_id: String,
    pub voice: String,
    pub model: String,
    pub api_url: String,
    pub ws_url: String,
    pub audio_format: String,
    pub sample_rate: u32,
}

impl DoubaoV3Config {
    pub fn from_config(config: &serde_json::Value) -> Self {
        let ws_url = config
            .get("ws_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| {
                "wss://openspeech.bytedance.com/api/v3/tts/unidirectional/stream".to_string()
            });
        let ws_url_ref = ws_url.as_str();
        let api_url = config
            .get("api_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| {
                if ws_url_ref.contains("/unidirectional/stream") {
                    ws_url.replace("/unidirectional/stream", "/unidirectional/sse")
                } else {
                    "https://openspeech.bytedance.com/api/v3/tts/unidirectional/sse".to_string()
                }
            });

        Self {
            appid: xiaozhi_core::trimmed_config_string(config, "appid"),
            access_token: xiaozhi_core::trimmed_config_string(config, "access_token"),
            resource_id: xiaozhi_core::trimmed_config_string(config, "resource_id"),
            voice: normalize_doubao_voice(
                config
                    .get("voice")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            ),
            model: config
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("seed-tts-1.1")
                .to_string(),
            api_url,
            ws_url,
            audio_format: config
                .get("audio_format")
                .or_else(|| config.get("response_format"))
                .and_then(|v| v.as_str())
                .unwrap_or("mp3")
                .to_string(),
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(24000) as u32,
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.appid.is_empty()
            && !self.access_token.is_empty()
            && !normalize_doubao_voice(&self.voice).is_empty()
    }
}

pub struct DoubaoV3Client {
    cfg: Arc<RwLock<DoubaoV3Config>>,
    client: Client,
}

impl DoubaoV3Client {
    pub fn new(cfg: DoubaoV3Config) -> Self {
        let client = crate::http_client::build_http_client(&cfg.api_url);
        Self {
            cfg: Arc::new(RwLock::new(cfg)),
            client,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.cfg
            .try_read()
            .map(|c| c.is_valid())
            .unwrap_or(false)
    }

    pub async fn apply_voice_config(&self, voice_config: &serde_json::Value) {
        let mut cfg = self.cfg.write().await;
        if let Some(voice) = voice_config.get("voice").and_then(|v| v.as_str()) {
            cfg.voice = normalize_doubao_voice(voice);
        }
        if let Some(model) = voice_config.get("model").and_then(|v| v.as_str()) {
            let model = model.trim();
            if !model.is_empty() {
                cfg.model = model.to_string();
            }
        }
        if let Some(resource_id) = voice_config.get("resource_id").and_then(|v| v.as_str()) {
            cfg.resource_id = resource_id.trim().to_string();
        }
    }

    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        let cfg = self.cfg.read().await.clone();
        if !cfg.is_valid() {
            return Err(Error::Config(
                "豆包 TTS 缺少 appid/access_token/voice".into(),
            ));
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let voice = normalize_doubao_voice(&cfg.voice);
        let mut resolved = resolve_doubao_tts_model(&cfg.model, &voice)?;
        if !cfg.resource_id.trim().is_empty() {
            resolved.resource_id = cfg.resource_id.trim().to_string();
        }

        let body = {
            let mut req_params = serde_json::json!({
                "text": text,
                "speaker": voice,
                "audio_params": {
                    "format": cfg.audio_format,
                    "sample_rate": cfg.sample_rate,
                }
            });
            if !resolved.request_model.is_empty() {
                req_params["model"] = serde_json::json!(resolved.request_model);
            }
            serde_json::json!({
                "user": { "uid": uuid::Uuid::new_v4().to_string() },
                "req_params": req_params,
            })
        };

        let resp = self
            .client
            .post(&cfg.api_url)
            .header("X-Api-App-Key", &cfg.appid)
            .header("X-Api-Access-Key", &cfg.access_token)
            .header("X-Api-Resource-Id", &resolved.resource_id)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("豆包 TTS 请求失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(Error::Http(format!("豆包 TTS HTTP {status}: {err}")));
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
                .map_err(|e| Error::Http(format!("豆包 TTS 读取失败: {e}")));
        }

        let text_body = resp
            .text()
            .await
            .map_err(|e| Error::Http(format!("豆包 TTS 读取失败: {e}")))?;

        parse_sse_or_json_audio(&text_body)
    }
}

fn parse_sse_or_json_audio(body: &str) -> Result<Vec<u8>> {
    let mut audio = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data.trim()) {
                append_audio_from_json(&mut audio, &v)?;
            }
        }
    }
    if !audio.is_empty() {
        return Ok(audio);
    }

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        append_audio_from_json(&mut audio, &v)?;
        if !audio.is_empty() {
            return Ok(audio);
        }
    }

    Err(Error::Http("豆包 TTS 响应中未找到音频数据".into()))
}

fn append_audio_from_json(out: &mut Vec<u8>, v: &serde_json::Value) -> Result<()> {
    use base64::Engine;

    if let Some(code) = v.get("code").and_then(|c| c.as_i64()) {
        if code != 0 && code != 20000000 {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("未知错误");
            return Err(Error::Http(format!("豆包 TTS 错误 {code}: {msg}")));
        }
    }

    for key in ["data", "audio", "audio_data"] {
        if let Some(b64) = v.get(key).and_then(|x| x.as_str()) {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                out.extend(bytes);
            }
        }
    }

    if let Some(arr) = v.pointer("/result/audio") {
        if let Some(b64) = arr.as_str() {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                out.extend(bytes);
            }
        }
    }

    Ok(())
}
