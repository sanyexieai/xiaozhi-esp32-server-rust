//! 豆包 / 火山引擎 V3 SAUC 语音识别（WebSocket 流式）

use async_trait::async_trait;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::{SinkExt, StreamExt};
use std::io::Write;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};
use xiaozhi_core::{Error, Result};

use crate::traits::{AsrProvider, StreamingResult};
use crate::ws_session::{ReusableWsSession, TaskSessionOutcome, WsConnParts, WsStream};

const MSG_FULL_CLIENT: u8 = 0x1;
const MSG_AUDIO_ONLY: u8 = 0x2;
const MSG_FULL_SERVER: u8 = 0x9;
const MSG_ERROR: u8 = 0xF;
const FLAG_LAST: u8 = 0x2;

#[derive(Debug, Clone)]
struct ParsedDoubaoMsg {
    text: String,
    is_last: bool,
    error: Option<String>,
}

pub struct DoubaoAsrProvider {
    appid: String,
    access_token: String,
    resource_id: String,
    ws_url: String,
    model_name: String,
    sample_rate: u32,
    enable_punc: bool,
    enable_itn: bool,
    connect_id: String,
    ws: ReusableWsSession,
}

impl DoubaoAsrProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let ws_url = config
            .get("ws_url")
            .and_then(|v| v.as_str())
            .map(xiaozhi_core::normalize_doubao_asr_ws_url)
            .unwrap_or_else(|| {
                "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async".to_string()
            });
        Ok(Self {
            appid: xiaozhi_core::trimmed_config_string(config, "appid"),
            access_token: xiaozhi_core::trimmed_config_string(config, "access_token"),
            resource_id: xiaozhi_core::trimmed_config_string(config, "resource_id"),
            ws_url,
            model_name: config
                .get("model_name")
                .and_then(|v| v.as_str())
                .unwrap_or("bigmodel")
                .to_string(),
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(16000) as u32,
            enable_punc: config
                .get("enable_punc")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            enable_itn: config
                .get("enable_itn")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            connect_id: uuid::Uuid::new_v4().to_string(),
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

    fn encode_frame(msg_type: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x11, (msg_type << 4) | flags, 0x11, 0x00];
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    fn gzip_json(v: &serde_json::Value) -> Result<Vec<u8>> {
        let raw = serde_json::to_vec(v).map_err(|e| Error::Http(format!("JSON 序列化失败: {e}")))?;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw)
            .map_err(|e| Error::Http(format!("gzip 压缩失败: {e}")))?;
        enc.finish()
            .map_err(|e| Error::Http(format!("gzip 完成失败: {e}")))
    }

    fn decode_frame(data: &[u8]) -> Result<(u8, u8, Vec<u8>)> {
        if data.len() < 8 {
            return Err(Error::Http("豆包 ASR 帧过短".into()));
        }
        let msg_type = (data[1] >> 4) & 0x0F;
        let flags = data[1] & 0x0F;
        let payload_len = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
        if data.len() < 8 + payload_len {
            return Err(Error::Http("豆包 ASR 帧长度不匹配".into()));
        }
        let payload = data[8..8 + payload_len].to_vec();
        Ok((msg_type, flags, payload))
    }

    fn parse_server_message(data: &[u8]) -> Result<Option<ParsedDoubaoMsg>> {
        let (msg_type, flags, payload) = Self::decode_frame(data)?;
        let is_last = flags & FLAG_LAST != 0;
        match msg_type {
            MSG_FULL_SERVER => {
                let body = Self::gunzip_payload(&payload).unwrap_or(payload);
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                    let text = extract_doubao_text(&v)
                        .or_else(|| extract_first_utterance(&v))
                        .unwrap_or_default();
                    return Ok(Some(ParsedDoubaoMsg {
                        text,
                        is_last,
                        error: None,
                    }));
                }
                Ok(None)
            }
            MSG_ERROR => {
                let err = String::from_utf8_lossy(&payload).to_string();
                Ok(Some(ParsedDoubaoMsg {
                    text: String::new(),
                    is_last: true,
                    error: Some(err),
                }))
            }
            _ => Ok(None),
        }
    }

    async fn send_init_frame(&self, parts: &mut WsConnParts) -> Result<()> {
        let init = serde_json::json!({
            "user": { "uid": uuid::Uuid::new_v4().to_string() },
            "audio": {
                "format": "pcm",
                "rate": self.sample_rate,
                "bits": 16,
                "channel": 1
            },
            "request": {
                "model_name": self.model_name,
                "enable_punc": self.enable_punc,
                "enable_itn": self.enable_itn,
                "result_type": "full",
                "show_utterances": true
            }
        });
        let init_payload = Self::gzip_json(&init)?;
        parts
            .write
            .send(Message::Binary(
                Self::encode_frame(MSG_FULL_CLIENT, 0, &init_payload).into(),
            ))
            .await
            .map_err(|e| Error::Http(format!("豆包 ASR 发送 init 失败: {e}")))
    }

    async fn send_audio_frame(
        parts: &mut WsConnParts,
        pcm: &[f32],
        is_last: bool,
    ) -> Result<()> {
        let audio = Self::float_to_i16_bytes(pcm);
        let audio_payload = {
            let mut enc = GzEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&audio)
                .map_err(|e| Error::Http(format!("音频 gzip 失败: {e}")))?;
            enc.finish()
                .map_err(|e| Error::Http(format!("音频 gzip 完成失败: {e}")))?
        };
        let flags = if is_last { FLAG_LAST } else { 0 };
        parts
            .write
            .send(Message::Binary(
                Self::encode_frame(MSG_AUDIO_ONLY, flags, &audio_payload).into(),
            ))
            .await
            .map_err(|e| Error::Http(format!("豆包 ASR 发送音频失败: {e}")))
    }

    async fn run_streaming_task(
        mut parts: WsConnParts,
        mut audio_rx: mpsc::Receiver<Vec<f32>>,
        this: DoubaoAsrProvider,
        result_tx: mpsc::Sender<StreamingResult>,
    ) -> TaskSessionOutcome {
        let mut init_sent = false;
        let mut audio_done = false;
        let mut last_non_empty = String::new();
        let mut last_partial = String::new();

        loop {
            tokio::select! {
                chunk = audio_rx.recv(), if !audio_done => {
                    match chunk {
                        Some(pcm) => {
                            if !init_sent {
                                if this.send_init_frame(&mut parts).await.is_err() {
                                    return TaskSessionOutcome::Invalidate;
                                }
                                init_sent = true;
                            }
                            if !pcm.is_empty() {
                                if Self::send_audio_frame(&mut parts, &pcm, false).await.is_err() {
                                    return TaskSessionOutcome::Invalidate;
                                }
                            }
                        }
                        None => {
                            if !init_sent {
                                return TaskSessionOutcome::Reuse(parts);
                            }
                            audio_done = true;
                            if Self::send_audio_frame(&mut parts, &[], true).await.is_err() {
                                return TaskSessionOutcome::Invalidate;
                            }
                        }
                    }
                }
                msg = parts.read.next() => {
                    let Some(msg) = msg else {
                        let _ = result_tx.send(StreamingResult {
                            text: last_non_empty.clone(),
                            is_final: true,
                            confidence: None,
                            error: None,
                        }).await;
                        return TaskSessionOutcome::Reuse(parts);
                    };
                    let Ok(msg) = msg else {
                        return TaskSessionOutcome::Invalidate;
                    };
                    if let Message::Binary(data) = msg {
                        match Self::parse_server_message(&data) {
                            Ok(Some(parsed)) => {
                                if let Some(err) = parsed.error {
                                    let _ = result_tx.send(StreamingResult {
                                        text: String::new(),
                                        is_final: true,
                                        confidence: None,
                                        error: Some(err),
                                    }).await;
                                    return TaskSessionOutcome::Invalidate;
                                }
                                let candidate = if parsed.text.is_empty() {
                                    last_non_empty.clone()
                                } else {
                                    last_non_empty = parsed.text.clone();
                                    parsed.text.clone()
                                };
                                if parsed.is_last {
                                    let final_text = if candidate.is_empty() {
                                        last_non_empty.clone()
                                    } else {
                                        candidate
                                    };
                                    let _ = result_tx.send(StreamingResult {
                                        text: final_text,
                                        is_final: true,
                                        confidence: None,
                                        error: None,
                                    }).await;
                                    return TaskSessionOutcome::Reuse(parts);
                                } else if !candidate.is_empty() && candidate != last_partial {
                                    last_partial = candidate.clone();
                                    let _ = result_tx.send(StreamingResult {
                                        text: candidate,
                                        is_final: false,
                                        confidence: None,
                                        error: None,
                                    }).await;
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                let _ = result_tx.send(StreamingResult {
                                    text: String::new(),
                                    is_final: true,
                                    confidence: None,
                                    error: Some(e.to_string()),
                                }).await;
                                return TaskSessionOutcome::Invalidate;
                            }
                        }
                    } else if matches!(msg, Message::Close(_)) {
                        let _ = result_tx.send(StreamingResult {
                            text: last_non_empty,
                            is_final: true,
                            confidence: None,
                            error: None,
                        }).await;
                        return TaskSessionOutcome::Reuse(parts);
                    }
                    if audio_done {
                        continue;
                    }
                }
            }
        }
    }

    fn gunzip_payload(payload: &[u8]) -> Result<Vec<u8>> {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut dec = GzDecoder::new(payload);
        let mut out = Vec::new();
        dec.read_to_end(&mut out)
            .map_err(|e| Error::Http(format!("gzip 解压失败: {e}")))?;
        Ok(out)
    }

    fn build_ws_request(&self, resource_id: &str) -> Result<tokio_tungstenite::tungstenite::http::Request<()>> {
        let mut request = self
            .ws_url
            .as_str()
            .into_client_request()
            .map_err(|e| Error::Http(format!("豆包 WS 请求构建失败: {e}")))?;
        {
            let headers = request.headers_mut();
            headers.insert(
                "X-Api-App-Key",
                HeaderValue::from_str(&self.appid)
                    .map_err(|e| Error::Http(format!("无效 appid: {e}")))?,
            );
            headers.insert(
                "X-Api-Access-Key",
                HeaderValue::from_str(&self.access_token)
                    .map_err(|e| Error::Http(format!("无效 access_token: {e}")))?,
            );
            headers.insert(
                "X-Api-Resource-Id",
                HeaderValue::from_str(resource_id)
                    .map_err(|e| Error::Http(format!("无效 resource_id: {e}")))?,
            );
            headers.insert(
                "X-Api-Connect-Id",
                HeaderValue::from_str(&self.connect_id)
                    .map_err(|e| Error::Http(format!("无效 connect_id: {e}")))?,
            );
            headers.insert(
                "X-Api-Request-Id",
                HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
                    .map_err(|e| Error::Http(format!("无效 request_id: {e}")))?,
            );
        }
        Ok(request)
    }

    async fn connect_ws(&self, resource_id: &str) -> Result<WsStream> {
        let request = self.build_ws_request(resource_id)?;
        let (ws, _) = connect_async(request)
            .await
            .map_err(|e| Error::Http(format!("豆包 ASR WS 连接失败: {e}")))?;
        Ok(ws)
    }

    async fn recognize_on_parts(
        &self,
        parts: WsConnParts,
        pcm: &[f32],
    ) -> Result<(String, TaskSessionOutcome)> {
        let mut parts = parts;
        self.send_init_frame(&mut parts).await?;
        Self::send_audio_frame(&mut parts, pcm, true).await?;

        let mut text = String::new();
        while let Some(msg) = parts.read.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => return Err(Error::Http(format!("豆包 ASR WS 接收失败: {e}"))),
            };
            if let Message::Binary(data) = msg {
                if let Ok(Some(parsed)) = Self::parse_server_message(&data) {
                    if let Some(err) = parsed.error {
                        return Err(Error::Http(format!("豆包 ASR 错误: {err}")));
                    }
                    if !parsed.text.is_empty() {
                        text = parsed.text;
                    }
                    if parsed.is_last {
                        break;
                    }
                }
            } else if matches!(msg, Message::Close(_)) {
                break;
            }
        }

        Ok((text, TaskSessionOutcome::Reuse(parts)))
    }

    async fn recognize_pcm(&self, pcm: &[f32]) -> Result<String> {
        if pcm.is_empty() {
            return Ok(String::new());
        }
        if self.appid.is_empty() || self.access_token.is_empty() {
            return Err(Error::Config("豆包 ASR 缺少 appid/access_token".into()));
        }

        let resource_id = if self.resource_id.is_empty() {
            self.model_name.clone()
        } else {
            self.resource_id.clone()
        };

        let task_guard = self.ws.acquire_task().await;
        let conn_mu = self.ws.conn_mu();
        let this = self.clone_for_task();

        let parts =
            ReusableWsSession::take_or_connect(&conn_mu, || this.connect_ws(&resource_id)).await?;
        let (text, outcome) = self.recognize_on_parts(parts, pcm).await?;
        ReusableWsSession::finish_task(&conn_mu, task_guard, outcome).await;
        Ok(text)
    }

    fn clone_for_task(&self) -> Self {
        Self {
            appid: self.appid.clone(),
            access_token: self.access_token.clone(),
            resource_id: self.resource_id.clone(),
            ws_url: self.ws_url.clone(),
            model_name: self.model_name.clone(),
            sample_rate: self.sample_rate,
            enable_punc: self.enable_punc,
            enable_itn: self.enable_itn,
            connect_id: self.connect_id.clone(),
            ws: self.ws.clone(),
        }
    }
}

fn extract_first_utterance(v: &serde_json::Value) -> Option<String> {
    v.pointer("/result/utterances")
        .and_then(|arr| arr.as_array())
        .and_then(|items| {
            items.iter().find_map(|u| {
                u.get("text")
                    .and_then(|t| t.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from)
            })
        })
}

fn extract_doubao_text(v: &serde_json::Value) -> Option<String> {
    for path in [
        "/result/text",
        "/result/utterances/0/text",
        "/payload_msg/result/text",
    ] {
        if let Some(t) = v.pointer(path).and_then(|x| x.as_str()) {
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    v.get("text")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[async_trait]
impl AsrProvider for DoubaoAsrProvider {
    async fn process(&self, pcm_data: &[f32]) -> Result<String> {
        self.recognize_pcm(pcm_data).await
    }

    async fn streaming_recognize(
        &self,
        audio_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<mpsc::Receiver<StreamingResult>> {
        if self.appid.is_empty() || self.access_token.is_empty() {
            return Err(Error::Config("豆包 ASR 缺少 appid/access_token".into()));
        }
        let resource_id = if self.resource_id.is_empty() {
            self.model_name.clone()
        } else {
            self.resource_id.clone()
        };

        let task_guard = self.ws.acquire_task().await;
        let conn_mu = self.ws.conn_mu();
        let this = self.clone_for_task();
        let (result_tx, result_rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let parts =
                match ReusableWsSession::take_or_connect(&conn_mu, || this.connect_ws(&resource_id))
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = result_tx
                            .send(StreamingResult {
                                text: String::new(),
                                is_final: true,
                                confidence: None,
                                error: Some(e.to_string()),
                            })
                            .await;
                        ReusableWsSession::finish_task(
                            &conn_mu,
                            task_guard,
                            TaskSessionOutcome::Invalidate,
                        )
                        .await;
                        return;
                    }
                };
            let outcome =
                DoubaoAsrProvider::run_streaming_task(parts, audio_rx, this, result_tx).await;
            ReusableWsSession::finish_task(&conn_mu, task_guard, outcome).await;
        });

        Ok(result_rx)
    }

    async fn close(&self) -> Result<()> {
        self.ws.close().await;
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.appid.is_empty() && !self.access_token.is_empty()
    }
}
