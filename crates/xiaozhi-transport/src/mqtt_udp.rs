//! MQTT+UDP 传输适配器
//!
//! 运行时由 `xiaozhi-server` 的 `MqttUdpService` 负责；本模块提供 `DeviceConn` 抽象供测试与扩展。

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};
use xiaozhi_core::{Error, Result, transport as transport_const};
use xiaozhi_protocol::mqtt;

use crate::conn::{CloseCallback, ConnData, DeviceConn};
use crate::udp::UdpCrypto;

pub struct MqttUdpConn {
    device_id: String,
    cmd_tx: mpsc::Sender<Vec<u8>>,
    cmd_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    audio_tx: mpsc::Sender<Vec<u8>>,
    audio_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    crypto: UdpCrypto,
    conn_id: u32,
    data: ConnData,
}

impl MqttUdpConn {
    pub fn new(device_id: String, crypto: UdpCrypto, conn_id: u32) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (audio_tx, audio_rx) = mpsc::channel(256);
        Self {
            device_id,
            cmd_tx,
            cmd_rx: Mutex::new(cmd_rx),
            audio_tx,
            audio_rx: Mutex::new(audio_rx),
            crypto,
            conn_id,
            data: ConnData::new(),
        }
    }

    pub fn publish_topic(&self) -> String {
        mqtt::device_sub_topic(&self.device_id)
    }

    pub fn subscribe_topic(&self) -> String {
        mqtt::device_public_topic(&self.device_id)
    }

    pub fn cmd_sender(&self) -> mpsc::Sender<Vec<u8>> {
        self.cmd_tx.clone()
    }

    pub fn audio_sender(&self) -> mpsc::Sender<Vec<u8>> {
        self.audio_tx.clone()
    }
}

#[async_trait]
impl DeviceConn for MqttUdpConn {
    async fn send_cmd(&self, msg: &[u8]) -> Result<()> {
        self.cmd_tx
            .send(msg.to_vec())
            .await
            .map_err(|e| Error::Mqtt(format!("MQTT 发送失败: {e}")))
    }

    async fn recv_cmd(&self, timeout_ms: u64) -> Result<Vec<u8>> {
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            async {
                let mut rx = self.cmd_rx.lock().await;
                rx.recv()
                    .await
                    .ok_or_else(|| Error::Mqtt("MQTT 命令通道关闭".into()))
            },
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    async fn send_audio(&self, audio: &[u8]) -> Result<()> {
        self.audio_tx
            .send(audio.to_vec())
            .await
            .map_err(|e| Error::Transport(format!("UDP 音频发送失败: {e}")))
    }

    async fn recv_audio(&self, timeout_ms: u64) -> Result<Vec<u8>> {
        tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            async {
                let mut rx = self.audio_rx.lock().await;
                rx.recv()
                    .await
                    .ok_or_else(|| Error::Transport("UDP 音频通道关闭".into()))
            },
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    fn device_id(&self) -> &str {
        &self.device_id
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }

    fn on_close(&self, _callback: CloseCallback) {}

    async fn close_audio_channel(&self) -> Result<()> {
        Ok(())
    }

    fn transport_type(&self) -> &str {
        transport_const::MQTT_UDP
    }

    async fn get_data(&self, key: &str) -> Result<Option<serde_json::Value>> {
        Ok(self.data.get(key).await)
    }

    async fn set_data(&self, key: &str, value: serde_json::Value) {
        self.data.set(key, value).await;
    }
}

pub struct MqttUdpAdapter {
    pub mqtt_broker: String,
    pub mqtt_port: u16,
    pub mqtt_type: String,
    pub udp_listen_port: u16,
    pub external_host: String,
    pub external_port: u16,
}

impl MqttUdpAdapter {
    pub async fn start(self: Arc<Self>) -> Result<()> {
        if self.mqtt_broker.is_empty() {
            return Err(Error::Mqtt("MQTT broker 未配置".into()));
        }
        tracing::info!(
            "MQTT+UDP 适配器配置: mqtt={}:{} (type={}), udp={}:{}",
            self.mqtt_broker,
            self.mqtt_port,
            self.mqtt_type,
            self.external_host,
            self.external_port
        );
        tracing::info!(
            "MQTT+UDP 运行时由 xiaozhi-server MqttUdpService 承载，适配器仅做配置校验"
        );
        Ok(())
    }
}
