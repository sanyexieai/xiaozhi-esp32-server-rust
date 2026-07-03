//! voice-server WebSocket 流式声纹识别

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message,
};
use xiaozhi_core::{Error, Result};

use crate::traits::{IdentifyResult, SpeakerProvider};

pub struct AsrServerSpeakerProvider {
    ws_url: String,
    threshold: f32,
    conn: Mutex<Option<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>>,
    active: AtomicBool,
    sample_rate: Mutex<u32>,
}

impl AsrServerSpeakerProvider {
    pub fn new(base_url: String, threshold: f64) -> Result<Self> {
        if base_url.is_empty() {
            return Err(Error::Config("声纹服务 base_url 未配置".into()));
        }
        let threshold = threshold.clamp(0.0, 1.0) as f32;
        Ok(Self {
            ws_url: derive_ws_url(&base_url),
            threshold,
            conn: Mutex::new(None),
            active: AtomicBool::new(false),
            sample_rate: Mutex::new(16000),
        })
    }

    async fn connect(&self, sample_rate: u32, agent_id: &str) -> Result<()> {
        let mut url = format!("{}?sample_rate={sample_rate}", self.ws_url);
        if !agent_id.is_empty() {
            url.push_str(&format!("&agent_id={}", url_encode(agent_id)));
        }
        if self.threshold > 0.0 {
            url.push_str(&format!("&threshold={:.6}", self.threshold));
        }

        let (mut ws, _) = connect_async(url.as_str())
            .await
            .map_err(|e| Error::Http(format!("声纹 WS 连接失败: {e}")))?;

        if let Some(msg) = ws.next().await {
            let msg = msg.map_err(|e| Error::Http(format!("声纹 WS 读取失败: {e}")))?;
            if let Message::Text(s) = msg {
                let v: serde_json::Value = serde_json::from_str(&s)
                    .map_err(|e| Error::Http(format!("声纹 WS 连接消息解析失败: {e}")))?;
                if v.get("type").and_then(|t| t.as_str()) != Some("connection") {
                    return Err(Error::Http(format!("声纹 WS 意外连接消息: {s}")));
                }
            }
        } else {
            return Err(Error::Http("声纹 WS 未收到连接确认".into()));
        }

        *self.conn.lock().await = Some(ws);
        *self.sample_rate.lock().await = sample_rate;
        self.active.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn close_conn(&self) {
        let mut guard = self.conn.lock().await;
        if let Some(mut ws) = guard.take() {
            let _ = ws.close(None).await;
        }
        self.active.store(false, Ordering::SeqCst);
    }
}

fn derive_ws_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.starts_with("https://") {
        format!(
            "wss://{}/api/v1/speaker/identify_ws",
            base.trim_start_matches("https://")
        )
    } else if base.starts_with("http://") {
        format!(
            "ws://{}/api/v1/speaker/identify_ws",
            base.trim_start_matches("http://")
        )
    } else if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        format!("ws://{base}/api/v1/speaker/identify_ws")
    }
}

fn url_encode(input: &str) -> String {
    input
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn float32_to_bytes(samples: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(samples.len() * 4);
    for &sample in samples {
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    buf
}

fn parse_identify_result(v: &serde_json::Value) -> IdentifyResult {
    IdentifyResult {
        identified: v.get("identified").and_then(|x| x.as_bool()).unwrap_or(false),
        speaker_id: v
            .get("speaker_id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        speaker_name: v
            .get("speaker_name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        confidence: v
            .get("confidence")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0),
        threshold: v
            .get("threshold")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0),
    }
}

#[async_trait]
impl SpeakerProvider for AsrServerSpeakerProvider {
    async fn start_streaming(&self, sample_rate: u32, agent_id: &str) -> Result<()> {
        if self.active.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.connect(sample_rate, agent_id).await
    }

    async fn send_audio_chunk(&self, pcm: &[f32]) -> Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        if !self.active.load(Ordering::SeqCst) {
            let sr = *self.sample_rate.lock().await;
            self.start_streaming(sr, "").await?;
        }
        let mut guard = self.conn.lock().await;
        let Some(ws) = guard.as_mut() else {
            return Ok(());
        };
        ws.send(Message::Binary(float32_to_bytes(pcm).into()))
            .await
            .map_err(|e| {
                self.active.store(false, Ordering::SeqCst);
                Error::Http(format!("声纹 WS 发送音频失败: {e}"))
            })
    }

    async fn finish_and_identify(&self) -> Result<Option<IdentifyResult>> {
        if !self.active.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut guard = self.conn.lock().await;
        let Some(ws) = guard.as_mut() else {
            self.active.store(false, Ordering::SeqCst);
            return Ok(None);
        };

        ws.send(Message::Text(r#"{"action":"finish"}"#.into()))
            .await
            .map_err(|e| Error::Http(format!("声纹 WS 发送 finish 失败: {e}")))?;

        while let Some(msg) = ws.next().await {
            let msg = msg.map_err(|e| Error::Http(format!("声纹 WS 读取结果失败: {e}")))?;
            if let Message::Text(s) = msg {
                let v: serde_json::Value = serde_json::from_str(&s)
                    .map_err(|e| Error::Http(format!("声纹 WS 结果解析失败: {e}")))?;
                match v.get("type").and_then(|t| t.as_str()) {
                    Some("result") => {
                        if let Some(result) = v.get("result") {
                            self.active.store(false, Ordering::SeqCst);
                            return Ok(Some(parse_identify_result(result)));
                        }
                    }
                    Some("error") => {
                        let msg = v
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("未知错误");
                        self.active.store(false, Ordering::SeqCst);
                        return Err(Error::Http(format!("声纹识别错误: {msg}")));
                    }
                    _ => {}
                }
            }
        }
        self.active.store(false, Ordering::SeqCst);
        Ok(None)
    }

    async fn reset(&self) -> Result<()> {
        self.close_conn().await;
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        self.close_conn().await;
        Ok(())
    }
}

pub type SharedSpeakerProvider = Arc<dyn SpeakerProvider>;
