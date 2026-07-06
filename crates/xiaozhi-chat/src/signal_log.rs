//! 设备会话信令流水（调试页展示用）

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use xiaozhi_protocol::messages::{ClientMessage, ServerMessage};

const MAX_ENTRIES: usize = 500;
const MAX_PAYLOAD_CHARS: usize = 2000;

#[derive(Debug, Clone, Serialize)]
pub struct SignalEntry {
    pub id: u64,
    pub ts_ms: i64,
    /// `in` 设备→服务端，`out` 服务端→设备
    pub direction: String,
    /// `mqtt` / `ws` / `udp`
    pub channel: String,
    pub msg_type: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

pub struct SignalLog {
    next_id: AtomicU64,
    entries: Mutex<VecDeque<SignalEntry>>,
}

impl Default for SignalLog {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalLog {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            entries: Mutex::new(VecDeque::new()),
        }
    }

    pub async fn clear(&self) {
        let mut guard = self.entries.lock().await;
        guard.clear();
    }

    pub async fn list_since(&self, after_id: u64) -> Vec<SignalEntry> {
        let guard = self.entries.lock().await;
        guard
            .iter()
            .filter(|e| e.id > after_id)
            .cloned()
            .collect()
    }

    pub async fn record_client(&self, channel: &str, msg: &ClientMessage) {
        let payload = client_payload(msg);
        let summary = summarize_client(msg);
        self.push("in", channel, &msg.msg_type, &summary, payload)
            .await;
    }

    pub async fn record_server_json(&self, channel: &str, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        let payload = parse_json_payload(&text);
        let (msg_type, summary) = summarize_server_json(&text, payload.as_ref());
        self.push("out", channel, &msg_type, &summary, payload)
            .await;
    }

    pub async fn record_server_message(&self, channel: &str, msg: &ServerMessage) {
        let payload = serde_json::to_value(msg).ok();
        let summary = summarize_server(msg);
        self.push("out", channel, &msg.msg_type, &summary, payload)
            .await;
    }

    /// LLM 回合内 MCP 工具调用（仅调试流水，不下发设备）。
    pub async fn record_mcp_tool_call(
        &self,
        name: &str,
        invoke_name: &str,
        tool_call_id: &str,
        arguments: &Value,
    ) {
        let summary = if name == invoke_name {
            format!("调用 MCP · {invoke_name}")
        } else {
            format!("调用 MCP · {invoke_name} (LLM: {name})")
        };
        let payload = Some(json!({
            "tool": name,
            "invoke_name": invoke_name,
            "tool_call_id": tool_call_id,
            "arguments": arguments,
        }));
        self.push("internal", "llm", "mcp_tool_call", &summary, payload)
            .await;
    }

    /// LLM 回合内 MCP 工具返回结果。
    pub async fn record_mcp_tool_result(
        &self,
        name: &str,
        invoke_name: &str,
        tool_call_id: &str,
        result_text: &str,
        raw: Option<&Value>,
        stop_llm: bool,
        ok: bool,
    ) {
        let status = if ok { "成功" } else { "失败" };
        let summary = if stop_llm {
            format!("MCP 结果 · {invoke_name} · {status} · 终止 LLM 回合")
        } else {
            format!(
                "MCP 结果 · {invoke_name} · {status} · {}",
                truncate_text(result_text, 64)
            )
        };
        let payload = Some(json!({
            "tool": name,
            "invoke_name": invoke_name,
            "tool_call_id": tool_call_id,
            "ok": ok,
            "stop_llm": stop_llm,
            "result": result_text,
            "raw": raw,
        }));
        self.push("internal", "llm", "mcp_tool_result", &summary, payload)
            .await;
    }

    /// 音频帧：同方向/通道连续帧合并为一条，避免刷屏
    pub async fn record_audio(&self, direction: &str, channel: &str, bytes: usize) {
        let mut guard = self.entries.lock().await;
        if let Some(last) = guard.back_mut() {
            if last.msg_type == "audio"
                && last.direction == direction
                && last.channel == channel
            {
                if let Some(Value::Object(map)) = last.payload.as_mut() {
                    let frames = map
                        .get("frames")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1)
                        + 1;
                    let total = map
                        .get("total_bytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        + bytes as u64;
                    map.insert("frames".into(), frames.into());
                    map.insert("total_bytes".into(), total.into());
                    last.summary = format!(
                        "opus {} · {} 帧 · {} B",
                        if direction == "in" { "↑" } else { "↓" },
                        frames,
                        total
                    );
                }
                return;
            }
        }
        drop(guard);
        let payload = Some(json!({
            "codec": "opus",
            "frames": 1,
            "total_bytes": bytes,
        }));
        let summary = format!(
            "opus {} · 1 帧 · {} B",
            if direction == "in" { "↑" } else { "↓" },
            bytes
        );
        self.push(direction, channel, "audio", &summary, payload)
            .await;
    }

    async fn push(
        &self,
        direction: &str,
        channel: &str,
        msg_type: &str,
        summary: &str,
        payload: Option<Value>,
    ) {
        let mut guard = self.entries.lock().await;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        guard.push_back(SignalEntry {
            id,
            ts_ms: now_ms(),
            direction: direction.to_string(),
            channel: channel.to_string(),
            msg_type: msg_type.to_string(),
            summary: summary.to_string(),
            payload: truncate_payload(payload),
        });
        while guard.len() > MAX_ENTRIES {
            guard.pop_front();
        }
    }
}

fn parse_json_payload(text: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(text).ok()?;
    truncate_payload(Some(v))
}

fn truncate_payload(payload: Option<Value>) -> Option<Value> {
    let v = payload?;
    let s = v.to_string();
    if s.len() <= MAX_PAYLOAD_CHARS {
        return Some(v);
    }
    Some(json!({
        "_truncated": true,
        "preview": &s[..MAX_PAYLOAD_CHARS],
    }))
}

fn client_payload(msg: &ClientMessage) -> Option<Value> {
    truncate_payload(serde_json::to_value(msg).ok())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn summarize_client(msg: &ClientMessage) -> String {
    match msg.msg_type.as_str() {
        "listen" => {
            let state = msg.state.as_deref().unwrap_or("-");
            let mode = msg.mode.as_deref().unwrap_or("");
            let text = msg.text.as_deref().unwrap_or("");
            if state == "detect" && !text.is_empty() {
                format!("listen.detect · 「{}」", truncate_text(text, 48))
            } else if !mode.is_empty() {
                format!("listen.{state} · mode={mode}")
            } else {
                format!("listen.{state}")
            }
        }
        "abort" => {
            let reason = msg
                .extra
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if reason.is_empty() {
                "abort".into()
            } else {
                format!("abort · reason={reason}")
            }
        }
        "hello" => {
            let transport = msg.transport.as_deref().unwrap_or("?");
            format!("hello · transport={transport}")
        }
        "goodbye" => "goodbye".into(),
        "mcp" => "mcp".into(),
        "iot" => "iot".into(),
        "speak_ready" => "speak_ready".into(),
        other => other.to_string(),
    }
}

fn summarize_server(msg: &ServerMessage) -> String {
    match msg.msg_type.as_str() {
        "tts" => {
            let state = msg.state.as_deref().unwrap_or("-");
            let text = msg.text.as_deref().unwrap_or("");
            if state == "sentence_start" && !text.is_empty() {
                format!("tts.{state} · 「{}」", truncate_text(text, 48))
            } else {
                format!("tts.{state}")
            }
        }
        "stt" => {
            let text = msg.text.as_deref().unwrap_or("");
            if text.is_empty() {
                "stt".into()
            } else {
                format!("stt · 「{}」", truncate_text(text, 48))
            }
        }
        "hello" => "hello".into(),
        "goodbye" => "goodbye".into(),
        "llm" => "llm".into(),
        "mcp" => "mcp".into(),
        other => other.to_string(),
    }
}

fn summarize_server_json(text: &str, payload: Option<&Value>) -> (String, String) {
    if let Some(v) = payload {
        let msg_type = v
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("json")
            .to_string();
        if msg_type == "tts" {
            let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("-");
            let summary = if state == "sentence_start" {
                let t = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
                if t.is_empty() {
                    format!("tts.{state}")
                } else {
                    format!("tts.{state} · 「{}」", truncate_text(t, 48))
                }
            } else {
                format!("tts.{state}")
            };
            return (msg_type, summary);
        }
        if msg_type == "stt" {
            let t = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
            let summary = if t.is_empty() {
                "stt".into()
            } else {
                format!("stt · 「{}」", truncate_text(t, 48))
            };
            return (msg_type, summary);
        }
        return (msg_type.clone(), msg_type);
    }
    let preview = truncate_text(text, 64);
    ("json".into(), preview)
}

fn truncate_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mcp_tool_events_recorded() {
        let log = SignalLog::new();
        log.record_mcp_tool_call(
            "weather",
            "global_weather",
            "call_1",
            &json!({"city": "北京"}),
        )
        .await;
        log.record_mcp_tool_result(
            "weather",
            "global_weather",
            "call_1",
            "晴，25°C",
            None,
            false,
            true,
        )
        .await;
        let items = log.list_since(0).await;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].msg_type, "mcp_tool_call");
        assert_eq!(items[0].direction, "internal");
        assert_eq!(items[1].msg_type, "mcp_tool_result");
    }

    #[tokio::test]
    async fn audio_entries_merge() {
        let log = SignalLog::new();
        log.record_audio("out", "udp", 120).await;
        log.record_audio("out", "udp", 80).await;
        let items = log.list_since(0).await;
        assert_eq!(items.len(), 1);
        assert!(items[0].summary.contains("2 帧"));
        assert!(items[0].summary.contains("200 B"));
    }
}
