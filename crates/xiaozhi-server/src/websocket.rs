use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use dashmap::DashMap;
use serde_json::json;
use tower_http::cors::CorsLayer;
use xiaozhi_chat::{ChatManager, ChatManagerRegistry, EndpointKind, OutboundFrame, SharedResourcePools};
use xiaozhi_config::user::ActivationPayload;
use xiaozhi_config::OtaEnvironment;
use xiaozhi_config_provider::events;
use xiaozhi_history::HistoryClient;
use xiaozhi_openclaw::OpenClawManager;
use xiaozhi_protocol::mqtt;
use xiaozhi_protocol::messages::{
    ClientMessage, OtaFirmware, OtaMqtt, OtaResponse, OtaServerTime, OtaWebsocket,
};

use crate::shared_config::SharedAppConfig;

pub struct WebSocketServer {
    config: SharedAppConfig,
    chat_registry: Arc<ChatManagerRegistry>,
    config_provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,
    history: Arc<HistoryClient>,
    openclaw: Arc<OpenClawManager>,
    mcp_manager: Arc<xiaozhi_mcp::McpManager>,
    knowledge_client: Arc<xiaozhi_rag::KnowledgeClient>,
    resource_pools: Arc<SharedResourcePools>,
}

impl WebSocketServer {
    pub fn new(
        config: SharedAppConfig,
        chat_registry: Arc<ChatManagerRegistry>,
        config_provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,
        history: Arc<HistoryClient>,
        openclaw: Arc<OpenClawManager>,
        mcp_manager: Arc<xiaozhi_mcp::McpManager>,
        knowledge_client: Arc<xiaozhi_rag::KnowledgeClient>,
        resource_pools: Arc<SharedResourcePools>,
    ) -> Self {
        Self {
            config,
            chat_registry,
            config_provider,
            history,
            openclaw,
            mcp_manager,
            knowledge_client,
            resource_pools,
        }
    }

    pub async fn start(self) -> anyhow::Result<()> {
        let boot_cfg = self.config.read().await.clone();
        let state = Arc::new(ServerState {
            config: self.config.clone(),
            chat_registry: self.chat_registry,
            config_provider: self.config_provider,
            history: self.history,
            openclaw: self.openclaw,
            mcp_manager: self.mcp_manager,
            knowledge_client: self.knowledge_client,
            resource_pools: self.resource_pools,
            device_conn_gen: DashMap::new(),
        });

        let vision_state = Arc::new(crate::vision::VisionState {
            config: self.config.clone(),
        });

        let mcp_state = crate::mcp_ws::McpWsState::from_server(
            self.config.clone(),
            state.chat_registry.clone(),
            state.config_provider.clone(),
            state.history.clone(),
            state.openclaw.clone(),
            state.mcp_manager.clone(),
            state.knowledge_client.clone(),
            state.resource_pools.clone(),
        );

        let mcp_api_state = Arc::new(crate::mcp_api::McpApiState {
            chat_registry: state.chat_registry.clone(),
            mcp_manager: state.mcp_manager.clone(),
        });

        let openclaw_state = crate::openclaw_ws::OpenClawWsState {
            shared_config: self.config.clone(),
            openclaw: state.openclaw.clone(),
        };

        let app = Router::new()
            .route("/xiaozhi/v1/", get(ws_handler))
            .route("/xiaozhi/v1", get(ws_handler))
            .route("/xiaozhi/mqtt_udp/v1/", get(ws_handler))
            .route("/xiaozhi/ota/", post(ota_handler))
            // 部分配网/旧固件 OTA URL 无尾斜杠，404 会导致设备 OTA 失败并回退 MQTT，卡在「登录服务器」
            .route("/xiaozhi/ota", post(ota_handler))
            .route("/xiaozhi/ota/activate", post(activate_handler))
            .route("/admin/inject_msg", post(inject_msg))
            .layer(CorsLayer::permissive())
            .with_state(state)
            .merge(
                Router::new()
                    .route("/xiaozhi/api/vision", post(crate::vision::vision_handler))
                    .with_state(vision_state),
            )
            .merge(
                Router::new()
                    .route("/xiaozhi/mcp/{device_id}", get(crate::mcp_ws::mcp_ws_handler))
                    .with_state(mcp_state),
            )
            .merge({
                Router::new()
                    .route(
                        "/xiaozhi/api/mcp/tools/{device_id}",
                        get(crate::mcp_api::list_device_mcp_tools),
                    )
                    .with_state(mcp_api_state)
            })
            .merge(
                Router::new()
                    .route(
                        "/ws/openclaw/{agent_id}",
                        get(crate::openclaw_ws::openclaw_ws_handler),
                    )
                    .with_state(openclaw_state),
            );

        let (listener, addr) =
            bind_listener(&boot_cfg.websocket.host, boot_cfg.websocket.port).await?;

        tracing::info!("HTTP/WebSocket 服务监听: {addr}");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        Ok(())
    }
}

/// 绑定 TCP 端口；若配置端口被占用则依次尝试后续端口
async fn bind_listener(
    host: &str,
    port: u16,
) -> anyhow::Result<(tokio::net::TcpListener, SocketAddr)> {
    const MAX_OFFSET: u16 = 20;

    for offset in 0..=MAX_OFFSET {
        let try_port = port.saturating_add(offset);
        let addr: SocketAddr = format!("{host}:{try_port}")
            .parse()
            .map_err(|e| anyhow::anyhow!("地址解析失败: {e}"))?;

        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                if offset > 0 {
                    tracing::warn!("端口 {port} 已被占用，已自动切换到 {try_port}");
                }
                return Ok((listener, addr));
            }
            Err(e) if is_addr_in_use(&e) => {
                tracing::debug!("端口 {try_port} 已被占用，尝试下一个");
            }
            Err(e) => return Err(e.into()),
        }
    }

    anyhow::bail!("端口 {port} ~ {} 均已被占用，请修改 config.yaml 中的 websocket.port", port + MAX_OFFSET)
}

fn is_addr_in_use(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::AddrInUse
        || (cfg!(windows) && err.raw_os_error() == Some(10048))
}

struct ServerState {
    config: SharedAppConfig,
    chat_registry: Arc<ChatManagerRegistry>,
    config_provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,
    history: Arc<HistoryClient>,
    openclaw: Arc<OpenClawManager>,
    mcp_manager: Arc<xiaozhi_mcp::McpManager>,
    knowledge_client: Arc<xiaozhi_rag::KnowledgeClient>,
    resource_pools: Arc<SharedResourcePools>,
    device_conn_gen: DashMap<String, Arc<AtomicU64>>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, headers))
}

async fn handle_socket(
    socket: WebSocket,
    state: Arc<ServerState>,
    headers: axum::http::HeaderMap,
) {
    let device_id = headers
        .get("Device-Id")
        .or(headers.get("device-id"))
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let protocol_version = headers
        .get("Protocol-Version")
        .or(headers.get("protocol-version"))
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(1);
    tracing::info!(
        "WebSocket 连接: device={device_id} protocol_version={protocol_version}"
    );

    let conn_gen = state
        .device_conn_gen
        .entry(device_id.clone())
        .or_insert_with(|| Arc::new(AtomicU64::new(0)))
        .clone();
    let my_gen = conn_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let endpoint_id = format!("web-{my_gen}");

    let app_config = state.config.read().await.clone();
    let created_new_manager = state.chat_registry.get(&device_id).is_none();
    let chat_mgr = if let Some(existing) = state.chat_registry.get(&device_id) {
        tracing::info!(
            device_id = %device_id,
            endpoint = %endpoint_id,
            endpoints = existing.endpoint_count(),
            "WebSocket 附着到已有 ChatManager（多端在线）"
        );
        existing
    } else {
        match ChatManager::new(
            device_id.clone(),
            app_config,
            state.config_provider.clone(),
            state.history.clone(),
            state.openclaw.clone(),
            state.mcp_manager.clone(),
            state.knowledge_client.clone(),
            Some(state.resource_pools.clone()),
        )
        .await
        {
            Ok(mgr) => {
                let mgr = Arc::new(mgr);
                state
                    .chat_registry
                    .insert_manager(device_id.clone(), mgr.clone());
                mgr
            }
            Err(e) => {
                tracing::error!("创建 ChatManager 失败: {e}");
                return;
            }
        }
    };

    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<OutboundFrame>();
    if created_new_manager {
        chat_mgr.set_mqtt_transport(false);
        chat_mgr.set_binary_protocol_version(protocol_version);
        chat_mgr.spawn_replay_openclaw_offline_messages();
    }
    chat_mgr
        .register_endpoint(endpoint_id.clone(), EndpointKind::Web, out_tx.clone());

    let (mut ws_tx, mut ws_rx) = socket.split();

    let config_provider = state.config_provider.clone();
    let presence_device_id = device_id.clone();
    let skip_presence = xiaozhi_core::constants::ota_test::is_probe_device(&device_id)
        || xiaozhi_core::constants::simulator::is_simulator_device(&device_id);
    if !skip_presence {
        let mut event_data = HashMap::new();
        event_data.insert("device_id".to_string(), json!(presence_device_id));
        config_provider.notify_device_event(events::DEVICE_ONLINE, event_data);
    }

    let chat_mgr_msg = chat_mgr.clone();
    let out_tx_listen = out_tx;
    let conn_gen_recv = conn_gen.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.next().await {
            if conn_gen_recv.load(Ordering::SeqCst) != my_gen {
                tracing::info!(
                    device_id = %chat_mgr_msg.device_id(),
                    "旧 WebSocket 连接已被新连接取代，停止接收"
                );
                break;
            }
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                        if client_msg.msg_type == xiaozhi_core::message::HELLO {
                            tracing::info!(
                                device_id = %chat_mgr_msg.device_id(),
                                "收到设备 hello"
                            );
                        }
                        chat_mgr_msg
                            .record_inbound_client("ws", &client_msg)
                            .await;
                        let responses = crate::device_handler::process_client_message(
                            &chat_mgr_msg,
                            client_msg,
                            None,
                        )
                        .await;
                        for resp in responses {
                            if let Ok(data) = serde_json::to_vec(&resp) {
                                let _ = out_tx_listen.send(OutboundFrame::Command(data));
                            }
                        }
                    } else {
                        tracing::warn!(
                            device_id = %chat_mgr_msg.device_id(),
                            text = %text.chars().take(200).collect::<String>(),
                            "无法解析设备 JSON 消息"
                        );
                    }
                }
                Ok(Message::Binary(data)) => {
                    if let Err(e) = chat_mgr_msg.handle_audio(&data).await {
                        tracing::warn!(
                            device_id = %chat_mgr_msg.device_id(),
                            "处理音频失败: {e}"
                        );
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    let conn_gen_send = conn_gen.clone();
    let send_task = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            if conn_gen_send.load(Ordering::SeqCst) != my_gen {
                break;
            }
            let result = match frame {
                OutboundFrame::Command(data) => {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    ws_tx.send(Message::Text(text.into())).await
                }
                OutboundFrame::Audio(data) => ws_tx.send(Message::Binary(data.into())).await,
            };
            if result.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(recv_task, send_task);
    if conn_gen.load(Ordering::SeqCst) == my_gen {
        let endpoint_empty = chat_mgr.unregister_endpoint(&endpoint_id);
        tracing::info!(
            device_id = %device_id,
            endpoint = %endpoint_id,
            endpoint_empty,
            remaining = chat_mgr.endpoint_count(),
            "WebSocket endpoint 已注销"
        );
        let retain_session = xiaozhi_core::constants::ota_test::is_probe_device(&device_id)
            || xiaozhi_core::constants::simulator::is_simulator_device(&device_id);
        if endpoint_empty && !retain_session {
            state
                .chat_registry
                .remove_and_shutdown(&device_id)
                .await;
        }
        if endpoint_empty && !retain_session {
            let mut event_data = HashMap::new();
            event_data.insert("device_id".to_string(), json!(device_id));
            state
                .config_provider
                .notify_device_event(events::DEVICE_OFFLINE, event_data);
        }
        state.device_conn_gen.remove(&device_id);
    }
    tracing::info!("WebSocket 断开: device={device_id} conn_gen={my_gen}");
}


async fn ota_handler(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let cfg = state.config.read().await;
    let body = body.map(|Json(v)| v);

    let device_id = headers
        .get("Device-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            body.as_ref()
                .and_then(|b| b.get("mac_address").and_then(|v| v.as_str()))
                .map(str::to_string)
        })
        .or_else(|| {
            body.as_ref()
                .and_then(|b| b.get("mac").and_then(|v| v.as_str()))
                .map(str::to_string)
        })
        .unwrap_or_default();

    let client_id = headers
        .get("Client-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            body.as_ref()
                .and_then(|b| b.get("uuid").and_then(|v| v.as_str()))
                .map(str::to_string)
        })
        .unwrap_or_default();

    if device_id.is_empty() || client_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "缺少 Device-Id 或 Client-Id",
        )
            .into_response();
    }

    let client_ip = client_ip_from_headers(&headers, peer);
    let (ota_env, ota_env_label) = select_ota_env_for_request(&cfg.ota, &client_ip, &headers);

    tracing::info!(
        device_id = %device_id,
        client_id = %client_id,
        client_ip = %client_ip,
        ota_env = ota_env_label,
        "OTA 请求"
    );

    let ws_url = resolve_ota_websocket_url(
        ota_env.websocket.url.trim(),
        &client_ip,
        &cfg.websocket.host,
        cfg.websocket.port,
    );

        let ws_token = ota_env.websocket.token.trim().to_string();
    let mut response = OtaResponse {
        websocket: OtaWebsocket {
            url: ws_url.clone(),
            token: ws_token,
        },
        mqtt: None,
        server_time: OtaServerTime {
            timestamp: chrono::Utc::now().timestamp_millis(),
            timezone_offset: 480,
        },
        firmware: OtaFirmware {
            version: "0.9.9".to_string(),
            url: String::new(),
        },
        activation: None,
    };

    let server_host = resolve_ota_server_host(&headers, &cfg, ota_env);

    if should_ota_include_mqtt(&cfg, ota_env, &server_host) {
        let configured = ota_env.mqtt.endpoint.trim();
        let listen_port = cfg.mqtt_server.listen_port;
        let endpoint = if configured.is_empty() {
            format!("{server_host}:{listen_port}")
        } else {
            ensure_mqtt_endpoint_port(
                &resolve_ota_mqtt_endpoint(configured, &server_host, listen_port),
                listen_port,
            )
        };

        match xiaozhi_auth::generate_go_mqtt_credentials(
            &device_id,
            &client_id,
            &client_ip,
            cfg.ota.signature_key.trim(),
        ) {
            Ok(creds) => {
                response.mqtt = Some(OtaMqtt {
                    endpoint,
                    client_id: creds.client_id,
                    username: creds.username,
                    password: creds.password,
                    publish_topic: mqtt::DEVICE_MOCK_PUB_TOPIC.to_string(),
                    subscribe_topic: mqtt::DEVICE_MOCK_SUB_TOPIC.to_string(),
                });
            }
            Err(e) => {
                tracing::error!(
                    device_id = %device_id,
                    client_id = %client_id,
                    "生成 OTA MQTT 凭据失败: {e}"
                );
            }
        }
    }

    let activated = if !cfg.auth.enable {
        true
    } else {
        match state
            .config_provider
            .is_device_activated(&device_id, &client_id)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    device_id = %device_id,
                    client_id = %client_id,
                    "查询设备激活状态失败，跳过 OTA 激活流程以免阻断已绑定设备: {e}"
                );
                true
            }
        }
    };

    if !activated && cfg.auth.enable
        && !xiaozhi_core::constants::ota_test::is_probe_device(&device_id)
        && !xiaozhi_core::constants::simulator::is_simulator_device(&device_id)
    {
        match state
            .config_provider
            .get_activation_info(&device_id, &client_id)
            .await
        {
            Ok((code, message, challenge, _)) => {
                if !code.trim().is_empty() || !challenge.trim().is_empty() {
                    response.activation = Some(xiaozhi_protocol::messages::OtaActivation {
                        code,
                        message,
                        challenge,
                    });
                } else {
                    tracing::warn!(
                        device_id = %device_id,
                        client_id = %client_id,
                        "激活信息为空 challenge/code，跳过 OTA activation 块以免设备陷入激活循环"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    device_id = %device_id,
                    client_id = %client_id,
                    "无法从管理后台获取激活信息，设备将不会收到验证码: {e}"
                );
            }
        }
    }

    tracing::info!(
        device_id = %device_id,
        client_id = %client_id,
        ws_url = %ws_url,
        mqtt_endpoint = response.mqtt.as_ref().map(|m| m.endpoint.as_str()).unwrap_or(""),
        activated = activated,
        has_activation = response.activation.is_some(),
        has_mqtt = response.mqtt.is_some(),
        "OTA 响应"
    );

    let config_provider = state.config_provider.clone();
    let touch_device_id = device_id.clone();
    if !xiaozhi_core::constants::ota_test::is_probe_device(&device_id)
        && !xiaozhi_core::constants::simulator::is_simulator_device(&device_id)
    {
        tokio::spawn(async move {
            if let Err(e) = config_provider
                .touch_device_activity(&touch_device_id)
                .await
            {
                tracing::debug!(
                    device_id = %touch_device_id,
                    "OTA 更新设备活跃时间失败: {e}"
                );
            }
        });
    }

    (
        [(header::CONTENT_TYPE, "application/json")],
        Json(response),
    )
        .into_response()
}

/// 是否与 Go `getMqttInfo` 一致：仅当 OTA 环境启用 MQTT 且确有可用 Broker 时下发 mqtt 块。
/// 固件优先使用 mqtt（`application.cc`），内置 Broker 关闭时不应再下发指向本机的 endpoint。
fn should_ota_include_mqtt(
    cfg: &xiaozhi_config::AppConfig,
    ota_env: &OtaEnvironment,
    server_host: &str,
) -> bool {
    if !ota_env.mqtt.enable {
        return false;
    }
    if cfg.mqtt_server.enable {
        return true;
    }
    let configured = ota_env.mqtt.endpoint.trim();
    if configured.is_empty() {
        tracing::info!(
            server_host = %server_host,
            "内置 MQTT Server 已关闭且 OTA 未配置外部 endpoint，不下发 mqtt，设备将走 WebSocket"
        );
        return false;
    }
    let listen_port = cfg.mqtt_server.listen_port;
    let resolved = resolve_ota_mqtt_endpoint(configured, server_host, listen_port);
    let builtin = format!("{server_host}:{listen_port}");
    if resolved == builtin {
        tracing::info!(
            server_host = %server_host,
            endpoint = %resolved,
            "内置 MQTT Server 已关闭，OTA endpoint 指向本机 Broker，不下发 mqtt，设备将走 WebSocket"
        );
        return false;
    }
    true
}

fn ensure_mqtt_endpoint_port(endpoint: &str, default_port: u16) -> String {
    if endpoint.contains(':') {
        endpoint.to_string()
    } else {
        format!("{endpoint}:{default_port}")
    }
}

const OTA_TEST_ENV_HEADER: &str = "X-Ota-Test-Env";

fn select_ota_env_for_request<'a>(
    ota: &'a xiaozhi_config::OtaConfig,
    client_ip: &str,
    headers: &HeaderMap,
) -> (&'a xiaozhi_config::OtaEnvironment, &'static str) {
    if let Some(env) = headers.get(OTA_TEST_ENV_HEADER).and_then(|v| v.to_str().ok()) {
        match env.trim().to_ascii_lowercase().as_str() {
            "test" => return (&ota.test, "test"),
            "external" => return (&ota.external, "external"),
            _ => {}
        }
    }
    if xiaozhi_config::is_private_client_ip(client_ip) {
        (&ota.test, "test")
    } else {
        (&ota.external, "external")
    }
}

fn resolve_ota_websocket_url(configured: &str, client_ip: &str, ws_host: &str, ws_port: u16) -> String {
    if configured.is_empty() {
        let host = if ws_host == "0.0.0.0" || ws_host.is_empty() {
            if client_ip.is_empty() {
                "127.0.0.1".to_string()
            } else {
                client_ip.to_string()
            }
        } else {
            ws_host.to_string()
        };
        return format!("ws://{host}:{ws_port}/xiaozhi/v1/");
    }

    if xiaozhi_config::is_private_client_ip(client_ip) && url_host_is_loopback(configured) {
        if let Some(resolved) = rewrite_url_host(configured, client_ip) {
            tracing::info!(
                client_ip = %client_ip,
                configured = %configured,
                resolved = %resolved,
                "OTA WebSocket 地址已从 loopback 重写为客户端 IP"
            );
            return resolved;
        }
    }

    configured.to_string()
}

/// OTA 下发的 MQTT/UDP 地址必须是**服务端**可达地址，不能使用设备 client_ip。
fn resolve_ota_server_host(
    headers: &HeaderMap,
    cfg: &xiaozhi_config::AppConfig,
    ota_env: &OtaEnvironment,
) -> String {
    if let Some(host) = headers
        .get("host")
        .or(headers.get("Host"))
        .and_then(|v| v.to_str().ok())
    {
        let host = host.split(':').next().unwrap_or(host).trim();
        if !host.is_empty() && !host_is_loopback(host) && host != "0.0.0.0" {
            return host.to_string();
        }
    }

    let ws_url = ota_env.websocket.url.trim();
    if !ws_url.is_empty() {
        if let Some(host) = extract_url_host(ws_url) {
            if !host_is_loopback(&host) && host != "0.0.0.0" {
                return host;
            }
        }
    }

    let udp_host = cfg.udp.external_host.trim();
    if !udp_host.is_empty() && udp_host != "0.0.0.0" && !host_is_loopback(udp_host) {
        return udp_host.to_string();
    }

    let ws_host = cfg.websocket.host.trim();
    if !ws_host.is_empty() && ws_host != "0.0.0.0" {
        return ws_host.to_string();
    }

    "127.0.0.1".to_string()
}

fn extract_url_host(url: &str) -> Option<String> {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let host_port = without_scheme.split('/').next()?;
    let host = host_port
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_port);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn resolve_ota_mqtt_endpoint(endpoint: &str, server_host: &str, listen_port: u16) -> String {
    let host = endpoint
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(endpoint);
    if mqtt_host_is_loopback(endpoint) || host == server_host {
        let resolved = format!("{server_host}:{listen_port}");
        if resolved != endpoint {
            tracing::info!(
                configured = %endpoint,
                resolved = %resolved,
                "OTA MQTT 端点已对齐为服务端地址与当前 Broker 端口"
            );
        }
        return resolved;
    }
    endpoint.to_string()
}

fn url_host_is_loopback(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost") || url.contains("[::1]")
}

fn mqtt_host_is_loopback(endpoint: &str) -> bool {
    let host = endpoint
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(endpoint);
    host_is_loopback(host)
}

fn host_is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn rewrite_url_host(url: &str, new_host: &str) -> Option<String> {
    for loopback in ["127.0.0.1", "localhost", "[::1]"] {
        if let Some(idx) = url.find(loopback) {
            let mut out = String::with_capacity(url.len() + new_host.len());
            out.push_str(&url[..idx]);
            out.push_str(new_host);
            out.push_str(&url[idx + loopback.len()..]);
            return Some(out);
        }
    }
    None
}

fn client_ip_from_headers(headers: &HeaderMap, peer: SocketAddr) -> String {
    if let Some(ip) = headers
        .get("X-Real-IP")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return strip_socket_port(ip);
    }
    if let Some(ip) = headers
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return strip_socket_port(ip);
    }
    peer.ip().to_string()
}

fn strip_socket_port(ip: &str) -> String {
    if ip.starts_with('[') {
        return ip.to_string();
    }
    ip.rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|_| host.to_string()))
        .unwrap_or_else(|| ip.to_string())
}

#[derive(serde::Deserialize)]
struct ActivateRequestWrapped {
    #[serde(alias = "Payload")]
    payload: ActivationPayload,
}

async fn activate_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Json<serde_json::Value>,
) -> impl IntoResponse {
    let device_id = headers
        .get("Device-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_string();
    let client_id = headers
        .get("Client-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_string();

    if device_id.is_empty() || client_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "缺少 Device-Id 或 Client-Id").into_response();
    }

    let payload = if let Ok(wrapped) = serde_json::from_value::<ActivateRequestWrapped>(body.0.clone())
    {
        wrapped.payload
    } else {
        serde_json::from_value::<ActivationPayload>(body.0).unwrap_or_default()
    };

    let ok = state
        .config_provider
        .verify_challenge(&device_id, &client_id, payload)
        .await
        .unwrap_or(false);

    if ok {
        (StatusCode::OK, "激活成功").into_response()
    } else {
        (StatusCode::ACCEPTED, "设备激活校验未通过").into_response()
    }
}

async fn inject_msg(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let device_id = body["device_id"].as_str().unwrap_or("");
    let message = body["message"]
        .as_str()
        .or_else(|| body["text"].as_str())
        .unwrap_or("");
    if device_id.is_empty() || message.is_empty() {
        return Json(serde_json::json!({"success": false, "error": "device_id 和 message 不能为空"}));
    }
    let skip_llm = body["skip_llm"].as_bool().unwrap_or(false);
    let auto_listen = body["auto_listen"].as_bool().unwrap_or(true);
    let audio_route = body
        .get("target")
        .or_else(|| body.get("audio_route"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| xiaozhi_chat::parse_tts_audio_route(s));

    if let Some(mgr) = state.chat_registry.get(device_id) {
        match mgr
            .inject_message_with_route(message, skip_llm, auto_listen, audio_route)
            .await
        {
            Ok((sent, count)) => {
                return Json(serde_json::json!({
                    "success": sent,
                    "online": true,
                    "messages_sent": count,
                    "target": audio_route.map(|r| r.as_str()),
                }));
            }
            Err(e) => {
                return Json(serde_json::json!({"success": false, "error": e.to_string()}));
            }
        }
    }

    state
        .openclaw
        .queue_offline_message(device_id, message.to_string());
    Json(serde_json::json!({"success": true, "online": false, "queued": true}))
}

#[cfg(test)]
mod ota_mqtt_tests {
    use super::*;
    use xiaozhi_config::{AppConfig, OtaEnvironment, OtaMqttConfig};

    fn ota_env(mqtt_enable: bool, endpoint: &str) -> OtaEnvironment {
        OtaEnvironment {
            mqtt: OtaMqttConfig {
                enable: mqtt_enable,
                endpoint: endpoint.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn cfg_with_mqtt_server(enable: bool) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.mqtt_server.enable = enable;
        cfg.mqtt_server.listen_port = 1883;
        cfg
    }

    #[test]
    fn ota_mqtt_disabled_when_ota_flag_off() {
        let cfg = cfg_with_mqtt_server(true);
        let env = ota_env(false, "192.168.1.1:1883");
        assert!(!should_ota_include_mqtt(&cfg, &env, "192.168.1.1"));
    }

    #[test]
    fn ota_mqtt_included_when_builtin_broker_on() {
        let cfg = cfg_with_mqtt_server(true);
        let env = ota_env(true, "192.168.1.1:1883");
        assert!(should_ota_include_mqtt(&cfg, &env, "192.168.1.1"));
    }

    #[test]
    fn ota_mqtt_skipped_when_builtin_off_and_endpoint_is_local_broker() {
        let cfg = cfg_with_mqtt_server(false);
        let env = ota_env(true, "192.168.1.1:1883");
        assert!(!should_ota_include_mqtt(&cfg, &env, "192.168.1.1"));
    }

    #[test]
    fn ota_mqtt_included_for_external_broker_when_builtin_off() {
        let cfg = cfg_with_mqtt_server(false);
        let env = ota_env(true, "mqtt.example.com:1883");
        assert!(should_ota_include_mqtt(&cfg, &env, "192.168.1.1"));
    }
}
