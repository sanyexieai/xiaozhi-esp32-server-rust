use std::sync::Arc;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;
use xiaozhi_core::{Error, Result, transport as transport_const};

use crate::conn::{CloseCallback, ConnData, DeviceConn};

pub struct WebSocketConn {
    device_id: String,
    cmd_tx: mpsc::Sender<Vec<u8>>,
    audio_tx: mpsc::Sender<Vec<u8>>,
    cmd_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    audio_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    data: ConnData,
    close_callback: RwLock<Option<CloseCallback>>,
}

impl WebSocketConn {
    pub fn new(device_id: String) -> (Self, WebSocketConnHandle) {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (audio_tx, audio_rx) = mpsc::channel(256);
        let (out_cmd_tx, out_cmd_rx) = mpsc::channel(64);
        let (out_audio_tx, out_audio_rx) = mpsc::channel(256);

        let conn = Self {
            device_id: device_id.clone(),
            cmd_tx,
            audio_tx,
            cmd_rx: Mutex::new(cmd_rx),
            audio_rx: Mutex::new(audio_rx),
            data: ConnData::new(),
            close_callback: RwLock::new(None),
        };

        let handle = WebSocketConnHandle {
            device_id,
            out_cmd_tx,
            out_audio_tx,
            in_cmd_rx: Mutex::new(out_cmd_rx),
            in_audio_rx: Mutex::new(out_audio_rx),
        };

        (conn, handle)
    }
}

pub struct WebSocketConnHandle {
    pub device_id: String,
    out_cmd_tx: mpsc::Sender<Vec<u8>>,
    out_audio_tx: mpsc::Sender<Vec<u8>>,
    in_cmd_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    in_audio_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
}

impl WebSocketConnHandle {
    pub async fn pump_from_ws(
        &self,
        mut ws_rx: futures::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        >,
    ) {
        while let Some(msg) = ws_rx.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let _ = self.out_cmd_tx.send(text.as_bytes().to_vec()).await;
                }
                Ok(Message::Binary(data)) => {
                    let _ = self.out_audio_tx.send(data.to_vec()).await;
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    }

    pub async fn pump_to_ws(
        &self,
        mut ws_tx: futures::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
            Message,
        >,
    ) {
        loop {
            tokio::select! {
                cmd = async {
                    let mut rx = self.in_cmd_rx.lock().await;
                    rx.recv().await
                } => {
                    match cmd {
                        Some(data) => {
                            if ws_tx.send(Message::Text(String::from_utf8_lossy(&data).into_owned().into())).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                audio = async {
                    let mut rx = self.in_audio_rx.lock().await;
                    rx.recv().await
                } => {
                    match audio {
                        Some(data) => {
                            if ws_tx.send(Message::Binary(data.into())).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    }
}

#[async_trait]
impl DeviceConn for WebSocketConn {
    async fn send_cmd(&self, msg: &[u8]) -> Result<()> {
        self.cmd_tx
            .send(msg.to_vec())
            .await
            .map_err(|e| Error::Transport(format!("发送命令失败: {e}")))
    }

    async fn recv_cmd(&self, timeout_ms: u64) -> Result<Vec<u8>> {
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            async {
                let mut rx = self.cmd_rx.lock().await;
                rx.recv()
                    .await
                    .ok_or_else(|| Error::Transport("命令通道已关闭".into()))
            },
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    async fn send_audio(&self, audio: &[u8]) -> Result<()> {
        self.audio_tx
            .send(audio.to_vec())
            .await
            .map_err(|e| Error::Transport(format!("发送音频失败: {e}")))
    }

    async fn recv_audio(&self, timeout_ms: u64) -> Result<Vec<u8>> {
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            async {
                let mut rx = self.audio_rx.lock().await;
                rx.recv()
                    .await
                    .ok_or_else(|| Error::Transport("音频通道已关闭".into()))
            },
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    fn device_id(&self) -> &str {
        &self.device_id
    }

    async fn close(&self) -> Result<()> {
        if let Some(cb) = self.close_callback.read().await.as_ref() {
            cb(&self.device_id);
        }
        Ok(())
    }

    fn on_close(&self, callback: CloseCallback) {
        if let Ok(mut guard) = self.close_callback.try_write() {
            *guard = Some(callback);
        }
    }

    async fn close_audio_channel(&self) -> Result<()> {
        Ok(())
    }

    fn transport_type(&self) -> &str {
        transport_const::WEBSOCKET
    }

    async fn get_data(&self, key: &str) -> Result<Option<serde_json::Value>> {
        Ok(self.data.get(key).await)
    }

    async fn set_data(&self, key: &str, value: serde_json::Value) {
        self.data.set(key, value).await;
    }
}

pub type SharedConn = Arc<dyn DeviceConn>;
