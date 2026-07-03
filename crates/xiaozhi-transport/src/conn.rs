use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use xiaozhi_core::Result;

pub type CloseCallback = Arc<dyn Fn(&str) + Send + Sync>;

#[async_trait]
pub trait DeviceConn: Send + Sync {
    async fn send_cmd(&self, msg: &[u8]) -> Result<()>;
    async fn recv_cmd(&self, timeout_ms: u64) -> Result<Vec<u8>>;
    async fn send_audio(&self, audio: &[u8]) -> Result<()>;
    async fn recv_audio(&self, timeout_ms: u64) -> Result<Vec<u8>>;
    fn device_id(&self) -> &str;
    async fn close(&self) -> Result<()>;
    fn on_close(&self, callback: CloseCallback);
    async fn close_audio_channel(&self) -> Result<()>;
    fn transport_type(&self) -> &str;
    async fn get_data(&self, key: &str) -> Result<Option<serde_json::Value>>;
    async fn set_data(&self, key: &str, value: serde_json::Value);
}

pub struct ConnData {
    inner: Mutex<HashMap<String, serde_json::Value>>,
}

impl ConnData {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.inner.lock().await.get(key).cloned()
    }

    pub async fn set(&self, key: &str, value: serde_json::Value) {
        self.inner.lock().await.insert(key.to_string(), value);
    }
}

impl Default for ConnData {
    fn default() -> Self {
        Self::new()
    }
}

pub type OnNewConnection = Arc<dyn Fn(Arc<dyn DeviceConn>) + Send + Sync>;
