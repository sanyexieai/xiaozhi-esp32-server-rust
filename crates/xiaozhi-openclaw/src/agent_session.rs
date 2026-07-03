use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;

use crate::stream::ResponseStreamState;
use crate::protocol::WsMessage;

pub struct AgentSession {
    pub agent_id: String,
    outbound: mpsc::UnboundedSender<String>,
    closed: AtomicBool,
    pending: DashMap<String, String>,
    modes: DashMap<String, bool>,
    streams: DashMap<String, ResponseStreamState>,
}

impl AgentSession {
    pub fn new(agent_id: String, outbound: mpsc::UnboundedSender<String>) -> Self {
        Self {
            agent_id,
            outbound,
            closed: AtomicBool::new(false),
            pending: DashMap::new(),
            modes: DashMap::new(),
            streams: DashMap::new(),
        }
    }

    pub fn copy_modes_from(&self, other: &AgentSession) {
        for entry in other.modes.iter() {
            self.modes.insert(entry.key().clone(), *entry.value());
        }
    }

    pub fn load_or_create_stream(&self, correlation_id: &str) -> ResponseStreamState {
        let id = correlation_id.trim().to_string();
        self.streams
            .entry(id)
            .or_insert_with(ResponseStreamState::new)
            .clone()
    }

    pub fn update_stream<F>(&self, correlation_id: &str, update: F)
    where
        F: FnOnce(&mut ResponseStreamState),
    {
        let correlation_id = correlation_id.trim();
        if correlation_id.is_empty() {
            return;
        }
        self.streams
            .entry(correlation_id.to_string())
            .or_insert_with(ResponseStreamState::new);
        if let Some(mut entry) = self.streams.get_mut(correlation_id) {
            update(&mut entry);
        }
    }

    pub fn get_stream_device_id(&self, correlation_id: &str) -> Option<String> {
        self.streams
            .get(correlation_id.trim())
            .map(|s| s.device_id.clone())
            .filter(|id| !id.is_empty())
    }

    pub fn remove_stream(&self, correlation_id: &str) {
        let correlation_id = correlation_id.trim();
        if !correlation_id.is_empty() {
            self.streams.remove(correlation_id);
        }
    }

    pub fn track_pending(&self, correlation_id: &str, device_id: &str) {
        let correlation_id = correlation_id.trim();
        let device_id = device_id.trim();
        if correlation_id.is_empty() || device_id.is_empty() {
            return;
        }
        self.pending
            .insert(correlation_id.to_string(), device_id.to_string());
    }

    pub fn remove_pending(&self, correlation_id: &str) {
        let correlation_id = correlation_id.trim();
        if !correlation_id.is_empty() {
            self.pending.remove(correlation_id);
        }
    }

    pub fn resolve_pending(&self, correlation_id: &str) -> Option<String> {
        let correlation_id = correlation_id.trim();
        if correlation_id.is_empty() {
            return None;
        }
        self.pending
            .remove(correlation_id)
            .map(|(_, device_id)| device_id)
    }

    pub fn peek_pending(&self, correlation_id: &str) -> Option<String> {
        self.pending
            .get(correlation_id.trim())
            .map(|v| v.clone())
    }

    pub fn enter_mode(&self, device_id: &str) -> bool {
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return false;
        }
        self.modes.insert(device_id.to_string(), true);
        true
    }

    pub fn exit_mode(&self, device_id: &str) -> bool {
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return false;
        }
        self.modes.remove(device_id).is_some()
    }

    pub fn is_mode_enabled(&self, device_id: &str) -> bool {
        self.modes
            .get(device_id.trim())
            .map(|v| *v)
            .unwrap_or(false)
    }

    pub fn send(&self, msg: WsMessage) -> xiaozhi_core::Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(xiaozhi_core::Error::Session("OpenClaw 会话已关闭".into()));
        }
        let text = serde_json::to_string(&msg)
            .map_err(|e| xiaozhi_core::Error::Session(format!("序列化失败: {e}")))?;
        self.outbound
            .send(text)
            .map_err(|_| xiaozhi_core::Error::Session("OpenClaw 发送失败".into()))
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

pub type SharedAgentSession = Arc<AgentSession>;
