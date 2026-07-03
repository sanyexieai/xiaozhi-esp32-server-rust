use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use xiaozhi_core::{Error, Result};

use crate::traits::{AsrProvider, StreamingResult};
use crate::ws_session::{ReusableWsSession, TaskSessionOutcome, WsConnParts, WsStream};

pub struct FunasrProvider {
    host: String,
    port: String,
    mode: String,
    sample_rate: u32,
    timeout_secs: u64,
    ws: ReusableWsSession,
}

impl FunasrProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            host: config
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_string(),
            port: config
                .get("port")
                .and_then(|v| {
                    v.as_u64()
                        .map(|n| n.to_string())
                        .or_else(|| v.as_str().map(String::from))
                })
                .unwrap_or_else(|| "10095".to_string()),
            mode: config
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("offline")
                .to_string(),
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(16000) as u32,
            timeout_secs: config
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(30),
            ws: ReusableWsSession::new(),
        })
    }

    fn ws_url(&self) -> String {
        format!("ws://{}:{}/", self.host, self.port)
    }

    fn float_to_i16_bytes(pcm: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &sample in pcm {
            let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        bytes
    }

    async fn connect_ws(url: &str, timeout_secs: u64) -> Result<WsStream> {
        let timeout = Duration::from_secs(timeout_secs.max(5));
        let (ws, _) = tokio::time::timeout(timeout, connect_async(url))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|e| Error::WebSocket(format!("FunASR 连接失败: {e}")))?;
        Ok(ws)
    }

    async fn run_streaming_task(
        mut parts: WsConnParts,
        mut audio_rx: mpsc::Receiver<Vec<f32>>,
        mode: String,
        sample_rate: u32,
        result_tx: mpsc::Sender<StreamingResult>,
    ) -> TaskSessionOutcome {
        let init_msg = serde_json::json!({
            "mode": mode,
            "wav_name": format!("rust_client_{}", uuid::Uuid::new_v4()),
            "is_speaking": true,
            "wav_format": "pcm",
            "chunk_size": [5, 10, 5],
            "chunk_interval": 10,
            "sample_rate": sample_rate,
        });
        if parts
            .write
            .send(Message::Text(init_msg.to_string().into()))
            .await
            .is_err()
        {
            let _ = result_tx
                .send(StreamingResult {
                    text: String::new(),
                    is_final: true,
                    confidence: None,
                    error: Some("FunASR 发送配置失败".into()),
                })
                .await;
            return TaskSessionOutcome::Invalidate;
        }

        let mut send_err: Option<String> = None;
        while let Some(chunk) = audio_rx.recv().await {
            let bytes = Self::float_to_i16_bytes(&chunk);
            if parts.write.send(Message::Binary(bytes.into())).await.is_err() {
                send_err = Some("FunASR 发送音频失败".into());
                break;
            }
        }
        if send_err.is_none() {
            let _ = parts
                .write
                .send(Message::Text(r#"{"is_speaking": false}"#.into()))
                .await;
        }

        while let Some(msg) = parts.read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        let is_final = v.get("is_final").and_then(|x| x.as_bool()).unwrap_or(true);
                        let text = v
                            .get("text")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !text.is_empty() {
                            let _ = result_tx
                                .send(StreamingResult {
                                    text,
                                    is_final,
                                    confidence: None,
                                    error: None,
                                })
                                .await;
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }

        if let Some(err) = send_err {
            let _ = result_tx
                .send(StreamingResult {
                    text: String::new(),
                    is_final: true,
                    confidence: None,
                    error: Some(err),
                })
                .await;
            TaskSessionOutcome::Invalidate
        } else {
            TaskSessionOutcome::Reuse(parts)
        }
    }
}

#[async_trait]
impl AsrProvider for FunasrProvider {
    async fn process(&self, pcm_data: &[f32]) -> Result<String> {
        let (tx, rx) = mpsc::channel(32);
        tx.send(pcm_data.to_vec()).await.ok();
        drop(tx);
        super::factory::collect_streaming(self, rx).await
    }

    async fn streaming_recognize(
        &self,
        audio_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<mpsc::Receiver<StreamingResult>> {
        let task_guard = self.ws.acquire_task().await;
        let url = self.ws_url();
        let timeout_secs = self.timeout_secs;
        let mode = self.mode.clone();
        let sample_rate = self.sample_rate;
        let conn_mu = self.ws.conn_mu();

        let parts = ReusableWsSession::take_or_connect(&conn_mu, || {
            Self::connect_ws(&url, timeout_secs)
        })
        .await?;

        let (result_tx, result_rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let outcome =
                Self::run_streaming_task(parts, audio_rx, mode, sample_rate, result_tx).await;
            ReusableWsSession::finish_task(&conn_mu, task_guard, outcome).await;
        });

        Ok(result_rx)
    }

    async fn close(&self) -> Result<()> {
        self.ws.close().await;
        Ok(())
    }

    fn is_valid(&self) -> bool {
        true
    }
}
