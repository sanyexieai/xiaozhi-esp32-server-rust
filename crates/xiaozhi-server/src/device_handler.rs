//! 设备会话共享逻辑（WebSocket / MQTT+UDP）

use std::sync::Arc;

use xiaozhi_chat::{has_mcp_feature, ChatManager, ChatManagerRegistry, SharedResourcePools};
use xiaozhi_config::AppConfig;
use xiaozhi_history::HistoryClient;
use xiaozhi_mcp::McpManager;
use xiaozhi_openclaw::OpenClawManager;
use xiaozhi_protocol::messages::{ClientMessage, ServerMessage, UdpConfig};
use xiaozhi_rag::KnowledgeClient;
use xiaozhi_transport::udp::UdpCrypto;

#[derive(Clone)]
pub struct DeviceRuntime {
    pub config: AppConfig,
    pub chat_registry: Arc<ChatManagerRegistry>,
    pub config_provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,
    pub history: Arc<HistoryClient>,
    pub openclaw: Arc<OpenClawManager>,
    pub mcp_manager: Arc<McpManager>,
    pub knowledge_client: Arc<KnowledgeClient>,
    pub resource_pools: Arc<SharedResourcePools>,
}

pub struct UdpHelloInfo {
    pub server_host: String,
    pub server_port: u16,
    pub key: [u8; 16],
    pub nonce: [u8; 16],
}

pub struct UdpSession {
    pub device_id: String,
    pub crypto: UdpCrypto,
    pub conn_id: u32,
    pub key: [u8; 16],
    pub nonce: [u8; 16],
}

impl DeviceRuntime {
    pub async fn ensure_chat_manager(&self, device_id: &str) -> Result<Arc<ChatManager>, String> {
        if let Some(mgr) = self.chat_registry.get(device_id) {
            return Ok(mgr);
        }
        let mgr = ChatManager::new(
            device_id.to_string(),
            self.config.clone(),
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
        mgr.spawn_replay_openclaw_offline_messages();
        Ok(mgr)
    }

    pub fn new_udp_session(device_id: &str) -> UdpSession {
        let (key, nonce, conn_id) = UdpCrypto::generate_session_keys();
        UdpSession {
            device_id: device_id.to_string(),
            crypto: UdpCrypto::new(key, nonce),
            conn_id,
            key,
            nonce,
        }
    }
}

pub async fn process_client_message(
    chat_mgr: &Arc<ChatManager>,
    msg: ClientMessage,
    udp_hello: Option<&UdpHelloInfo>,
) -> Vec<ServerMessage> {
    let mut responses = Vec::new();

    if msg.msg_type.as_str() != xiaozhi_core::message::GOODBYE {
        chat_mgr.on_device_activity().await;
    }

    match msg.msg_type.as_str() {
        xiaozhi_core::message::HELLO => {
            tracing::info!(device_id = %chat_mgr.device_id(), "处理设备 hello");
            let is_duplicate_mqtt = chat_mgr.is_hello_inited()
                && !chat_mgr.requires_fresh_hello()
                && udp_hello.is_some();
            if udp_hello.is_some() {
                chat_mgr.set_mqtt_transport(true);
            } else if !chat_mgr.has_hardware_endpoint() {
                // 纯 Web 连接可标记 websocket；硬件仍在线时保留 MQTT 播报链路
                chat_mgr.set_mqtt_transport(false);
            }

            if chat_mgr.is_hello_inited() {
                if let Err(e) = chat_mgr.refresh_device_config_on_hello().await {
                    tracing::warn!(
                        device_id = %chat_mgr.device_id(),
                        "duplicate hello 刷新配置失败，降级继续: {e:#}"
                    );
                }
            }
            chat_mgr.reset_openclaw_mode_on_hello();

            if is_duplicate_mqtt {
                chat_mgr
                    .mark_mqtt_conversation_state_stale("duplicate_hello")
                    .await;
            }

            if let Some(v) = msg.version {
                chat_mgr.set_binary_protocol_version(v as u8);
            }
            let resume_session_id = msg
                .features
                .as_ref()
                .and_then(|f| f.get("resume_session_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let session_id = match resume_session_id.clone() {
                Some(id) => id,
                None => chat_mgr
                    .active_session_id()
                    .await
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            };
            let transport = if udp_hello.is_some() { "udp" } else { "websocket" };
            let audio_params = msg
                .audio_params
                .clone()
                .unwrap_or_else(xiaozhi_protocol::audio::AudioParams::default);
            let mut hello = ServerMessage::hello_with_transport(
                session_id.clone(),
                audio_params,
                transport,
            );
            if let Some(udp) = udp_hello {
                tracing::info!(
                    device_id = %chat_mgr.device_id(),
                    udp_server = %udp.server_host,
                    udp_port = udp.server_port,
                    "hello 下发 UDP 地址（来自管理后台系统配置）"
                );
                hello.udp = Some(UdpConfig {
                    server: udp.server_host.clone(),
                    port: udp.server_port,
                    key: hex::encode(udp.key),
                    nonce: hex::encode(udp.nonce),
                });
            }
            chat_mgr.prepare_session(session_id.clone()).await;

            // 对齐 Go `ensureSessionForHello`：先完成会话 bootstrap，再下发 hello 响应，
            // 避免设备收到 hello 后立即 listen/detect 时会话尚未就绪。
            let mgr = chat_mgr.clone();
            let features = msg.features.clone();
            let resume_for_restore = resume_session_id.clone();
            match mgr.clone().init_session(session_id.clone()).await {
                Ok(()) => {
                    if has_mcp_feature(&features) {
                        mgr.schedule_mcp_init(session_id.clone(), features);
                    }
                    mgr.spawn_replay_openclaw_offline_messages();
                    if let Some(resume_id) = resume_for_restore {
                        match mgr.restore_dialogue_from_history(&resume_id).await {
                            Ok(count) => tracing::info!(
                                device_id = %mgr.device_id(),
                                session_id = %resume_id,
                                restored = count,
                                "已恢复历史会话上下文"
                            ),
                            Err(e) => tracing::warn!(
                                device_id = %mgr.device_id(),
                                session_id = %resume_id,
                                "恢复历史会话失败: {e:#}"
                            ),
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        device_id = %mgr.device_id(),
                        "init_session 失败: {e:#}"
                    );
                    if chat_mgr.requires_fresh_hello() {
                        chat_mgr.mark_need_fresh_hello();
                    }
                }
            }

            responses.push(hello);
        }
        xiaozhi_core::message::MCP => {
            if let Some(payload) = msg.payload.as_ref() {
                chat_mgr.clone().handle_mcp_payload(payload).await;
            }
        }
        xiaozhi_core::message::LISTEN => {
            if let Err(e) = chat_mgr
                .handle_listen_message(msg.state.as_deref(), msg.mode.as_deref(), msg.text.as_deref())
                .await
            {
                tracing::error!(
                    device_id = %chat_mgr.device_id(),
                    "listen 消息处理失败: {e:#}"
                );
            }
        }
        xiaozhi_core::message::ABORT => {
            tracing::info!(
                device_id = %chat_mgr.device_id(),
                "收到设备 MQTT abort 信令"
            );
            chat_mgr
                .clone()
                .on_abort(xiaozhi_chat::detect::AbortOrigin::Device)
                .await;
        }
        xiaozhi_core::message::GOODBYE => {
            chat_mgr.clone().handle_device_goodbye().await;
        }
        xiaozhi_core::message::SPEAK_READY => {
            if let Err(e) = chat_mgr
                .clone()
                .handle_speak_ready_message(
                    msg.session_id.as_deref(),
                    msg.state.as_deref(),
                    msg.udp_config.as_ref(),
                )
                .await
            {
                tracing::warn!(
                    device_id = %chat_mgr.device_id(),
                    "speak_ready 处理失败: {e:#}"
                );
            }
        }
        xiaozhi_core::message::IOT => {
            responses.push(ServerMessage::iot_success());
        }
        _ => {}
    }

    let channel = if udp_hello.is_some() || chat_mgr.is_mqtt_transport() {
        "mqtt"
    } else {
        "ws"
    };
    for resp in &responses {
        chat_mgr.record_outbound_server(channel, resp).await;
    }

    responses
}
