use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};
use xiaozhi_mcp::McpTool;
use xiaozhi_protocol::messages::ServerMessage;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_INIT_ID: u64 = 1;
const MCP_TOOLS_LIST_ID: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpInboundAction {
    None,
    Respond(Value),
    RefreshTools,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitState {
    Idle,
    InFlight,
    Ready,
}

pub struct DeviceMcpRuntime {
    init_state: InitState,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    device_tools: Vec<McpTool>,
    next_request_id: AtomicU64,
}

impl Default for DeviceMcpRuntime {
    fn default() -> Self {
        Self {
            init_state: InitState::Idle,
            pending: Arc::new(Mutex::new(HashMap::new())),
            device_tools: Vec::new(),
            next_request_id: AtomicU64::new(3),
        }
    }
}

impl DeviceMcpRuntime {
    pub fn is_ready(&self) -> bool {
        self.init_state == InitState::Ready
    }

    pub fn device_tools(&self) -> &[McpTool] {
        &self.device_tools
    }

    pub fn should_schedule_init(&self) -> bool {
        matches!(self.init_state, InitState::Idle)
    }

    /// 尝试开始初始化，避免 duplicate hello 并发重复 initialize
    pub fn try_begin_init(&mut self) -> bool {
        if self.init_state == InitState::InFlight {
            return false;
        }
        if self.init_state == InitState::Ready {
            tracing::warn!("MCP 状态漂移: runtime 已 ready，仍重新初始化");
        }
        self.init_state = InitState::InFlight;
        true
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.device_tools.iter().any(|t| t.name == name)
    }

    pub fn allocate_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn pending_hub(&self) -> Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> {
        Arc::clone(&self.pending)
    }

    pub fn mark_ready(&mut self, tools: Vec<McpTool>) {
        self.device_tools = tools;
        self.init_state = InitState::Ready;
    }

    pub fn mark_failed(&mut self) {
        self.init_state = InitState::Idle;
    }

    pub fn reset_on_transport_ready(&mut self) {
        self.init_state = InitState::Idle;
        self.device_tools.clear();
        self.next_request_id.store(3, Ordering::Relaxed);
    }

    pub fn update_tools(&mut self, tools: Vec<McpTool>) {
        self.device_tools = tools;
        if self.init_state == InitState::Idle || self.init_state == InitState::InFlight {
            self.init_state = InitState::Ready;
        }
    }

    pub async fn handle_inbound(&self, payload: &Value) -> McpInboundAction {
        let Some(id) = payload.get("id") else {
            if let Some(method) = payload.get("method").and_then(|v| v.as_str()) {
                tracing::debug!("设备 MCP 通知: {method}");
                if method == "notifications/tools/list_changed" {
                    return McpInboundAction::RefreshTools;
                }
            }
            return McpInboundAction::None;
        };

        let id_key = json_id_key(id);
        if payload.get("result").is_some() || payload.get("error").is_some() {
            let mut pending = self.pending.lock().await;
            if let Some(tx) = pending.remove(&id_key) {
                let _ = tx.send(payload.clone());
            } else {
                tracing::warn!("未匹配的 MCP 响应 id={id_key}");
            }
            return McpInboundAction::None;
        }

        let method = match payload.get("method").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => return McpInboundAction::None,
        };
        McpInboundAction::Respond(self.handle_device_request(id, method))
    }

    fn handle_device_request(&self, id: &Value, method: &str) -> Value {
        match method {
            "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "serverInfo": {
                        "name": "xiaozhi-server-rust",
                        "version": "0.1.0"
                    }
                }
            }),
            "tools/list" => {
                let tools: Vec<Value> = self
                    .device_tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema,
                        })
                    })
                    .collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                })
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {method}")
                }
            }),
        }
    }
}

pub async fn run_device_mcp_init<F, Fut>(
    session_id: &str,
    vision_url: &str,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    mut send: F,
) -> Result<Vec<McpTool>, String>
where
    F: FnMut(ServerMessage) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    run_device_mcp_init_json(vision_url, pending, |payload| {
        send(ServerMessage::mcp(session_id, payload))
    })
    .await
}

pub async fn run_device_mcp_init_json<F, Fut>(
    vision_url: &str,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    mut send: F,
) -> Result<Vec<McpTool>, String>
where
    F: FnMut(Value) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let init_payload = json!({
        "jsonrpc": "2.0",
        "id": MCP_INIT_ID,
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "vision": {
                    "url": vision_url,
                    "token": "1234567890"
                }
            },
            "clientInfo": {
                "name": "xiaozhi-server-rust",
                "version": "0.1.0"
            }
        }
    });

    let init_resp =
        send_and_wait_json(MCP_INIT_ID, init_payload, pending, &mut send).await?;
    if init_resp.get("error").is_some() {
        return Err(format!("initialize 失败: {init_resp}"));
    }

    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    if !send(initialized).await {
        return Err("发送 notifications/initialized 失败".into());
    }

    let tools_payload = json!({
        "jsonrpc": "2.0",
        "id": MCP_TOOLS_LIST_ID,
        "method": "tools/list",
        "params": {}
    });
    let tools_resp =
        send_and_wait_json(MCP_TOOLS_LIST_ID, tools_payload, pending, &mut send).await?;
    if tools_resp.get("error").is_some() {
        return Err(format!("tools/list 失败: {tools_resp}"));
    }

    let tools = parse_tools_list(tools_resp.get("result"));
    tracing::info!("设备 MCP 初始化完成，工具数={}", tools.len());
    Ok(tools)
}

pub async fn refresh_device_tools_json<F, Fut>(
    request_id: u64,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    mut send: F,
) -> Result<Vec<McpTool>, String>
where
    F: FnMut(Value) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let tools_payload = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "tools/list",
        "params": {}
    });
    let tools_resp =
        send_and_wait_json(request_id, tools_payload, pending, &mut send).await?;
    if tools_resp.get("error").is_some() {
        return Err(format!("tools/list 失败: {tools_resp}"));
    }
    let tools = parse_tools_list(tools_resp.get("result"));
    tracing::info!("设备 MCP 工具列表已刷新，工具数={}", tools.len());
    Ok(tools)
}

pub async fn call_device_tool<F, Fut>(
    session_id: &str,
    tool_name: &str,
    arguments: Value,
    request_id: u64,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    mut send: F,
) -> Result<String, String>
where
    F: FnMut(ServerMessage) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let payload = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let resp = send_and_wait(session_id, request_id, payload, pending, &mut send).await?;
    if let Some(err) = resp.get("error") {
        return Err(format!("设备工具调用失败: {err}"));
    }
    Ok(format_tool_result(resp.get("result")))
}

pub async fn call_device_tool_raw<F, Fut>(
    session_id: &str,
    tool_name: &str,
    arguments: Value,
    request_id: u64,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    mut send: F,
) -> Result<Value, String>
where
    F: FnMut(ServerMessage) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let payload = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let resp = send_and_wait(session_id, request_id, payload, pending, &mut send).await?;
    if let Some(err) = resp.get("error") {
        return Err(format!("设备工具调用失败: {err}"));
    }
    Ok(resp.get("result").cloned().unwrap_or(json!({})))
}

fn format_tool_result(result: Option<&Value>) -> String {
    let Some(result) = result else {
        return "ok".to_string();
    };
    if let Some(items) = result.get("content").and_then(|v| v.as_array()) {
        let texts: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if item.get("type")?.as_str()? == "text" {
                    item.get("text")?.as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect();
        if !texts.is_empty() {
            return texts.join("\n");
        }
    }
    result.to_string()
}

async fn send_and_wait_json<F, Fut>(
    id: u64,
    payload: Value,
    pending_map: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    send: &mut F,
) -> Result<Value, String>
where
    F: FnMut(Value) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let id_key = id.to_string();
    let (tx, rx) = oneshot::channel();
    pending_map.lock().await.insert(id_key.clone(), tx);

    if !send(payload).await {
        pending_map.lock().await.remove(&id_key);
        return Err("下发 MCP 消息失败".into());
    }

    match tokio::time::timeout(Duration::from_secs(15), rx).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(_)) => Err("MCP 响应通道已关闭".into()),
        Err(_) => {
            pending_map.lock().await.remove(&id_key);
            Err(format!("MCP 请求超时 id={id_key}"))
        }
    }
}

async fn send_and_wait<F, Fut>(
    session_id: &str,
    id: u64,
    payload: Value,
    pending_map: &Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    send: &mut F,
) -> Result<Value, String>
where
    F: FnMut(ServerMessage) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let id_key = id.to_string();
    let (tx, rx) = oneshot::channel();
    pending_map.lock().await.insert(id_key.clone(), tx);

    if !send(ServerMessage::mcp(session_id, payload)).await {
        pending_map.lock().await.remove(&id_key);
        return Err("下发 MCP 消息失败".into());
    }

    match tokio::time::timeout(Duration::from_secs(15), rx).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(_)) => Err("MCP 响应通道已关闭".into()),
        Err(_) => {
            pending_map.lock().await.remove(&id_key);
            Err(format!("MCP 请求超时 id={id_key}"))
        }
    }
}

fn json_id_key(id: &Value) -> String {
    match id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn parse_tools_list(result: Option<&Value>) -> Vec<McpTool> {
    let Some(result) = result else {
        return Vec::new();
    };
    let tools = result
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    tools
        .into_iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?.to_string();
            let description = t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input_schema = t
                .get("inputSchema")
                .or_else(|| t.get("input_schema"))
                .cloned()
                .unwrap_or(json!({}));
            Some(McpTool {
                name,
                description,
                input_schema,
            })
        })
        .collect()
}

pub fn has_mcp_feature(features: &Option<Value>) -> bool {
    features.as_ref().is_some_and(|f| {
        f.get("mcp")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || f.get("mcp").and_then(|v| v.as_str()) == Some("true")
    })
}

pub type SharedDeviceMcp = Mutex<DeviceMcpRuntime>;
