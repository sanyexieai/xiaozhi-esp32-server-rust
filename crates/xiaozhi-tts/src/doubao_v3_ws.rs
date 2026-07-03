//! 豆包 TTS V3 WebSocket 单向流式

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};
use xiaozhi_core::{Error, Result};

use crate::doubao::DoubaoV3Config;
use crate::doubao_model::{
    build_doubao_ws_attempt_models, is_doubao_retryable_resource_error, normalize_doubao_voice,
    resolve_doubao_tts_model, ResolvedTtsModel,
};
use crate::volcengine_protocol::{
    extract_audio_from_event, is_stream_end, marshal_doubao_ws_binary_request,
    marshal_finish_connection, parse_message,
};

pub struct DoubaoV3WsClient {
    cfg: Arc<RwLock<DoubaoV3Config>>,
    /// 上次 WS 合成成功的 resource/model，避免每次先打失败的重试请求
    cached_model: Arc<RwLock<Option<ResolvedTtsModel>>>,
}

impl DoubaoV3WsClient {
    pub fn new(cfg: DoubaoV3Config) -> Self {
        Self {
            cfg: Arc::new(RwLock::new(cfg)),
            cached_model: Arc::new(RwLock::new(None)),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.cfg
            .try_read()
            .map(|c| c.is_valid())
            .unwrap_or(false)
    }

    pub async fn set_voice(&self, voice: &str) {
        let mut cfg = self.cfg.write().await;
        cfg.voice = normalize_doubao_voice(voice);
    }

    pub async fn audio_format(&self) -> String {
        self.cfg
            .read()
            .await
            .audio_format
            .trim()
            .to_ascii_lowercase()
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
        *self.cached_model.write().await = None;
    }

    pub async fn synthesize_stream(&self, text: &str) -> Result<mpsc::Receiver<Vec<u8>>> {
        let cfg = self.cfg.read().await.clone();
        if !cfg.is_valid() {
            return Err(Error::Config(
                "豆包 WS TTS 缺少 appid/access_token/voice".into(),
            ));
        }
        if text.trim().is_empty() {
            let (_tx, rx) = mpsc::channel(1);
            return Ok(rx);
        }

        let voice = normalize_doubao_voice(&cfg.voice);
        let derived = resolve_doubao_tts_model(&cfg.model, &voice)?;
        let mut attempts =
            build_doubao_ws_attempt_models(&derived, &cfg.resource_id, &voice);
        if let Some(cached) = self.cached_model.read().await.clone() {
            attempts.retain(|m| {
                m.resource_id != cached.resource_id
                    || m.request_model != cached.request_model
            });
            attempts.insert(0, cached);
        }

        let (tx, rx) = mpsc::channel(32);
        let text = text.to_string();
        let mut last_err: Option<Error> = None;
        let cache = Arc::clone(&self.cached_model);

        for (idx, attempt) in attempts.iter().enumerate() {
            match try_ws_stream(cfg.clone(), text.clone(), attempt.clone()).await {
                Ok(stream_rx) => {
                    *cache.write().await = Some(attempt.clone());
                    forward_audio(stream_rx, tx);
                    return Ok(rx);
                }
                Err(e) => {
                    let retryable = is_doubao_retryable_resource_error(&e.to_string());
                    last_err = Some(e);
                    if idx + 1 < attempts.len() && retryable {
                        tracing::warn!(
                            "豆包 WS TTS 资源族不匹配，尝试切换: voice={} resource_id={} -> {}",
                            voice,
                            attempt.resource_id,
                            attempts[idx + 1].resource_id
                        );
                        continue;
                    }
                    return Err(last_err.unwrap());
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            Error::Http("豆包 WS TTS 未找到可用的资源族".into())
        }))
    }

    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        let mut rx = self.synthesize_stream(text).await?;
        let mut audio = Vec::new();
        while let Some(chunk) = rx.recv().await {
            audio.extend(chunk);
        }
        if audio.is_empty() {
            return Err(Error::Http(
                "豆包 WebSocket TTS 返回音频为空，请检查 appid/access_token/voice/resource_id"
                    .into(),
            ));
        }
        Ok(audio)
    }
}

async fn try_ws_stream(
    cfg: DoubaoV3Config,
    text: String,
    resolved: ResolvedTtsModel,
) -> Result<mpsc::Receiver<Vec<u8>>> {
    let ws_url = if cfg.ws_url.is_empty() {
        "wss://openspeech.bytedance.com/api/v3/tts/unidirectional/stream".to_string()
    } else {
        cfg.ws_url.clone()
    };

    let mut request = ws_url
        .as_str()
        .into_client_request()
        .map_err(|e| Error::Http(format!("豆包 WS 请求构建失败: {e}")))?;
    {
        let headers = request.headers_mut();
        headers.insert(
            "X-Api-App-Key",
            HeaderValue::from_str(&cfg.appid)
                .map_err(|e| Error::Http(format!("无效 appid: {e}")))?,
        );
        headers.insert(
            "X-Api-Access-Key",
            HeaderValue::from_str(&cfg.access_token)
                .map_err(|e| Error::Http(format!("无效 access_token: {e}")))?,
        );
        headers.insert(
            "X-Api-Resource-Id",
            HeaderValue::from_str(&resolved.resource_id)
                .map_err(|e| Error::Http(format!("无效 resource_id: {e}")))?,
        );
        headers.insert(
            "X-Api-Connect-Id",
            HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
                .map_err(|e| Error::Http(format!("无效 connect_id: {e}")))?,
        );
    }

    let (ws, _) = connect_async(request)
        .await
        .map_err(|e| Error::Http(format!("豆包 WS TTS 连接失败: {e}")))?;
    let (mut write, read) = ws.split();

    let speaker = normalize_doubao_voice(&cfg.voice);
    let mut body = serde_json::json!({
        "user": { "uid": uuid::Uuid::new_v4().to_string() },
        "req_params": {
            "text": text,
            "speaker": speaker,
            "audio_params": {
                "format": cfg.audio_format,
                "sample_rate": cfg.sample_rate,
            }
        }
    });
    if !resolved.request_model.is_empty() {
        body["req_params"]["model"] = serde_json::json!(resolved.request_model);
    }

    let payload = serde_json::to_vec(&body)
        .map_err(|e| Error::Http(format!("豆包 WS JSON 序列化失败: {e}")))?;
    let frame = marshal_doubao_ws_binary_request(&payload)?;
    write
        .send(Message::Binary(frame.into()))
        .await
        .map_err(|e| Error::Http(format!("豆包 WS 发送失败: {e}")))?;

    let (stream_tx, stream_rx) = mpsc::channel(32);
    let (attempt_tx, attempt_rx) = oneshot::channel();

    tokio::spawn(async move {
        let _ = pump_ws_stream(read, write, stream_tx, attempt_tx).await;
    });

    match attempt_rx.await {
        Ok(Ok(())) => Ok(stream_rx),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(Error::Http("豆包 WS TTS attempt 已取消".into())),
    }
}

fn forward_audio(mut src: mpsc::Receiver<Vec<u8>>, dst: mpsc::Sender<Vec<u8>>) {
    tokio::spawn(async move {
        while let Some(chunk) = src.recv().await {
            if dst.send(chunk).await.is_err() {
                break;
            }
        }
    });
}

async fn pump_ws_stream(
    mut read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    mut write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    tx: mpsc::Sender<Vec<u8>>,
    attempt_tx: oneshot::Sender<Result<()>>,
) -> Result<()> {
    let mut got_audio = false;
    let mut attempt_tx = Some(attempt_tx);

    let signal_attempt = |tx: &mut Option<oneshot::Sender<Result<()>>>, result: Result<()>| {
        if let Some(sender) = tx.take() {
            let _ = sender.send(result);
        }
    };

    loop {
        let msg = match read.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                let msg = format!("豆包 WS 接收失败: {e}");
                signal_attempt(&mut attempt_tx, Err(Error::Http(msg.clone())));
                return Err(Error::Http(msg));
            }
            None => break,
        };

        let data = match msg {
            Message::Binary(d) => d.to_vec(),
            Message::Text(s) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    if let Some(bytes) = extract_audio_from_json(&v) {
                        got_audio = true;
                        signal_attempt(&mut attempt_tx, Ok(()));
                        let _ = tx.send(bytes).await;
                    }
                }
                continue;
            }
            Message::Close(_) => break,
            _ => continue,
        };

        let parsed = match parse_message(&data) {
            Ok(p) => p,
            Err(e) => {
                signal_attempt(&mut attempt_tx, Err(Error::Http(e.to_string())));
                return Err(e);
            }
        };

        if parsed.msg_type == 0xF {
            let err_body = String::from_utf8_lossy(&parsed.payload).into_owned();
            let msg = format!("豆包 WS TTS 错误 code={}: {err_body}", parsed.error_code);
            signal_attempt(&mut attempt_tx, Err(Error::Http(msg.clone())));
            return Err(Error::Http(msg));
        }

        if let Some(audio) = extract_audio_from_event(&parsed) {
            if !audio.is_empty() {
                got_audio = true;
                signal_attempt(&mut attempt_tx, Ok(()));
                let _ = tx.send(audio).await;
            }
        }

        if is_stream_end(&parsed) {
            break;
        }
    }

    let _ = write
        .send(Message::Binary(marshal_finish_connection().into()))
        .await;

    if !got_audio {
        let msg = "豆包 WS TTS 未返回音频数据，请检查密钥、voice 与 resource_id 是否匹配火山控制台授权".to_string();
        signal_attempt(&mut attempt_tx, Err(Error::Http(msg.clone())));
        return Err(Error::Http(msg));
    }
    Ok(())
}

fn extract_audio_from_json(v: &serde_json::Value) -> Option<Vec<u8>> {
    use base64::Engine;
    for key in ["data", "audio", "audio_data"] {
        if let Some(b64) = v.get(key).and_then(|x| x.as_str()) {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                return Some(bytes);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volcengine_protocol::marshal_full_client;

    #[test]
    fn parse_full_client_roundtrip() {
        let payload = br#"{"text":"hi"}"#;
        let frame = marshal_full_client(payload);
        let msg = parse_message(&frame).unwrap();
        assert_eq!(msg.msg_type, 1);
        assert_eq!(msg.payload, payload);
    }
}
