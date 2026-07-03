//! 阿里云 Qwen3 实时 ASR（与 Go `aliyun_qwen3` 对齐）

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::header::AUTHORIZATION,
        http::HeaderValue,
        Message,
    },
};
use xiaozhi_core::{Error, Result};

use crate::traits::{AsrProvider, StreamingResult};
use crate::ws_session::{ReusableWsSession, TaskSessionOutcome, WsConnParts, WsStream, WsWrite};

#[derive(Clone)]
struct Qwen3Config {
    api_key: String,
    ws_url: String,
    model: String,
    format: String,
    sample_rate: u32,
    language: String,
    auto_end: bool,
    vad_threshold: f64,
    vad_silence_ms: u32,
    timeout_secs: u64,
}

pub struct AliyunQwen3AsrProvider {
    cfg: Qwen3Config,
    ws: ReusableWsSession,
}

impl AliyunQwen3AsrProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            cfg: Qwen3Config {
                api_key: xiaozhi_core::dashscope_api_key(config),
                ws_url: config
                    .get("ws_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("wss://dashscope.aliyuncs.com/api-ws/v1/realtime")
                    .to_string(),
                model: config
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("qwen3-asr-flash-realtime")
                    .to_string(),
                format: config
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pcm")
                    .to_string(),
                sample_rate: config
                    .get("sample_rate")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(16000) as u32,
                language: config
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("zh")
                    .to_string(),
                auto_end: config
                    .get("auto_end")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                vad_threshold: config
                    .get("vad_threshold")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                vad_silence_ms: config
                    .get("vad_silence_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(400) as u32,
                timeout_secs: config
                    .get("timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30),
            },
            ws: ReusableWsSession::new(),
        })
    }

    fn ws_url_with_model(cfg: &Qwen3Config) -> String {
        if cfg.model.is_empty() || cfg.ws_url.contains("model=") {
            return cfg.ws_url.clone();
        }
        format!("{}?model={}", cfg.ws_url, cfg.model)
    }

    fn float_to_i16_bytes(pcm: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &sample in pcm {
            let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        bytes
    }

    fn build_session_update(cfg: &Qwen3Config) -> serde_json::Value {
        let mut session = serde_json::json!({
            "modalities": ["text"],
            "input_audio_format": cfg.format,
            "sample_rate": cfg.sample_rate,
            "input_audio_transcription": {
                "language": cfg.language,
            },
        });
        if cfg.auto_end {
            session["turn_detection"] = serde_json::json!({
                "type": "server_vad",
                "threshold": cfg.vad_threshold,
                "silence_duration_ms": cfg.vad_silence_ms,
            });
        }
        serde_json::json!({
            "event_id": "session_update",
            "type": "session.update",
            "session": session,
        })
    }

    async fn connect_ws(cfg: &Qwen3Config) -> Result<WsStream> {
        if let Some(issue) = xiaozhi_core::dashscope_http_api_key_issue(&cfg.api_key) {
            return Err(Error::Auth(issue.to_string()));
        }
        let url = Self::ws_url_with_model(cfg);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| Error::WebSocket(format!("构建 WebSocket 请求失败: {e}")))?;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", cfg.api_key))
                .map_err(|e| Error::WebSocket(format!("Authorization 头无效: {e}")))?,
        );
        request
            .headers_mut()
            .insert("OpenAI-Beta", HeaderValue::from_static("realtime=v1"));
        let timeout = Duration::from_secs(cfg.timeout_secs.max(5));
        let (ws, _) = tokio::time::timeout(timeout, connect_async(request))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("401") || msg.contains("403") {
                    Error::Auth(xiaozhi_core::dashscope_ws_auth_error_hint(
                        &cfg.api_key,
                        "阿里云 Qwen3 ASR",
                    ))
                } else {
                    Error::WebSocket(format!("阿里云 Qwen3 连接失败: {msg}"))
                }
            })?;
        Ok(ws)
    }

    fn transcription_text(v: &serde_json::Value) -> String {
        v.pointer("/item/transcription/text")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("transcript").and_then(|x| x.as_str()))
            .or_else(|| v.get("text").and_then(|x| x.as_str()))
            .or_else(|| v.get("stash").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string()
    }

    async fn send_error(tx: &mpsc::Sender<StreamingResult>, err: impl ToString) {
        let _ = tx
            .send(StreamingResult {
                text: String::new(),
                is_final: true,
                confidence: None,
                error: Some(err.to_string()),
            })
            .await;
    }

    async fn wait_signal(
        rx: &mut mpsc::Receiver<()>,
        timeout: Duration,
        label: &str,
    ) -> Result<()> {
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Some(())) => Ok(()),
            Ok(None) => Err(Error::WebSocket(format!("{label}: 通道已关闭"))),
            Err(_) => Err(Error::Timeout),
        }
    }

    async fn send_pcm_chunk(write: &mut WsWrite, pcm: &[f32]) -> Result<()> {
        let bytes = Self::float_to_i16_bytes(pcm);
        let append = serde_json::json!({
            "type": "input_audio_buffer.append",
            "audio": B64.encode(bytes),
        });
        write
            .send(Message::Text(append.to_string().into()))
            .await
            .map_err(|e| Error::WebSocket(format!("发送音频失败: {e}")))
    }

    async fn run_qwen3_task(
        cfg: Qwen3Config,
        conn_mu: std::sync::Arc<tokio::sync::Mutex<Option<WsConnParts>>>,
        mut audio_rx: mpsc::Receiver<Vec<f32>>,
        result_tx: mpsc::Sender<StreamingResult>,
    ) -> TaskSessionOutcome {
        let wait = Duration::from_secs(cfg.timeout_secs.max(5));
        let short_wait = Duration::from_secs(5);

        let (session_updated_tx, mut session_updated_rx) = mpsc::channel(1);
        let (buffer_committed_tx, mut buffer_committed_rx) = mpsc::channel(1);
        let (final_result_tx, mut final_result_rx) = mpsc::channel(1);
        let (session_finished_tx, mut session_finished_rx) = mpsc::channel(1);

        let Some(first_pcm) = audio_rx.recv().await else {
            return TaskSessionOutcome::Invalidate;
        };

        let mut parts = match ReusableWsSession::take_or_connect(&conn_mu, || Self::connect_ws(&cfg))
            .await
        {
            Ok(parts) => parts,
            Err(e) => {
                Self::send_error(&result_tx, e).await;
                return TaskSessionOutcome::Invalidate;
            }
        };

        let (read_done_tx, read_done_rx) = tokio::sync::oneshot::channel();
        let mut read = parts.read;
        let result_tx_reader = result_tx.clone();

        let reader = tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                let Ok(Message::Text(text)) = msg else {
                    break;
                };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                match v.get("type").and_then(|x| x.as_str()) {
                    Some("session.updated") => {
                        let _ = session_updated_tx.send(()).await;
                    }
                    Some("input_audio_buffer.committed") => {
                        let _ = buffer_committed_tx.send(()).await;
                    }
                    Some("conversation.item.input_audio_transcription.text") => {
                        let text = Self::transcription_text(&v);
                        if !text.is_empty() {
                            let _ = result_tx_reader
                                .send(StreamingResult {
                                    text,
                                    is_final: false,
                                    confidence: None,
                                    error: None,
                                })
                                .await;
                        }
                    }
                    Some("conversation.item.input_audio_transcription.completed") => {
                        let text = Self::transcription_text(&v);
                        let _ = final_result_tx.send(()).await;
                        let _ = result_tx_reader
                            .send(StreamingResult {
                                text,
                                is_final: true,
                                confidence: None,
                                error: None,
                            })
                            .await;
                    }
                    Some("session.finished") => {
                        let _ = session_finished_tx.send(()).await;
                        break;
                    }
                    Some("error") => {
                        let err = v
                            .pointer("/error/message")
                            .and_then(|x| x.as_str())
                            .unwrap_or("unknown error");
                        Self::send_error(
                            &result_tx_reader,
                            format!("aliyun qwen3 error: {err}"),
                        )
                        .await;
                        break;
                    }
                    _ => {}
                }
            }
            let _ = read_done_tx.send(read);
        });

        let fail = |reader: tokio::task::JoinHandle<()>| {
            reader.abort();
            TaskSessionOutcome::Invalidate
        };

        let update = Self::build_session_update(&cfg);
        if parts
            .write
            .send(Message::Text(update.to_string().into()))
            .await
            .is_err()
        {
            Self::send_error(&result_tx, "发送 session.update 失败").await;
            return fail(reader);
        }
        if Self::wait_signal(&mut session_updated_rx, wait, "wait session.updated")
            .await
            .is_err()
        {
            Self::send_error(&result_tx, "wait session.updated timeout").await;
            return fail(reader);
        }

        if Self::send_pcm_chunk(&mut parts.write, &first_pcm)
            .await
            .is_err()
        {
            Self::send_error(&result_tx, "发送音频失败").await;
            return fail(reader);
        }
        while let Some(pcm) = audio_rx.recv().await {
            if Self::send_pcm_chunk(&mut parts.write, &pcm).await.is_err() {
                Self::send_error(&result_tx, "发送音频失败").await;
                return fail(reader);
            }
        }

        if !cfg.auto_end {
            let commit = serde_json::json!({
                "event_id": "audio_commit",
                "type": "input_audio_buffer.commit",
            });
            if parts
                .write
                .send(Message::Text(commit.to_string().into()))
                .await
                .is_err()
            {
                Self::send_error(&result_tx, "发送 commit 失败").await;
                return fail(reader);
            }
            let _ = Self::wait_signal(
                &mut buffer_committed_rx,
                short_wait,
                "wait buffer.committed",
            )
            .await;
        }

        let _ = Self::wait_signal(
            &mut final_result_rx,
            short_wait,
            "wait final transcription",
        )
        .await;

        let finish = serde_json::json!({
            "event_id": "session_finish",
            "type": "session.finish",
        });
        let _ = parts
            .write
            .send(Message::Text(finish.to_string().into()))
            .await;
        let finished_ok = Self::wait_signal(
            &mut session_finished_rx,
            wait,
            "wait session.finished",
        )
        .await
        .is_ok();

        let read = match read_done_rx.await {
            Ok(read) => read,
            Err(_) => {
                reader.abort();
                return TaskSessionOutcome::Invalidate;
            }
        };
        let _ = reader.await;

        if finished_ok {
            TaskSessionOutcome::Reuse(WsConnParts {
                write: parts.write,
                read,
            })
        } else {
            TaskSessionOutcome::Invalidate
        }
    }
}

#[async_trait]
impl AsrProvider for AliyunQwen3AsrProvider {
    async fn process(&self, pcm_data: &[f32]) -> Result<String> {
        if pcm_data.is_empty() {
            return Ok(String::new());
        }
        let (tx, rx) = mpsc::channel(4);
        tx.send(pcm_data.to_vec()).await.ok();
        drop(tx);
        let text = super::factory::collect_streaming(self, rx).await?;
        if text.trim().is_empty() {
            return Err(Error::Http("阿里云 Qwen3 ASR 未返回识别文本".into()));
        }
        Ok(text)
    }

    async fn streaming_recognize(
        &self,
        audio_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<mpsc::Receiver<StreamingResult>> {
        let task_guard = self.ws.acquire_task().await;
        let cfg = self.cfg.clone();
        let conn_mu = self.ws.conn_mu();
        let (result_tx, result_rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let outcome =
                AliyunQwen3AsrProvider::run_qwen3_task(cfg, conn_mu.clone(), audio_rx, result_tx)
                    .await;
            ReusableWsSession::finish_task(&conn_mu, task_guard, outcome).await;
        });

        Ok(result_rx)
    }

    async fn close(&self) -> Result<()> {
        self.ws.close().await;
        Ok(())
    }

    fn is_valid(&self) -> bool {
        xiaozhi_core::dashscope_http_api_key_issue(&self.cfg.api_key).is_none()
    }
}
