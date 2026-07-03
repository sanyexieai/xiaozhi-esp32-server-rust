use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use xiaozhi_chat::{parse_tts_audio_route, ChatManagerRegistry, SharedResourcePools, TtsAudioRoute};
use xiaozhi_config_provider::{events, UserConfigProvider};
use xiaozhi_history::HistoryClient;
use xiaozhi_mcp::{McpManager, McpRequest};
use xiaozhi_openclaw::OpenClawManager;

use super::types::{WsRequest, WsResponse};
use crate::config_test;
use crate::mqtt_runtime::{MqttRuntime, MqttServiceDeps};
use crate::shared_config::{apply_system_config, SharedAppConfig};

pub struct BridgeDispatcher {
    pub config: SharedAppConfig,
    pub mcp_manager: Arc<McpManager>,
    pub openclaw: Arc<OpenClawManager>,
    pub chat_registry: Arc<ChatManagerRegistry>,
    pub config_provider: Arc<dyn UserConfigProvider>,
    pub history: Arc<HistoryClient>,
    pub resource_pools: Arc<SharedResourcePools>,
    pub mqtt_runtime: Arc<MqttRuntime>,
}

impl BridgeDispatcher {
    pub async fn handle(&self, req: WsRequest) -> WsResponse {
        let id = req.id.clone();
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", path) if path.ends_with("/mcp-tools") && path.contains("/agents/") => {
                self.agent_mcp_tools(id, path).await
            }
            ("POST", path) if path.ends_with("/mcp-call") && path.contains("/agents/") => {
                self.agent_mcp_call(id, req.body).await
            }
            ("GET", path) if path.ends_with("/mcp-tools") && path.contains("/devices/") => {
                self.device_mcp_tools(id, path).await
            }
            ("POST", path) if path.ends_with("/mcp-call") && path.contains("/devices/") => {
                self.device_mcp_call(id, path, req.body).await
            }
            ("POST", "/api/device/inject_msg") => self.inject_message(id, req.body).await,
            ("POST", "/api/device/speak") => self.device_speak(id, req.body).await,
            ("POST", "/api/device/abort") => self.device_abort(id, req.body).await,
            ("POST", "/api/device/goodbye") => self.device_goodbye(id, req.body).await,
            ("POST", "/api/device/endpoints") => self.device_endpoints(id, req.body).await,
            ("POST", "/api/device/endpoints/batch") => {
                self.device_endpoints_batch(id, req.body).await
            }
            ("POST", "/api/device/signals") => self.device_signals(id, req.body).await,
            ("POST", path) if path.ends_with("/openclaw-chat-test") => {
                self.openclaw_chat_test(id, path, req.body).await
            }
            ("GET", "/api/openclaw/status") => self.openclaw_status(id, req.body).await,
            ("POST", path) if path.starts_with("/ws/test/") => {
                self.config_test(id, path, req.body).await
            }
            _ => WsResponse::err(id, 404, format!("未实现: {} {}", req.method, req.path)),
        }
    }

    pub async fn apply_system_config(&self, data: Value) {
        let reload_mcp = data.get("mcp").is_some();
        // ota 变更（签名密钥等）不需重启 MQTT 客户端；仅传输相关块变更时才热重载
        let reload_mqtt = data.get("mqtt").is_some()
            || data.get("mqtt_server").is_some()
            || data.get("udp").is_some();

        apply_system_config(&self.config, &data).await;

        if data.get("local_mcp").is_some() {
            let cfg = self.config.read().await;
            self.mcp_manager.reload_local_mcp(&cfg.local_mcp);
        }

        if reload_mcp {
            let cfg = self.config.read().await;
            self.mcp_manager.reload_global(&cfg.mcp.global);
        } else {
            let mut cfg = self.config.write().await;
            cfg.mcp.global.enabled = false;
            cfg.mcp.global.servers.clear();
            let snapshot = cfg.mcp.global.clone();
            drop(cfg);
            self.mcp_manager.reload_global(&snapshot);
        }

        if reload_mqtt {
            let cfg = self.config.read().await.clone();
            self.mqtt_runtime
                .apply_from_config_change(MqttServiceDeps {
                    config: cfg,
                    chat_registry: self.chat_registry.clone(),
                    config_provider: self.config_provider.clone(),
                    history: self.history.clone(),
                    openclaw: self.openclaw.clone(),
                    mcp_manager: self.mcp_manager.clone(),
                    resource_pools: self.resource_pools.clone(),
                })
                .await;
        }
    }

    async fn agent_mcp_tools(&self, id: String, path: &str) -> WsResponse {
        let _agent_id = parse_trailing_id(path, "/mcp-tools");
        let entries = self.mcp_manager.list_all_tool_entries().await;
        let tool_groups = xiaozhi_mcp::McpManager::group_tool_entries(&entries);
        let tools: Vec<Value> = entries
            .iter()
            .map(|entry| {
                json!({
                    "name": entry.name,
                    "description": entry.description,
                    "input_schema": entry.input_schema,
                    "server_name": entry.server_name,
                })
            })
            .collect();
        let global_count = self.mcp_manager.global_tool_count().await;
        let global_servers: Vec<Value> = self
            .mcp_manager
            .enabled_global_servers()
            .iter()
            .filter(|s| s.enabled)
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": format!("全局 MCP 服务: {}", s.name),
                    "transport": s.r#type,
                    "url": s.url,
                })
            })
            .collect();
        WsResponse::ok(
            id,
            json!({
                "tools": tools,
                "tool_groups": tool_groups,
                "global_servers": global_servers,
                "global_count": global_count,
                "total_count": tools.len(),
            }),
        )
    }

    async fn agent_mcp_call(&self, id: String, body: Value) -> WsResponse {
        let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let arguments = body.get("arguments").cloned().unwrap_or(json!({}));
        if tool_name.is_empty() {
            return WsResponse::err(id, 400, "tool_name 不能为空");
        }

        if self.mcp_manager.has_global_tool(tool_name) {
            match self
                .mcp_manager
                .call_global_tool_raw(tool_name, arguments.clone())
                .await
            {
                Ok(result) => {
                    return WsResponse::ok(
                        id,
                        json!({
                            "tool_name": tool_name,
                            "arguments": arguments,
                            "result": result,
                        }),
                    );
                }
                Err(e) => return WsResponse::err(id, 500, e),
            }
        }

        let req = McpRequest {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: "tools/call".into(),
            params: json!({ "name": tool_name, "arguments": arguments }),
        };
        let resp = self.mcp_manager.handle_request(req).await;
        if let Some(err) = resp.error {
            return WsResponse::err(id, 500, err.message);
        }
        WsResponse::ok(id, resp.result.unwrap_or(json!({})))
    }

    async fn device_mcp_tools(&self, id: String, path: &str) -> WsResponse {
        use std::collections::HashSet;

        let device_id = parse_trailing_id(path, "/mcp-tools");
        let mut entries = self.mcp_manager.list_all_tool_entries().await;
        let existing: HashSet<_> = entries.iter().map(|t| t.name.clone()).collect();
        let mut device_ready = false;

        if let Some(mgr) = self.chat_registry.get(&device_id) {
            let device_tools = mgr.list_device_mcp_tools().await;
            device_ready = mgr.is_device_mcp_ready().await;
            for tool in device_tools {
                if existing.contains(&tool.name) {
                    continue;
                }
                entries.push(xiaozhi_mcp::McpToolEntry {
                    name: tool.name,
                    description: tool.description,
                    input_schema: tool.input_schema,
                    server_name: xiaozhi_mcp::DEVICE_MCP_SERVER.to_string(),
                });
            }
        }

        let tool_groups = xiaozhi_mcp::McpManager::group_tool_entries(&entries);
        let tools: Vec<Value> = entries
            .iter()
            .map(|entry| {
                json!({
                    "name": entry.name,
                    "description": entry.description,
                    "input_schema": entry.input_schema,
                    "server_name": entry.server_name,
                })
            })
            .collect();

        WsResponse::ok(
            id,
            json!({
                "tools": tools,
                "tool_groups": tool_groups,
                "device_id": device_id,
                "device_mcp_ready": device_ready,
                "online": self.chat_registry.get(&device_id).is_some(),
            }),
        )
    }

    async fn device_mcp_call(&self, id: String, path: &str, body: Value) -> WsResponse {
        let device_id = parse_trailing_id(path, "/mcp-call");
        let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let arguments = body.get("arguments").cloned().unwrap_or(json!({}));
        if tool_name.is_empty() {
            return WsResponse::err(id, 400, "tool_name 不能为空");
        }

        if let Some(mgr) = self.chat_registry.get(&device_id) {
            let result = mgr.execute_tool(tool_name, arguments.clone()).await;
            return WsResponse::ok(
                id,
                json!({
                    "device_id": device_id,
                    "tool_name": tool_name,
                    "arguments": arguments,
                    "result": result,
                }),
            );
        }

        WsResponse::err(id, 404, format!("设备 {device_id} 不在线"))
    }

    async fn inject_message(&self, id: String, body: Value) -> WsResponse {
        self.config_provider.invoke_message_handlers(
            events::HANDLE_MESSAGE_INJECT,
            &json_value_to_map(&body),
        );

        let device_id = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
        if device_id.is_empty() || message.is_empty() {
            return WsResponse::err(id, 400, "device_id 和 message 不能为空");
        }
        let skip_llm = body.get("skip_llm").and_then(|v| v.as_bool()).unwrap_or(false);
        let auto_listen = body.get("auto_listen").and_then(|v| v.as_bool()).unwrap_or(true);
        let audio_route = body
            .get("target")
            .or_else(|| body.get("audio_route"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(parse_tts_audio_route);

        if let Some(mgr) = self.chat_registry.get(device_id) {
            match mgr
                .inject_message_with_route(message, skip_llm, auto_listen, audio_route)
                .await
            {
                Ok((sent, count)) => {
                    return WsResponse::ok(
                        id,
                        json!({
                            "success": sent,
                            "online": true,
                            "messages_sent": count,
                            "device_id": device_id,
                            "message": message,
                            "skip_llm": skip_llm,
                            "auto_listen": auto_listen,
                            "target": audio_route.map(|r| r.as_str()),
                        }),
                    );
                }
                Err(e) => {
                    return WsResponse::err(id, 500, e.to_string());
                }
            }
        }

        self.openclaw
            .queue_offline_message(device_id, message.to_string());
        WsResponse::ok(
            id,
            json!({
                "success": true,
                "online": false,
                "queued": true,
                "device_id": device_id,
                "message": message,
            }),
        )
    }

    async fn device_endpoints(&self, id: String, body: Value) -> WsResponse {
        let device_id = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        if device_id.is_empty() {
            return WsResponse::err(id, 400, "device_id 不能为空");
        }

        WsResponse::ok(id, self.device_endpoints_snapshot(device_id).await)
    }

    async fn device_endpoints_snapshot(&self, device_id: &str) -> Value {
        let mqtt_online = self
            .mqtt_runtime
            .device_gateway()
            .is_broker_online(device_id)
            .await;
        let has_udp = self
            .mqtt_runtime
            .device_gateway()
            .has_active_udp_session(device_id)
            .await;

        if let Some(mgr) = self.chat_registry.get(device_id) {
            let mut snapshot = mgr.endpoint_snapshot();
            let runtime = mgr.debug_runtime_snapshot().await;
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("success".to_string(), json!(true));
                if let Some(runtime_obj) = runtime.as_object() {
                    for (k, v) in runtime_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
                if let Some(v) = mqtt_online {
                    obj.insert("mqtt_broker_online".to_string(), json!(v));
                }
                if let Some(v) = has_udp {
                    obj.insert("has_udp_session".to_string(), json!(v));
                }
            }
            return snapshot;
        }

        let mut offline = json!({
            "success": true,
            "device_id": device_id,
            "online": false,
            "endpoint_count": 0,
            "has_hardware": false,
            "has_web": false,
            "tts_audio_route": "hardware_first",
            "endpoints": [],
            "hello_inited": false,
            "needs_fresh_hello": true,
            "session_active": false,
            "listen_phase": "idle",
            "tts_active": false,
            "is_speaking": false,
            "is_listening": false,
            "injected_speech_guard": false,
            "is_mqtt_transport": false,
        });
        if let Some(obj) = offline.as_object_mut() {
            if let Some(v) = mqtt_online {
                obj.insert("mqtt_broker_online".to_string(), json!(v));
            }
            if let Some(v) = has_udp {
                obj.insert("has_udp_session".to_string(), json!(v));
            }
        }
        offline
    }

    async fn device_endpoints_batch(&self, id: String, body: Value) -> WsResponse {
        let device_ids: Vec<String> = body
            .get("device_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let mut devices = serde_json::Map::new();
        for device_id in device_ids {
            let snapshot = self.device_endpoints_snapshot(&device_id).await;
            devices.insert(device_id, snapshot);
        }

        WsResponse::ok(id, json!({ "devices": devices }))
    }

    async fn device_signals(&self, id: String, body: Value) -> WsResponse {
        let device_id = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        if device_id.is_empty() {
            return WsResponse::err(id, 400, "device_id 不能为空");
        }
        if body.get("clear").and_then(|v| v.as_bool()) == Some(true) {
            if let Some(mgr) = self.chat_registry.get(device_id) {
                mgr.clear_signal_log().await;
            }
            return WsResponse::ok(
                id,
                json!({
                    "device_id": device_id,
                    "cleared": true,
                    "signals": [],
                }),
            );
        }
        let after_id = body.get("after_id").and_then(|v| v.as_u64()).unwrap_or(0);
        let signals = if let Some(mgr) = self.chat_registry.get(device_id) {
            mgr.signal_log_since(after_id).await
        } else {
            Vec::new()
        };
        WsResponse::ok(
            id,
            json!({
                "device_id": device_id,
                "signals": signals,
            }),
        )
    }

    async fn device_abort(&self, id: String, body: Value) -> WsResponse {
        let device_id = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        if device_id.is_empty() {
            return WsResponse::err(id, 400, "device_id 不能为空");
        }
        let Some(mgr) = self.chat_registry.get(device_id) else {
            return WsResponse::err(id, 404, format!("设备 {device_id} 不在线"));
        };
        mgr.on_abort(xiaozhi_chat::detect::AbortOrigin::Explicit).await;
        WsResponse::ok(
            id,
            json!({
                "success": true,
                "device_id": device_id,
                "action": "abort",
            }),
        )
    }

    async fn device_goodbye(&self, id: String, body: Value) -> WsResponse {
        let device_id = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        if device_id.is_empty() {
            return WsResponse::err(id, 400, "device_id 不能为空");
        }
        let Some(mgr) = self.chat_registry.get(device_id) else {
            return WsResponse::err(id, 404, format!("设备 {device_id} 不在线"));
        };
        mgr.request_explicit_goodbye().await;
        WsResponse::ok(
            id,
            json!({
                "success": true,
                "device_id": device_id,
                "action": "goodbye",
            }),
        )
    }

    async fn device_speak(&self, id: String, body: Value) -> WsResponse {
        let device_id = body.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        let text = body
            .get("text")
            .or_else(|| body.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if device_id.is_empty() || text.is_empty() {
            return WsResponse::err(id, 400, "device_id 和 text 不能为空");
        }
        let target = body
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("hardware_first");
        let auto_listen = body.get("auto_listen").and_then(|v| v.as_bool()).unwrap_or(false);
        let route = parse_tts_audio_route(target);

        if matches!(
            route,
            TtsAudioRoute::HardwareFirst | TtsAudioRoute::HardwareOnly
        ) {
            if let Err(e) = self.mqtt_runtime.prepare_hardware_wake(device_id).await {
                tracing::warn!(
                    device_id = %device_id,
                    error = %e,
                    "硬件唤醒前建立 MQTT 通道失败"
                );
            } else if let Some(false) = self.mqtt_runtime.device_gateway().is_broker_online(device_id).await {
                tracing::warn!(
                    device_id = %device_id,
                    "设备 MQTT broker 离线，speak_request 可能无法送达"
                );
            }
        }

        if let Some(mgr) = self.chat_registry.get(device_id) {
            match mgr.speak(text, route, auto_listen).await {
                Ok((sent, count)) => {
                    return WsResponse::ok(
                        id,
                        json!({
                            "success": sent,
                            "online": true,
                            "messages_sent": count,
                            "device_id": device_id,
                            "text": text,
                            "target": route.as_str(),
                            "auto_listen": auto_listen,
                        }),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    let hint = if msg.contains("speak_ready") {
                        "设备未在超时内回复 speak_ready（请确认 MQTT 在线、固件支持 speak_request，或增大 chat.speak_ready_timeout_ms）"
                    } else {
                        ""
                    };
                    let detail = if hint.is_empty() {
                        msg
                    } else {
                        format!("{msg}。{hint}")
                    };
                    return WsResponse::err(id, 500, detail);
                }
            }
        }

        WsResponse::err(id, 404, format!("设备 {device_id} 不在线"))
    }

    async fn openclaw_status(&self, id: String, body: Value) -> WsResponse {
        let agent_id = body
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if agent_id.is_empty() {
            return WsResponse::err(id, 400, "missing agent_id");
        }
        WsResponse::ok(id, self.openclaw.openclaw_status(agent_id))
    }

    async fn openclaw_chat_test(&self, id: String, path: &str, body: Value) -> WsResponse {
        use xiaozhi_llm::{create_llm, ChatMessage};

        let agent_id = parse_trailing_id(path, "/openclaw-chat-test");
        let user_msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("你好");

        let cfg = self.config.read().await;
        let provider = body
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(&cfg.llm.provider);
        let llm_config = body
            .get("config")
            .cloned()
            .or_else(|| cfg.llm.providers.get(provider).cloned())
            .unwrap_or(json!({}));

        match create_llm(provider, &llm_config) {
            Ok(llm) => {
                let dialogue = vec![
                    ChatMessage::system("你是 OpenClaw 助手，请简洁回复。"),
                    ChatMessage::user(user_msg),
                ];
                match llm
                    .response_with_context("openclaw-test", &dialogue, &[])
                    .await
                {
                    Ok(mut rx) => {
                        let mut reply = String::new();
                        while let Some(msg) = rx.recv().await {
                            reply.push_str(&msg.content);
                        }
                        if reply.is_empty() {
                            WsResponse::ok(
                                id,
                                json!({
                                    "reply": format!("OpenClaw 测试：已连接 LLM ({provider})，但未收到内容"),
                                    "agent_id": agent_id,
                                    "stream": false,
                                }),
                            )
                        } else {
                            WsResponse::ok(
                                id,
                                json!({
                                    "reply": reply,
                                    "agent_id": agent_id,
                                    "stream": false,
                                }),
                            )
                        }
                    }
                    Err(e) => WsResponse::err(id, 500, e.to_string()),
                }
            }
            Err(e) => WsResponse::err(id, 500, format!("LLM 创建失败: {e}")),
        }
    }

    async fn config_test(&self, id: String, path: &str, body: Value) -> WsResponse {
        let kind = path.trim_start_matches("/ws/test/");
        let provider = body
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let config = body.get("config").cloned().unwrap_or(body.clone());

        let result = config_test::run_config_test(kind, provider, &config).await;
        WsResponse::ok(
            id,
            json!({
                "ok": result.ok,
                "message": result.message,
                "first_packet_ms": result.first_packet_ms,
            }),
        )
    }
}

fn parse_trailing_id(path: &str, suffix: &str) -> String {
    path.trim_end_matches(suffix)
        .rsplit('/')
        .next()
        .unwrap_or("0")
        .to_string()
}

fn json_value_to_map(value: &Value) -> HashMap<String, Value> {
    value
        .as_object()
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}
