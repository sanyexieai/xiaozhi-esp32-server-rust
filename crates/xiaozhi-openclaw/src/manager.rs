use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::mpsc;
use uuid::Uuid;
use xiaozhi_config::user::OpenClawConfig;
use xiaozhi_core::Result;

use crate::agent_session::{AgentSession, SharedAgentSession};
use crate::delivery::ResponseDelivery;
use crate::protocol::{
    build_prompted_content, metadata_bool, metadata_i64, metadata_string, MessagePayload,
    ResponsePayload, WsMessage,
};
use crate::stream::{
    extract_openclaw_sentences, is_openclaw_snapshot_frame, normalize_openclaw_speech_text,
    SENTENCE_MIN_LEN,
};

const MAX_OFFLINE_MESSAGES: usize = 20;
const OFFLINE_TTL: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone)]
pub struct OpenClawOfflineMessage {
    pub text: String,
    pub correlation_id: String,
    pub is_end: bool,
}

struct OfflineMessage {
    text: String,
    correlation_id: String,
    is_end: bool,
    created_at: Instant,
}

pub type ResponseHandler = Arc<dyn Fn(ResponseDelivery) + Send + Sync>;

pub struct OpenClawManager {
    agent_sessions: DashMap<String, SharedAgentSession>,
    legacy_active_devices: DashMap<String, bool>,
    offline_queue: DashMap<String, VecDeque<OfflineMessage>>,
    response_handler: RwLock<Option<ResponseHandler>>,
    default_enter: Vec<String>,
    default_exit: Vec<String>,
}

impl OpenClawManager {
    pub fn new() -> Self {
        Self {
            agent_sessions: DashMap::new(),
            legacy_active_devices: DashMap::new(),
            offline_queue: DashMap::new(),
            response_handler: RwLock::new(None),
            default_enter: vec![
                "打开龙虾".into(),
                "进入龙虾".into(),
                "open claw".into(),
            ],
            default_exit: vec![
                "关闭龙虾".into(),
                "退出龙虾".into(),
                "close claw".into(),
            ],
        }
    }

    pub fn set_response_handler(&self, handler: ResponseHandler) {
        if let Ok(mut guard) = self.response_handler.write() {
            *guard = Some(handler);
        }
    }

    pub fn agent_session_count(&self) -> usize {
        self.agent_sessions.len()
    }

    pub fn register_agent_connection(
        &self,
        agent_id: &str,
    ) -> (SharedAgentSession, mpsc::UnboundedReceiver<String>) {
        let agent_id = agent_id.trim().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Arc::new(AgentSession::new(agent_id.clone(), tx));
        if let Some(old) = self.agent_sessions.insert(agent_id.clone(), session.clone()) {
            session.copy_modes_from(&old);
            old.close();
            tracing::info!(agent_id = %agent_id, "OpenClaw 会话已替换");
        }
        tracing::info!(agent_id = %agent_id, "OpenClaw 会话已注册");
        (session, rx)
    }

    pub fn unregister_agent_connection(&self, agent_id: &str, session: &SharedAgentSession) {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return;
        }
        if let Some(current) = self.agent_sessions.get(agent_id) {
            if Arc::ptr_eq(&*current, session) {
                drop(current);
                self.agent_sessions.remove(agent_id);
                tracing::info!(agent_id, "OpenClaw 会话已注销");
            }
        }
        session.close();
    }

    pub fn get_agent_session(&self, agent_id: &str) -> Option<SharedAgentSession> {
        self.agent_sessions.get(agent_id.trim()).map(|s| s.clone())
    }

    pub fn send_message(
        &self,
        agent_id: &str,
        device_id: &str,
        content: &str,
        session_id: &str,
    ) -> Result<String> {
        let agent_id = agent_id.trim();
        let device_id = device_id.trim();
        let session_id = session_id.trim();
        let content = content.trim();
        if agent_id.is_empty() || device_id.is_empty() || content.is_empty() {
            return Err(xiaozhi_core::Error::Session(
                "OpenClaw SendMessage 参数不完整".into(),
            ));
        }
        let prompted = build_prompted_content(content);
        if prompted.is_empty() {
            return Err(xiaozhi_core::Error::Session(
                "OpenClaw 提示内容为空".into(),
            ));
        }
        let session = self
            .get_agent_session(agent_id)
            .ok_or_else(|| xiaozhi_core::Error::Session(format!("OpenClaw agent {agent_id} 未连接")))?;

        let message_id = Uuid::new_v4().to_string();
        let payload = serde_json::to_value(MessagePayload {
            content: prompted,
            session_id: session_id.to_string(),
            metadata: Some(serde_json::json!({
                "device_id": device_id,
                "agent_id": agent_id,
                "stream": true,
            })),
        })
        .map_err(|e| xiaozhi_core::Error::Session(format!("payload 序列化失败: {e}")))?;

        session.track_pending(&message_id, device_id);
        session.send(WsMessage {
            id: message_id.clone(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: "message".into(),
            correlation_id: String::new(),
            payload,
        })?;
        Ok(message_id)
    }

    pub fn enter_mode(&self, agent_id: &str, device_id: &str) -> bool {
        let agent_id = agent_id.trim();
        let device_id = device_id.trim();
        if let Some(session) = self.get_agent_session(agent_id) {
            let ok = session.enter_mode(device_id);
            tracing::info!(agent_id, device_id, ok, "OpenClaw 进入模式");
            return ok;
        }
        tracing::warn!(agent_id, device_id, "OpenClaw 进入模式失败：agent 未连接");
        false
    }

    pub fn exit_mode(&self, agent_id: &str, device_id: &str) -> bool {
        let agent_id = agent_id.trim();
        let device_id = device_id.trim();
        let mut ok = false;
        if let Some(session) = self.get_agent_session(agent_id) {
            ok = session.exit_mode(device_id);
        }
        self.legacy_active_devices.remove(device_id);
        tracing::info!(agent_id, device_id, ok, "OpenClaw 退出模式");
        ok
    }

    pub fn is_mode_enabled(&self, agent_id: &str, device_id: &str) -> bool {
        if let Some(session) = self.get_agent_session(agent_id) {
            return session.is_mode_enabled(device_id);
        }
        self.legacy_active_devices
            .get(device_id.trim())
            .map(|v| *v)
            .unwrap_or(false)
    }

    pub fn should_enter(&self, text: &str, config: &OpenClawConfig) -> bool {
        if !config.allowed {
            return false;
        }
        let keywords = if config.enter_keywords.is_empty() {
            &self.default_enter
        } else {
            &config.enter_keywords
        };
        keywords.iter().any(|k| text.contains(k))
    }

    pub fn should_exit(&self, text: &str, config: &OpenClawConfig) -> bool {
        let keywords = if config.exit_keywords.is_empty() {
            &self.default_exit
        } else {
            &config.exit_keywords
        };
        keywords.iter().any(|k| text.contains(k))
    }

    pub fn openclaw_status(&self, agent_id: &str) -> serde_json::Value {
        let agent_id = agent_id.trim();
        let connected = self.get_agent_session(agent_id).is_some();
        let status = if connected { "online" } else { "offline" };
        serde_json::json!({
            "agent_id": agent_id,
            "connected": connected,
            "status": status,
        })
    }

    pub fn queue_offline_message(&self, device_id: &str, text: String) {
        self.queue_offline_message_ex(device_id, text, "", false);
    }

    pub fn queue_offline_message_ex(
        &self,
        device_id: &str,
        text: String,
        correlation_id: &str,
        is_end: bool,
    ) {
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return;
        }
        let text = text.trim().to_string();
        if text.is_empty() && !is_end {
            return;
        }
        let mut queue = self
            .offline_queue
            .entry(device_id.to_string())
            .or_default();
        queue.push_back(OfflineMessage {
            text,
            correlation_id: correlation_id.trim().to_string(),
            is_end,
            created_at: Instant::now(),
        });
        while queue.len() > MAX_OFFLINE_MESSAGES {
            queue.pop_front();
        }
    }

    pub fn clear_offline_messages(&self, device_id: &str) {
        let device_id = device_id.trim();
        if !device_id.is_empty() {
            self.offline_queue.remove(device_id);
        }
    }

    pub fn drain_offline_messages(&self, device_id: &str) -> Vec<OpenClawOfflineMessage> {
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return Vec::new();
        }
        let Some(mut queue) = self.offline_queue.get_mut(device_id) else {
            return Vec::new();
        };
        queue.retain(|msg| msg.created_at.elapsed() < OFFLINE_TTL);
        let drained: Vec<_> = queue
            .drain(..)
            .map(|msg| OpenClawOfflineMessage {
                text: msg.text,
                correlation_id: msg.correlation_id,
                is_end: msg.is_end,
            })
            .collect();
        drop(queue);
        if self
            .offline_queue
            .get(device_id)
            .is_some_and(|q| q.is_empty())
        {
            self.offline_queue.remove(device_id);
        }
        drained
    }

    pub fn handle_response(
        &self,
        agent_id: &str,
        session: Option<&AgentSession>,
        correlation_id: &str,
        payload: ResponsePayload,
    ) {
        let agent_id = agent_id.trim();
        let correlation_id = correlation_id.trim();
        let session_id = payload.session_id.trim().to_string();
        let raw_content = payload.content.clone();
        let content = raw_content.trim().to_string();

        let stream_phase = metadata_string(&payload.metadata, "phase");
        let stream_content_type = metadata_string(&payload.metadata, "content_type");
        let stream_seq = metadata_i64(&payload.metadata, "seq");
        let is_snapshot = is_openclaw_snapshot_frame(&stream_phase, &stream_content_type);

        let mut stream_done = metadata_bool(&payload.metadata, "done")
            || stream_phase.eq_ignore_ascii_case("done");
        let has_stream_markers = stream_done
            || stream_seq > 0
            || !metadata_string(&payload.metadata, "stream_id").is_empty()
            || !stream_phase.is_empty()
            || !stream_content_type.is_empty();
        if !has_stream_markers || correlation_id.is_empty() {
            stream_done = true;
        }

        let mut device_id = metadata_string(&payload.metadata, "device_id");
        if device_id.is_empty() {
            if let Some(sess) = session {
                if let Some(id) = sess.get_stream_device_id(correlation_id) {
                    device_id = id;
                }
            }
        }
        if device_id.is_empty() {
            if let Some(sess) = session {
                if let Some(id) = sess.peek_pending(correlation_id) {
                    device_id = id;
                }
            }
        }
        if device_id.is_empty() {
            tracing::warn!(agent_id, correlation_id, "OpenClaw response 缺少 device 路由");
            return;
        }

        let emit = |mgr: &Self, text: String, is_start: bool, is_end: bool| {
            let text = text.trim().to_string();
            if text.is_empty() && !is_end {
                return;
            }
            let offline_text = text.clone();
            let event = ResponseDelivery {
                device_id: device_id.clone(),
                correlation_id: correlation_id.to_string(),
                session_id: session_id.clone(),
                text,
                is_start,
                is_end,
                metadata: payload.metadata.clone(),
            };
            if mgr.deliver_response(event) {
                return;
            }
            mgr.queue_offline_message_ex(
                &device_id,
                offline_text,
                correlation_id,
                is_end,
            );
        };

        if session.is_none() || correlation_id.is_empty() {
            if !content.is_empty() {
                emit(self, content, true, stream_done);
            } else if stream_done {
                emit(self, String::new(), false, true);
            }
            return;
        }

        let sess = session.unwrap();
        if stream_seq > 0 {
            let mut skip = false;
            sess.update_stream(correlation_id, |state| {
                if state.last_seq > 0 && stream_seq <= state.last_seq {
                    skip = true;
                    return;
                }
                state.last_seq = stream_seq;
            });
            if skip {
                tracing::warn!(
                    agent_id,
                    correlation_id,
                    stream_seq,
                    "OpenClaw response seq 重复，已忽略"
                );
                return;
            }
        }

        sess.update_stream(correlation_id, |state| {
            if state.device_id.is_empty() {
                state.device_id = device_id.clone();
            }
        });

        let mut incremental = String::new();
        let mut working_text = String::new();
        let mut is_first = true;
        let mut buffered_snapshot = false;

        sess.update_stream(correlation_id, |state| {
            is_first = state.is_first;
            if is_snapshot {
                state.apply_snapshot_content(&raw_content);
                incremental.clear();
                working_text.clear();
                buffered_snapshot = !raw_content.trim().is_empty();
            } else {
                incremental = state.to_incremental_content(&raw_content, stream_done);
                if !incremental.is_empty() {
                    state.buffer =
                        normalize_openclaw_speech_text(format!("{}{}", state.buffer, incremental));
                }
                working_text = state.buffer.trim().to_string();
            }
        });

        let (sentences, mut remaining) = if working_text.is_empty() {
            (Vec::new(), String::new())
        } else {
            extract_openclaw_sentences(&working_text, SENTENCE_MIN_LEN, is_first)
        };

        sess.update_stream(correlation_id, |state| {
            if buffered_snapshot {
                remaining = state.buffer.trim().to_string();
            } else {
                state.buffer = remaining.clone();
            }
        });

        for (idx, sentence) in sentences.iter().enumerate() {
            emit(self, sentence.clone(), is_first && idx == 0, false);
            sess.update_stream(correlation_id, |state| state.mark_emitted(sentence));
        }
        if !sentences.is_empty() {
            sess.update_stream(correlation_id, |state| state.is_first = false);
        }

        if !stream_done {
            return;
        }

        let final_text = remaining.trim().to_string();
        let final_is_start = is_first && sentences.is_empty();
        if !final_text.is_empty() || final_is_start {
            emit(self, final_text.clone(), final_is_start, true);
            if !final_text.is_empty() {
                sess.update_stream(correlation_id, |state| state.mark_emitted(&final_text));
            }
        } else {
            emit(self, String::new(), false, true);
        }

        sess.remove_pending(correlation_id);
        sess.remove_stream(correlation_id);
    }

    fn deliver_response(&self, event: ResponseDelivery) -> bool {
        let handler = self
            .response_handler
            .read()
            .ok()
            .and_then(|g| g.clone());
        if let Some(handler) = handler {
            handler(event);
            true
        } else {
            false
        }
    }

    pub async fn replay_offline_messages<F, Fut>(
        &self,
        device_id: &str,
        mut deliver: F,
    ) -> (usize, usize)
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let device_id = device_id.trim();
        if device_id.is_empty() {
            return (0, 0);
        }

        let snapshot: Vec<String> = {
            let Some(mut queue) = self.offline_queue.get_mut(device_id) else {
                return (0, 0);
            };
            queue.retain(|msg| msg.created_at.elapsed() < OFFLINE_TTL);
            queue.iter().map(|m| m.text.clone()).collect()
        };

        let mut delivered = 0usize;
        for text in &snapshot {
            if deliver(text.clone()).await.is_err() {
                break;
            }
            delivered += 1;
        }

        let remaining = if delivered > 0 {
            if let Some(mut queue) = self.offline_queue.get_mut(device_id) {
                if delivered >= queue.len() {
                    drop(queue);
                    self.offline_queue.remove(device_id);
                    0
                } else {
                    for _ in 0..delivered {
                        queue.pop_front();
                    }
                    queue.len()
                }
            } else {
                0
            }
        } else {
            self.offline_queue
                .get(device_id)
                .map(|q| q.len())
                .unwrap_or(0)
        };

        (delivered, remaining)
    }
}

impl Default for OpenClawManager {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedOpenClawManager = Arc<OpenClawManager>;
