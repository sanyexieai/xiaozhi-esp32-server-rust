//! 讯飞 IAT v2 WebSocket 语音识别

use async_trait::async_trait;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use xiaozhi_core::{Error, Result};

use crate::traits::{AsrProvider, StreamingResult};
use crate::ws_session::{ReusableWsSession, TaskSessionOutcome, WsConnParts, WsWrite};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug)]
struct ParsedXunfeiMsg {
    segment: String,
    data_status: Option<i64>,
    error: Option<String>,
}

pub struct XunfeiAsrProvider {
    appid: String,
    api_key: String,
    api_secret: String,
    host: String,
    path: String,
    domain: String,
    language: String,
    accent: String,
    sample_rate: u32,
    /// 对齐 Go：讯飞每次识别独立 WS（签名 URL），此处仅用于任务串行
    ws: ReusableWsSession,
}

impl XunfeiAsrProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        Ok(Self {
            appid: xiaozhi_core::trimmed_config_string(config, "appid"),
            api_key: xiaozhi_core::trimmed_config_string(config, "api_key"),
            api_secret: xiaozhi_core::trimmed_config_string(config, "api_secret"),
            host: config
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("iat-api.xfyun.cn")
                .to_string(),
            path: config
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("/v2/iat")
                .to_string(),
            domain: config
                .get("domain")
                .and_then(|v| v.as_str())
                .unwrap_or("iat")
                .to_string(),
            language: config
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("zh_cn")
                .to_string(),
            accent: config
                .get("accent")
                .and_then(|v| v.as_str())
                .unwrap_or("mandarin")
                .to_string(),
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(16000) as u32,
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

    fn build_ws_url(&self) -> Result<String> {
        if self.api_key.is_empty() || self.api_secret.is_empty() {
            return Err(Error::Config("讯飞 ASR 缺少 api_key/api_secret".into()));
        }
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

    async fn connect_ws(&self) -> Result<WsConnParts> {
        let ws_url = self.build_ws_url()?;
        let mut request = ws_url
            .into_client_request()
            .map_err(|e| Error::Http(format!("讯飞 WS 请求构建失败: {e}")))?;
        request
            .headers_mut()
            .insert("Host", self.host.parse().unwrap());

        let (ws, _) = connect_async(request)
            .await
            .map_err(|e| Error::Http(format!("讯飞 WS 连接失败: {e}")))?;
        Ok(WsConnParts::split(ws))
    }

    async fn send_frame(
        write: &mut WsWrite,
        provider: &XunfeiAsrProvider,
        audio_bytes: &[u8],
        status: i32,
    ) -> Result<()> {
        let audio_b64 = base64::engine::general_purpose::STANDARD.encode(audio_bytes);
        let frame = if status == 0 {
            serde_json::json!({
                "common": { "app_id": provider.appid },
                "business": {
                    "language": provider.language,
                    "domain": provider.domain,
                    "accent": provider.accent,
                    "vad_eos": 10000,
                    "dwa": "wpgs"
                },
                "data": {
                    "status": status,
                    "format": format!("audio/L16;rate={}", provider.sample_rate),
                    "encoding": "raw",
                    "audio": audio_b64
                }
            })
        } else {
            serde_json::json!({
                "data": {
                    "status": status,
                    "format": format!("audio/L16;rate={}", provider.sample_rate),
                    "encoding": "raw",
                    "audio": audio_b64
                }
            })
        };
        write
            .send(Message::Text(frame.to_string().into()))
            .await
            .map_err(|e| Error::Http(format!("讯飞 WS 发送失败: {e}")))
    }

    fn parse_response_text(s: &str) -> Result<Option<ParsedXunfeiMsg>> {
        let v: serde_json::Value =
            serde_json::from_str(s).map_err(|e| Error::Http(format!("讯飞响应 JSON 解析失败: {e}")))?;
        if let Some(code) = v.get("code").and_then(|c| c.as_i64()) {
            if code != 0 {
                let msg = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("未知错误");
                return Ok(Some(ParsedXunfeiMsg {
                    segment: String::new(),
                    data_status: None,
                    error: Some(format!("讯飞 ASR 错误 {code}: {msg}")),
                }));
            }
        }
        let segment = v
            .pointer("/data/result")
            .map(parse_xunfei_result)
            .unwrap_or_default();
        let data_status = v.pointer("/data/status").and_then(|s| s.as_i64());
        Ok(Some(ParsedXunfeiMsg {
            segment,
            data_status,
            error: None,
        }))
    }

    async fn run_streaming_task(
        mut parts: WsConnParts,
        mut audio_rx: mpsc::Receiver<Vec<f32>>,
        this: XunfeiAsrProvider,
        result_tx: mpsc::Sender<StreamingResult>,
    ) -> TaskSessionOutcome {
        let mut send_status = 0i32;
        let mut audio_done = false;
        let mut result_builder = String::new();
        let mut last_partial = String::new();

        loop {
            tokio::select! {
                chunk = audio_rx.recv(), if !audio_done => {
                    match chunk {
                        Some(pcm) => {
                            let audio_bytes = Self::float_to_i16_bytes(&pcm);
                            if Self::send_frame(&mut parts.write, &this, &audio_bytes, send_status).await.is_err() {
                                return TaskSessionOutcome::Invalidate;
                            }
                            if send_status == 0 {
                                send_status = 1;
                            }
                        }
                        None => {
                            audio_done = true;
                            if Self::send_frame(&mut parts.write, &this, &[], 2).await.is_err() {
                                return TaskSessionOutcome::Invalidate;
                            }
                        }
                    }
                }
                msg = parts.read.next() => {
                    let Some(msg) = msg else {
                        let _ = result_tx.send(StreamingResult {
                            text: result_builder.clone(),
                            is_final: true,
                            confidence: None,
                            error: None,
                        }).await;
                        return TaskSessionOutcome::Invalidate;
                    };
                    let Ok(msg) = msg else {
                        return TaskSessionOutcome::Invalidate;
                    };
                    if let Message::Text(s) = msg {
                        match Self::parse_response_text(&s) {
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
                                if !parsed.segment.is_empty() {
                                    result_builder.push_str(&parsed.segment);
                                    if result_builder != last_partial {
                                        last_partial = result_builder.clone();
                                        let _ = result_tx.send(StreamingResult {
                                            text: result_builder.clone(),
                                            is_final: false,
                                            confidence: None,
                                            error: None,
                                        }).await;
                                    }
                                }
                                if parsed.data_status == Some(2) {
                                    let _ = result_tx.send(StreamingResult {
                                        text: result_builder.clone(),
                                        is_final: true,
                                        confidence: None,
                                        error: None,
                                    }).await;
                                    return TaskSessionOutcome::Invalidate;
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
                            text: result_builder,
                            is_final: true,
                            confidence: None,
                            error: None,
                        }).await;
                        return TaskSessionOutcome::Invalidate;
                    }
                }
            }
        }
    }

    async fn recognize_pcm(&self, pcm: &[f32]) -> Result<String> {
        if pcm.is_empty() {
            return Ok(String::new());
        }
        if self.appid.is_empty() {
            return Err(Error::Config("讯飞 ASR appid 未配置".into()));
        }

        let audio_bytes = Self::float_to_i16_bytes(pcm);
        let mut parts = self.connect_ws().await?;

        const CHUNK: usize = 1280;
        let mut offset = 0;
        let mut frame_idx = 0;
        while offset <= audio_bytes.len() {
            let end = (offset + CHUNK).min(audio_bytes.len());
            let chunk = &audio_bytes[offset..end];
            let status = if frame_idx == 0 {
                0
            } else if end >= audio_bytes.len() {
                2
            } else {
                1
            };
            Self::send_frame(&mut parts.write, self, chunk, status).await?;
            if status == 2 {
                break;
            }
            offset = end;
            frame_idx += 1;
        }

        let mut text = String::new();
        while let Some(msg) = parts.read.next().await {
            let msg = msg.map_err(|e| Error::Http(format!("讯飞 WS 接收失败: {e}")))?;
            if let Message::Text(s) = msg {
                if let Ok(Some(parsed)) = Self::parse_response_text(&s) {
                    if let Some(err) = parsed.error {
                        return Err(Error::Http(err));
                    }
                    if !parsed.segment.is_empty() {
                        text.push_str(&parsed.segment);
                    }
                    if parsed.data_status == Some(2) {
                        break;
                    }
                }
            }
        }
        Ok(text)
    }
}

fn parse_xunfei_result(result: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(ws) = result.get("ws").and_then(|w| w.as_array()) {
        for item in ws {
            if let Some(cw) = item.get("cw").and_then(|c| c.as_array()) {
                for word in cw {
                    if let Some(w) = word.get("w").and_then(|x| x.as_str()) {
                        out.push_str(w);
                    }
                }
            }
        }
    }
    out
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
impl AsrProvider for XunfeiAsrProvider {
    async fn process(&self, pcm_data: &[f32]) -> Result<String> {
        self.recognize_pcm(pcm_data).await
    }

    async fn streaming_recognize(
        &self,
        audio_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<mpsc::Receiver<StreamingResult>> {
        if self.appid.is_empty() {
            return Err(Error::Config("讯飞 ASR appid 未配置".into()));
        }

        let task_guard = self.ws.acquire_task().await;
        let conn_mu = self.ws.conn_mu();
        let this = self.clone_for_task();
        let (result_tx, result_rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let parts = match this.connect_ws().await {
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
                XunfeiAsrProvider::run_streaming_task(parts, audio_rx, this, result_tx).await;
            ReusableWsSession::finish_task(&conn_mu, task_guard, outcome).await;
        });

        Ok(result_rx)
    }

    async fn close(&self) -> Result<()> {
        self.ws.close().await;
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.appid.is_empty() && !self.api_key.is_empty() && !self.api_secret.is_empty()
    }
}

impl XunfeiAsrProvider {
    fn clone_for_task(&self) -> Self {
        Self {
            appid: self.appid.clone(),
            api_key: self.api_key.clone(),
            api_secret: self.api_secret.clone(),
            host: self.host.clone(),
            path: self.path.clone(),
            domain: self.domain.clone(),
            language: self.language.clone(),
            accent: self.accent.clone(),
            sample_rate: self.sample_rate,
            ws: self.ws.clone(),
        }
    }
}
