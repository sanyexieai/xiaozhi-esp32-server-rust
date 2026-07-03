//! 阿里云 FunASR - DashScope WebSocket 流式识别（与 Go `aliyun_funasr` 对齐）

use async_trait::async_trait;
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
use uuid::Uuid;
use xiaozhi_core::{Error, Result};

use crate::traits::{AsrProvider, StreamingResult};
use crate::ws_session::{ReusableWsSession, TaskSessionOutcome, WsConnParts, WsRead, WsStream};

struct AliyunFunAsrConfig {
    api_key: String,
    ws_url: String,
    model: String,
    format: String,
    sample_rate: u32,
    language_hints: Vec<String>,
    vocabulary_id: String,
    disfluency_removal_enabled: bool,
    semantic_punctuation_enabled: bool,
    timeout_secs: u64,
}

pub struct AliyunFunAsrProvider {
    cfg: AliyunFunAsrConfig,
    ws: ReusableWsSession,
}

impl AliyunFunAsrProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let language_hints = parse_language_hints(config.get("language_hints"))
            .or_else(|| {
                config
                    .get("language")
                    .and_then(|v| v.as_str())
                    .map(|s| vec![s.to_string()])
            })
            .unwrap_or_else(|| vec!["zh".to_string()]);

        let api_key = xiaozhi_core::dashscope_api_key(config);

        Ok(Self {
            cfg: AliyunFunAsrConfig {
                api_key,
                ws_url: config
                    .get("ws_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("wss://dashscope.aliyuncs.com/api-ws/v1/inference/")
                    .to_string(),
                model: config
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("fun-asr-realtime")
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
                language_hints,
                vocabulary_id: config
                    .get("vocabulary_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                disfluency_removal_enabled: config
                    .get("disfluency_removal_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                semantic_punctuation_enabled: config
                    .get("semantic_punctuation_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                timeout_secs: config
                    .get("timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30),
            },
            ws: ReusableWsSession::new(),
        })
    }

    fn float_to_i16_bytes(pcm: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &sample in pcm {
            let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        bytes
    }

    fn build_run_task(task_id: &str, cfg: &AliyunFunAsrConfig) -> serde_json::Value {
        serde_json::json!({
            "header": {
                "action": "run-task",
                "task_id": task_id,
                "streaming": "duplex"
            },
            "payload": {
                "task_group": "audio",
                "task": "asr",
                "function": "recognition",
                "model": cfg.model,
                "parameters": {
                    "format": cfg.format,
                    "sample_rate": cfg.sample_rate,
                    "language_hints": cfg.language_hints,
                    "vocabulary_id": cfg.vocabulary_id,
                    "disfluency_removal_enabled": cfg.disfluency_removal_enabled,
                    "semantic_punctuation_enabled": cfg.semantic_punctuation_enabled,
                },
                "input": {}
            }
        })
    }

    fn build_finish_task(task_id: &str) -> serde_json::Value {
        serde_json::json!({
            "header": {
                "action": "finish-task",
                "task_id": task_id,
                "streaming": "duplex"
            },
            "payload": {
                "input": {}
            }
        })
    }

    fn wait_timeout(cfg: &AliyunFunAsrConfig) -> Duration {
        let secs = cfg.timeout_secs;
        if secs == 0 {
            Duration::from_secs(10)
        } else {
            Duration::from_secs(secs.max(10))
        }
    }

    async fn connect_ws(cfg: &AliyunFunAsrConfig) -> Result<WsStream> {
        if cfg.api_key.is_empty() {
            return Err(Error::Config(
                "阿里云 ASR api_key 未配置（或设置 DASHSCOPE_API_KEY）".into(),
            ));
        }
        let mut request = cfg
            .ws_url
            .as_str()
            .into_client_request()
            .map_err(|e| Error::WebSocket(format!("构建 WebSocket 请求失败: {e}")))?;
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("bearer {}", cfg.api_key))
                .map_err(|e| Error::WebSocket(format!("Authorization 头无效: {e}")))?,
        );
        let timeout = Duration::from_secs(cfg.timeout_secs.max(5));
        let (ws, _) = tokio::time::timeout(timeout, connect_async(request))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|e| Error::WebSocket(format!("阿里云 FunASR 连接失败: {e}")))?;
        Ok(ws)
    }

    async fn send_partial_result(tx: &mpsc::Sender<StreamingResult>, text: String) {
        let _ = tx
            .try_send(StreamingResult {
                text,
                is_final: false,
                confidence: None,
                error: None,
            })
            .ok();
    }

    async fn send_final_result(tx: &mpsc::Sender<StreamingResult>, result: StreamingResult) {
        let _ = tx.send(result).await;
    }

    async fn wait_task_started(read: &mut WsRead, cfg: &AliyunFunAsrConfig) -> Result<()> {
        let wait_started = Self::wait_timeout(cfg);
        let wait_secs = wait_started.as_secs();
        match tokio::time::timeout(wait_started, async {
            while let Some(msg) = read.next().await {
                let msg = msg.map_err(|e| Error::WebSocket(format!("读取消息失败: {e}")))?;
                if let Message::Text(text) = msg {
                    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    match v.pointer("/header/event").and_then(|x| x.as_str()) {
                        Some("task-started") => return Ok(()),
                        Some("task-failed") => {
                            let err = v
                                .pointer("/header/error_message")
                                .and_then(|x| x.as_str())
                                .unwrap_or("task failed");
                            return Err(Error::WebSocket(format!(
                                "阿里云 FunASR task-failed: {err}"
                            )));
                        }
                        _ => {}
                    }
                }
            }
            Err(Error::WebSocket("等待 task-started 时连接关闭".into()))
        })
        .await
        {
            Ok(inner) => inner,
            Err(_) => Err(Error::WebSocket(format!(
                "wait task-started timeout after {wait_secs}s"
            ))),
        }
    }

    async fn run_streaming_task(
        mut parts: WsConnParts,
        mut audio_rx: mpsc::Receiver<Vec<f32>>,
        task_id: String,
        cfg: AliyunFunAsrConfig,
        result_tx: mpsc::Sender<StreamingResult>,
    ) -> TaskSessionOutcome {
        if let Err(e) = Self::wait_task_started(&mut parts.read, &cfg).await {
            tracing::warn!("阿里云 FunASR 识别失败: {e}");
            Self::send_final_result(
                &result_tx,
                StreamingResult {
                    text: String::new(),
                    is_final: true,
                    confidence: None,
                    error: Some(e.to_string()),
                },
            )
            .await;
            return TaskSessionOutcome::Invalidate;
        }

        let mut final_text = String::new();
        let mut audio_done = false;

        loop {
            if audio_done {
                let finish_wait = Self::wait_timeout(&cfg);
                let finished = tokio::time::timeout(finish_wait, async {
                    while let Some(msg) = parts.read.next().await {
                        let msg = msg.map_err(|e| {
                            Error::WebSocket(format!("读取识别结果失败: {e}"))
                        })?;
                        if let Message::Text(text) = msg {
                            let v: serde_json::Value =
                                serde_json::from_str(&text).unwrap_or_default();
                            match v.pointer("/header/event").and_then(|x| x.as_str()) {
                                Some("result-generated") => {
                                    if v.pointer("/payload/output/sentence/heartbeat")
                                        .and_then(|x| x.as_bool())
                                        .unwrap_or(false)
                                    {
                                        continue;
                                    }
                                    if let Some(t) = v
                                        .pointer("/payload/output/sentence/text")
                                        .and_then(|x| x.as_str())
                                    {
                                        if !t.is_empty() {
                                            final_text = t.to_string();
                                            Self::send_partial_result(
                                                &result_tx,
                                                t.to_string(),
                                            )
                                            .await;
                                        }
                                    }
                                }
                                Some("task-finished") => return Ok(final_text.clone()),
                                Some("task-failed") => {
                                    let err = v
                                        .pointer("/header/error_message")
                                        .and_then(|x| x.as_str())
                                        .unwrap_or("task failed");
                                    return Err(Error::WebSocket(format!(
                                        "阿里云 FunASR task-failed: {err}"
                                    )));
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(final_text.clone())
                })
                .await;

                return match finished {
                    Ok(Ok(text)) => {
                        Self::send_final_result(
                            &result_tx,
                            StreamingResult {
                                text,
                                is_final: true,
                                confidence: None,
                                error: None,
                            },
                        )
                        .await;
                        TaskSessionOutcome::Reuse(parts)
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("阿里云 FunASR 识别失败: {e}");
                        Self::send_final_result(
                            &result_tx,
                            StreamingResult {
                                text: String::new(),
                                is_final: true,
                                confidence: None,
                                error: Some(e.to_string()),
                            },
                        )
                        .await;
                        TaskSessionOutcome::Invalidate
                    }
                    Err(_) => {
                        Self::send_final_result(
                            &result_tx,
                            StreamingResult {
                                text: final_text,
                                is_final: true,
                                confidence: None,
                                error: Some("wait task-finished timeout".into()),
                            },
                        )
                        .await;
                        TaskSessionOutcome::Invalidate
                    }
                };
            }

            tokio::select! {
                chunk = audio_rx.recv() => {
                    match chunk {
                        Some(pcm) => {
                            let bytes = Self::float_to_i16_bytes(&pcm);
                            if parts.write.send(Message::Binary(bytes.into())).await.is_err() {
                                Self::send_final_result(
                                    &result_tx,
                                    StreamingResult {
                                        text: String::new(),
                                        is_final: true,
                                        confidence: None,
                                        error: Some("send audio failed".into()),
                                    },
                                ).await;
                                return TaskSessionOutcome::Invalidate;
                            }
                        }
                        None => {
                            let finish = Self::build_finish_task(&task_id).to_string();
                            let _ = parts.write.send(Message::Text(finish.into())).await;
                            audio_done = true;
                        }
                    }
                }
                msg = parts.read.next() => {
                    let Some(msg) = msg else {
                        audio_done = true;
                        continue;
                    };
                    match msg {
                        Ok(Message::Text(text)) => {
                            let v: serde_json::Value =
                                serde_json::from_str(&text).unwrap_or_default();
                            match v.pointer("/header/event").and_then(|x| x.as_str()) {
                                Some("result-generated") => {
                                    if v.pointer("/payload/output/sentence/heartbeat")
                                        .and_then(|x| x.as_bool())
                                        .unwrap_or(false)
                                    {
                                        continue;
                                    }
                                    if let Some(t) = v
                                        .pointer("/payload/output/sentence/text")
                                        .and_then(|x| x.as_str())
                                    {
                                        if !t.is_empty() {
                                            final_text = t.to_string();
                                            Self::send_partial_result(
                                                &result_tx,
                                                t.to_string(),
                                            ).await;
                                        }
                                    }
                                }
                                Some("task-finished") => {
                                    Self::send_final_result(
                                        &result_tx,
                                        StreamingResult {
                                            text: final_text.clone(),
                                            is_final: true,
                                            confidence: None,
                                            error: None,
                                        },
                                    ).await;
                                    return TaskSessionOutcome::Reuse(parts);
                                }
                                Some("task-failed") => {
                                    let err = v.pointer("/header/error_message")
                                        .and_then(|x| x.as_str())
                                        .unwrap_or("task failed");
                                    Self::send_final_result(
                                        &result_tx,
                                        StreamingResult {
                                            text: String::new(),
                                            is_final: true,
                                            confidence: None,
                                            error: Some(format!(
                                                "阿里云 FunASR task-failed: {err}"
                                            )),
                                        },
                                    ).await;
                                    return TaskSessionOutcome::Invalidate;
                                }
                                _ => {}
                            }
                        }
                        Err(e) => {
                            Self::send_final_result(
                                &result_tx,
                                StreamingResult {
                                    text: String::new(),
                                    is_final: true,
                                    confidence: None,
                                    error: Some(format!("read message failed: {e}")),
                                },
                            ).await;
                            return TaskSessionOutcome::Invalidate;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn parse_language_hints(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let arr = value?.as_array()?;
    let hints: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if hints.is_empty() {
        None
    } else {
        Some(hints)
    }
}

#[async_trait]
impl AsrProvider for AliyunFunAsrProvider {
    async fn process(&self, pcm_data: &[f32]) -> Result<String> {
        if pcm_data.is_empty() {
            return Ok(String::new());
        }
        let (tx, rx) = mpsc::channel(4);
        tx.send(pcm_data.to_vec()).await.ok();
        drop(tx);
        let text = super::factory::collect_streaming(self, rx).await?;
        if text.trim().is_empty() {
            return Err(Error::Http("阿里云 FunASR 未返回识别文本".into()));
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

        let mut parts =
            ReusableWsSession::take_or_connect(&conn_mu, || Self::connect_ws(&cfg)).await?;
        let task_id = Uuid::new_v4().to_string();

        if parts
            .write
            .send(Message::Text(
                Self::build_run_task(&task_id, &cfg).to_string().into(),
            ))
            .await
            .is_err()
        {
            ReusableWsSession::invalidate_conn(&conn_mu).await;
            drop(task_guard);
            return Err(Error::WebSocket("发送 run-task 失败".into()));
        }

        let (result_tx, result_rx) = mpsc::channel(20);

        tokio::spawn(async move {
            let outcome =
                AliyunFunAsrProvider::run_streaming_task(parts, audio_rx, task_id, cfg, result_tx)
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
        !self.cfg.api_key.is_empty()
    }
}

impl Clone for AliyunFunAsrConfig {
    fn clone(&self) -> Self {
        Self {
            api_key: self.api_key.clone(),
            ws_url: self.ws_url.clone(),
            model: self.model.clone(),
            format: self.format.clone(),
            sample_rate: self.sample_rate,
            language_hints: self.language_hints.clone(),
            vocabulary_id: self.vocabulary_id.clone(),
            disfluency_removal_enabled: self.disfluency_removal_enabled,
            semantic_punctuation_enabled: self.semantic_punctuation_enabled,
            timeout_secs: self.timeout_secs,
        }
    }
}
