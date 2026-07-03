//! 讯飞在线 TTS v2 WebSocket

use async_trait::async_trait;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream_with_source;
use crate::traits::TtsProvider;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct XunfeiTtsConfig {
    pub app_id: String,
    pub api_key: String,
    pub api_secret: String,
    pub host: String,
    pub path: String,
    pub voice: String,
    pub audio_encoding: String,
    pub sample_rate: u32,
    pub speed: i32,
    pub volume: i32,
    pub pitch: i32,
    pub tte: String,
}

impl XunfeiTtsConfig {
    pub fn from_config(config: &serde_json::Value) -> Self {
        let ws_url = config
            .get("ws_url")
            .and_then(|v| v.as_str())
            .unwrap_or("wss://tts-api.xfyun.cn/v2/tts");
        let (host, path) = parse_ws_host_path(ws_url);
        Self {
            app_id: config
                .get("app_id")
                .or_else(|| config.get("appid"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            api_key: config
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            api_secret: config
                .get("api_secret")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            host,
            path,
            voice: config
                .get("voice")
                .and_then(|v| v.as_str())
                .unwrap_or("xiaoyan")
                .to_string(),
            audio_encoding: config
                .get("audio_encoding")
                .and_then(|v| v.as_str())
                .unwrap_or("raw")
                .to_string(),
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(16000) as u32,
            speed: config
                .get("speed")
                .and_then(|v| v.as_i64())
                .unwrap_or(50) as i32,
            volume: config
                .get("volume")
                .and_then(|v| v.as_i64())
                .unwrap_or(50) as i32,
            pitch: config
                .get("pitch")
                .and_then(|v| v.as_i64())
                .unwrap_or(50) as i32,
            tte: config
                .get("tte")
                .and_then(|v| v.as_str())
                .unwrap_or("UTF8")
                .to_string(),
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.app_id.is_empty() && !self.api_key.is_empty() && !self.api_secret.is_empty()
    }

    fn build_ws_url(&self) -> Result<String> {
        let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let signature_origin = format!(
            "host: {}\ndate: {}\nGET {} HTTP/1.1",
            self.host, date, self.path
        );
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes())
            .map_err(|e| Error::Config(format!("讯飞 HMAC 初始化失败: {e}")))?;
        mac.update(signature_origin.as_bytes());
        let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let authorization_origin = format!(
            r#"api_key="{}", algorithm="hmac-sha256", headers="host date request-line", signature="{}""#,
            self.api_key, signature
        );
        let authorization = base64::engine::general_purpose::STANDARD
            .encode(authorization_origin.as_bytes());
        Ok(format!(
            "wss://{}{}?authorization={}&date={}&host={}",
            self.host,
            self.path,
            url_encode(&authorization),
            url_encode(&date),
            url_encode(&self.host)
        ))
    }

    fn audio_format(&self) -> String {
        format!("audio/L16;rate={}", self.sample_rate)
    }

    fn aue(&self) -> &str {
        match self.audio_encoding.as_str() {
            "mp3" | "lame" => "lame",
            "opus" => "opus",
            _ => "raw",
        }
    }
}

pub struct XunfeiTtsProvider {
    cfg: XunfeiTtsConfig,
    voice: RwLock<String>,
}

impl XunfeiTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let cfg = XunfeiTtsConfig::from_config(config);
        Ok(Self {
            voice: RwLock::new(cfg.voice.clone()),
            cfg,
        })
    }

    async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        if !self.cfg.is_valid() {
            return Err(Error::Config(
                "讯飞 TTS 缺少 app_id/api_key/api_secret".into(),
            ));
        }
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let ws_url = self.cfg.build_ws_url()?;
        let mut request = ws_url
            .into_client_request()
            .map_err(|e| Error::Http(format!("讯飞 TTS WS 请求构建失败: {e}")))?;
        request
            .headers_mut()
            .insert("Host", self.cfg.host.parse().unwrap());

        let (ws, _) = connect_async(request)
            .await
            .map_err(|e| Error::Http(format!("讯飞 TTS WS 连接失败: {e}")))?;
        let (mut write, mut read) = ws.split();

        let text_b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let voice = self.voice.read().await.clone();
        let frame = serde_json::json!({
            "common": { "app_id": self.cfg.app_id },
            "business": {
                "aue": self.cfg.aue(),
                "auf": self.cfg.audio_format(),
                "vcn": voice,
                "speed": self.cfg.speed,
                "volume": self.cfg.volume,
                "pitch": self.cfg.pitch,
                "tte": self.cfg.tte,
            },
            "data": {
                "status": 2,
                "text": text_b64
            }
        });
        write
            .send(Message::Text(frame.to_string().into()))
            .await
            .map_err(|e| Error::Http(format!("讯飞 TTS 发送失败: {e}")))?;

        let mut audio = Vec::new();
        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| Error::Http(format!("讯飞 TTS 接收失败: {e}")))?;
            if let Message::Text(s) = msg {
                let v: serde_json::Value = serde_json::from_str(&s)
                    .map_err(|e| Error::Http(format!("讯飞 TTS JSON 解析失败: {e}")))?;
                if let Some(code) = v.get("code").and_then(|c| c.as_i64()) {
                    if code != 0 {
                        let msg = v
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("未知错误");
                        return Err(Error::Http(format!("讯飞 TTS 错误 {code}: {msg}")));
                    }
                }
                if let Some(b64) = v.pointer("/data/audio").and_then(|x| x.as_str()) {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                        audio.extend(bytes);
                    }
                }
                if v.pointer("/data/status")
                    .and_then(|s| s.as_i64())
                    == Some(2)
                {
                    break;
                }
            }
        }
        Ok(audio)
    }
}

fn parse_ws_host_path(ws_url: &str) -> (String, String) {
    let without_scheme = ws_url
        .trim_start_matches("wss://")
        .trim_start_matches("ws://");
    if let Some((host, path)) = without_scheme.split_once('/') {
        (host.to_string(), format!("/{path}"))
    } else {
        (without_scheme.to_string(), "/v2/tts".to_string())
    }
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[async_trait]
impl TtsProvider for XunfeiTtsProvider {
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
        let (fmt, source_rate) = match self.cfg.aue() {
            "lame" => ("mp3", 0),
            "opus" => ("opus", 0),
            _ => ("pcm", self.cfg.sample_rate),
        };
        Ok(wrap_tts_audio_stream_with_source(
            raw_rx,
            fmt,
            sample_rate,
            source_rate,
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
        self.cfg.is_valid()
    }
}

pub struct XunfeiSuperTtsProvider {
    inner: XunfeiTtsProvider,
}

impl XunfeiSuperTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            inner: XunfeiTtsProvider::from_config(config)?,
        })
    }
}

#[async_trait]
impl TtsProvider for XunfeiSuperTtsProvider {
    async fn text_to_speech(
        &self,
        text: &str,
        sample_rate: u32,
        channels: u8,
        frame_duration: u32,
    ) -> Result<Vec<Vec<u8>>> {
        self.inner
            .text_to_speech(text, sample_rate, channels, frame_duration)
            .await
    }

    async fn text_to_speech_stream(
        &self,
        text: &str,
        sample_rate: u32,
        channels: u8,
        frame_duration: u32,
    ) -> Result<mpsc::Receiver<Vec<u8>>> {
        self.inner
            .text_to_speech_stream(text, sample_rate, channels, frame_duration)
            .await
    }

    async fn set_voice(&self, voice_config: &serde_json::Value) -> Result<()> {
        self.inner.set_voice(voice_config).await
    }

    async fn close(&self) -> Result<()> {
        self.inner.close().await
    }

    fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }
}
