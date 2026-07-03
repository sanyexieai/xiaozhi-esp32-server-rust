use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use xiaozhi_chat::{run_device_mcp_init_json, ChatManager, ChatManagerRegistry, McpInboundAction, SharedResourcePools};
use xiaozhi_history::HistoryClient;
use xiaozhi_mcp::McpManager;
use xiaozhi_openclaw::OpenClawManager;
use xiaozhi_rag::KnowledgeClient;

use crate::shared_config::SharedAppConfig;

const MCP_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const MCP_PING_INTERVAL: Duration = Duration::from_secs(30);

pub struct McpWsState {
    pub shared_config: SharedAppConfig,
    pub chat_registry: Arc<ChatManagerRegistry>,
    pub config_provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,
    pub history: Arc<HistoryClient>,
    pub openclaw: Arc<OpenClawManager>,
    pub mcp_manager: Arc<McpManager>,
    pub knowledge_client: Arc<KnowledgeClient>,
    pub resource_pools: Arc<SharedResourcePools>,
    pub connections: DashMap<String, Arc<AtomicU32>>,
}

impl McpWsState {
    pub fn from_server(
        shared_config: SharedAppConfig,
        chat_registry: Arc<ChatManagerRegistry>,
        config_provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,
        history: Arc<HistoryClient>,
        openclaw: Arc<OpenClawManager>,
        mcp_manager: Arc<McpManager>,
        knowledge_client: Arc<KnowledgeClient>,
        resource_pools: Arc<SharedResourcePools>,
    ) -> Arc<Self> {
        Arc::new(Self {
            shared_config,
            chat_registry,
            config_provider,
            history,
            openclaw,
            mcp_manager,
            knowledge_client,
            resource_pools,
            connections: DashMap::new(),
        })
    }

    fn acquire_connection(&self, device_id: &str, max_connections: u32) -> bool {
        let max = max_connections.max(1);
        let counter = self
            .connections
            .entry(device_id.to_string())
            .or_insert_with(|| Arc::new(AtomicU32::new(0)))
            .clone();
        let current = counter.fetch_add(1, Ordering::Relaxed) + 1;
        if current > max {
            counter.fetch_sub(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn release_connection(&self, device_id: &str) {
        if let Some(counter) = self.connections.get(device_id) {
            counter.fetch_sub(1, Ordering::Relaxed);
        }
    }

    async fn ensure_chat_manager(&self, device_id: &str) -> Result<Arc<ChatManager>, String> {
        if let Some(mgr) = self.chat_registry.get(device_id) {
            return Ok(mgr);
        }
        let app_config = self.shared_config.read().await.clone();
        let mgr = ChatManager::new(
            device_id.to_string(),
            app_config,
            self.config_provider.clone(),
            self.history.clone(),
            self.openclaw.clone(),
            self.mcp_manager.clone(),
            self.knowledge_client.clone(),
            Some(self.resource_pools.clone()),
        )
        .await
        .map_err(|e| e.to_string())?;
        let mgr = Arc::new(mgr);
        self.chat_registry
            .insert_manager(device_id.to_string(), mgr.clone());
        Ok(mgr)
    }
}

pub async fn mcp_ws_handler(
    ws: WebSocketUpgrade,
    Path(device_id): Path<String>,
    State(state): State<Arc<McpWsState>>,
) -> impl IntoResponse {
    let device_enabled = state
        .shared_config
        .read()
        .await
        .mcp
        .device
        .enabled;
    if !device_enabled {
        return ws.on_upgrade(|mut socket| async move {
            let _ = socket.send(Message::Close(None)).await;
        });
    }
    ws.on_upgrade(move |socket| handle_mcp_socket(socket, device_id, state))
}

async fn handle_mcp_socket(socket: WebSocket, device_id: String, state: Arc<McpWsState>) {
    let runtime_cfg = state.shared_config.read().await.clone();
    if !state.acquire_connection(
        &device_id,
        runtime_cfg.mcp.device.max_connections_per_device,
    ) {
        tracing::warn!("设备 {device_id} MCP 连接数已达上限");
        return;
    }

    let chat_mgr = match state.ensure_chat_manager(&device_id).await {
        Ok(mgr) => mgr,
        Err(e) => {
            tracing::error!("设备 {device_id} 创建 ChatManager 失败: {e}");
            state.release_connection(&device_id);
            return;
        }
    };

    chat_mgr.set_mcp_session_id(device_id.clone()).await;
    tracing::info!("设备 MCP WebSocket 连接: device={device_id}");

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (mut ws_tx, mut ws_rx) = socket.split();
    let ping_id = Arc::new(AtomicU64::new(10_000));

    let send_task = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if ws_tx.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    if chat_mgr.should_schedule_mcp_init().await && chat_mgr.try_begin_mcp_init().await {
        let vision_url = runtime_cfg.vision.vision_url.clone();
        let pending = chat_mgr.mcp_pending_hub().await;
        let out = out_tx.clone();
        match run_device_mcp_init_json(&vision_url, &pending, move |payload| {
            let out = out.clone();
            async move { out.send(payload.to_string()).is_ok() }
        })
        .await
        {
            Ok(tools) => {
                chat_mgr.mcp_mark_ready(tools).await;
                tracing::info!("设备 {device_id} MCP WS 初始化完成");
            }
            Err(e) => {
                chat_mgr.mcp_mark_failed().await;
                tracing::warn!("设备 {device_id} MCP WS 初始化失败: {e}");
            }
        }
    }

    let mut last_activity = Instant::now();
    let mut ping_timer = tokio::time::interval(MCP_PING_INTERVAL);
    ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ping_timer.tick() => {
                if last_activity.elapsed() >= MCP_IDLE_TIMEOUT {
                    tracing::warn!("设备 {device_id} MCP 连接空闲超时");
                    break;
                }
                let id = ping_id.fetch_add(1, Ordering::Relaxed);
                let ping = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "ping",
                    "params": {}
                });
                let _ = out_tx.send(ping.to_string());
            }
            msg = ws_rx.next() => {
                let Some(msg) = msg else { break };
                let Ok(msg) = msg else { break };
                last_activity = Instant::now();
                match msg {
                    Message::Text(text) => {
                        if let Ok(payload) = serde_json::from_str::<Value>(&text) {
                            if payload.get("method").and_then(|v| v.as_str()) == Some("ping")
                                && payload.get("id").is_some()
                            {
                                let pong = json!({
                                    "jsonrpc": "2.0",
                                    "id": payload.get("id").cloned().unwrap_or(json!(0)),
                                    "result": {}
                                });
                                let _ = out_tx.send(pong.to_string());
                                continue;
                            }
                            match chat_mgr.mcp_handle_inbound(&payload).await {
                                McpInboundAction::None => {}
                                McpInboundAction::Respond(resp) => {
                                    let _ = out_tx.send(resp.to_string());
                                }
                                McpInboundAction::RefreshTools => {
                                    let out = out_tx.clone();
                                    if let Err(e) = chat_mgr
                                        .refresh_tools_over_json(move |payload| {
                                            let out = out.clone();
                                            async move { out.send(payload.to_string()).is_ok() }
                                        })
                                        .await
                                    {
                                        tracing::warn!("设备 {device_id} MCP 工具刷新失败: {e}");
                                    }
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(data) => {
                        let pong = json!({
                            "jsonrpc": "2.0",
                            "method": "pong",
                            "params": { "data": String::from_utf8_lossy(&data) }
                        });
                        let _ = out_tx.send(pong.to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    drop(out_tx);
    let _ = send_task.await;
    state.release_connection(&device_id);
    tracing::info!("设备 MCP WebSocket 断开: device={device_id}");
}
