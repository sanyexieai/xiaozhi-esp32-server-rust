//! 小智官方 TTS WebSocket（tenclass 等兼容服务端）

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest, tungstenite::Message};
use xiaozhi_core::{Error, Result};

use crate::audio_decoder::wrap_tts_audio_stream;
use crate::traits::TtsProvider;

#[derive(Clone)]
pub struct XiaozhiTtsProvider {
    server_addr: String,
    device_id: String,
    client_id: String,
    token: String,
    device_pool: Vec<String>,
    sample_rate: u32,
    channels: u8,
    frame_duration: u32,
    audio_format: String,
}

impl XiaozhiTtsProvider {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let device_pool = config
            .get("device_ids")
            .or_else(|| config.get("device_id_list"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(Self {
            server_addr: config
                .get("server_addr")
                .and_then(|v| v.as_str())
                .unwrap_or("wss://api.tenclass.net/xiaozhi/v1/")
                .to_string(),
            device_id: config
                .get("device_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            client_id: config
                .get("client_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            token: config
                .get("token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            device_pool,
            sample_rate: config
                .get("sample_rate")
                .and_then(|v| v.as_u64())
                .unwrap_or(16000) as u32,
            channels: config
                .get("channels")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u8,
            frame_duration: config
                .get("frame_duration")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as u32,
            audio_format: config
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("opus")
                .to_string(),
        })
    }

    fn pick_device_id(&self) -> String {
        if !self.device_pool.is_empty() {
            let idx = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0) as usize)
                % self.device_pool.len();
            return self.device_pool[idx].clone();
        }
        self.device_id.clone()
    }

    async fn synthesize_stream_inner(
        &self,
        text: &str,
        tx: mpsc::Sender<Vec<u8>>,
    ) -> Result<()> {
        if self.server_addr.is_empty() {
            return Err(Error::Config("小智 TTS server_addr 未配置".into()));
        }
        if text.trim().is_empty() {
            return Ok(());
        }

        let device_id = self.pick_device_id();
        if device_id.is_empty() {
            return Err(Error::Config("小智 TTS device_id 未配置".into()));
        }

        let mut request = self
            .server_addr
            .as_str()
            .into_client_request()
            .map_err(|e| Error::Http(format!("小智 WS 请求构建失败: {e}")))?;
        {
            let headers = request.headers_mut();
            headers.insert("Device-Id", device_id.parse().unwrap());
            headers.insert("Content-Type", "application/json".parse().unwrap());
            if !self.token.is_empty() {
                headers.insert(
                    "Authorization",
                    format!("Bearer {}", self.token).parse().unwrap(),
                );
            }
            headers.insert("Protocol-Version", "1".parse().unwrap());
            if !self.client_id.is_empty() {
                headers.insert("Client-Id", self.client_id.parse().unwrap());
            }
        }

        let (ws, _) = connect_async(request)
            .await
            .map_err(|e| Error::Http(format!("小智 TTS 连接失败: {e}")))?;
        let (mut write, mut read) = ws.split();

        let hello = serde_json::json!({
            "type": "hello",
            "device_id": device_id,
            "transport": "websocket",
            "version": 1,
            "audio_params": {
                "format": self.audio_format,
                "sample_rate": self.sample_rate,
                "channels": self.channels,
                "frame_duration": self.frame_duration,
            }
        });
        write
            .send(Message::Text(hello.to_string().into()))
            .await
            .map_err(|e| Error::Http(format!("小智 TTS hello 发送失败: {e}")))?;

        let wrapped = format!("`{}`", text.trim());
        let listen = serde_json::json!({
            "type": "listen",
            "device_id": device_id,
            "state": "detect",
            "text": wrapped,
        });
        write
            .send(Message::Text(listen.to_string().into()))
            .await
            .map_err(|e| Error::Http(format!("小智 TTS listen 发送失败: {e}")))?;

        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| Error::Http(format!("小智 TTS 接收失败: {e}")))?;
            match msg {
                Message::Binary(data) => {
                    if !data.is_empty() {
                        let _ = tx.send(data.to_vec()).await;
                    }
                }
                Message::Text(s) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if v.get("type").and_then(|t| t.as_str()) == Some("tts")
                            && v.get("state").and_then(|t| t.as_str()) == Some("stop")
                        {
                            break;
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        let stop = serde_json::json!({
            "type": "listen",
            "device_id": device_id,
            "state": "stop",
        });
        let _ = write.send(Message::Text(stop.to_string().into())).await;
        Ok(())
    }
}

#[async_trait]
impl TtsProvider for XiaozhiTtsProvider {
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
        let (raw_tx, raw_rx) = mpsc::channel(64);
        let provider = self.clone();
        let text = text.to_string();
        tokio::spawn(async move {
            if let Err(e) = provider.synthesize_stream_inner(&text, raw_tx).await {
                tracing::error!("小智 TTS 合成失败: {e}");
            }
        });
        Ok(wrap_tts_audio_stream(
            raw_rx,
            &self.audio_format,
            sample_rate,
            channels,
            frame_duration,
        ))
    }

    async fn set_voice(&self, _voice_config: &serde_json::Value) -> Result<()> {
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.server_addr.is_empty() && (!self.device_id.is_empty() || !self.device_pool.is_empty())
    }
}
