use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tracing::warn;
use xiaozhi_chat::{ChatManagerRegistry, SharedResourcePools};
use xiaozhi_config::AppConfig;
use xiaozhi_config_provider::{create_provider, events, WsDeviceEventNotifier};

use xiaozhi_history::HistoryClient;

use xiaozhi_hooks::HookManager;

use xiaozhi_mcp::{local_tools, GlobalMcpHub, McpManager};

use xiaozhi_openclaw::OpenClawManager;

use xiaozhi_rag::KnowledgeClient;



use crate::bridge::BridgeDispatcher;

use crate::manager_client::ManagerWsClient;

use crate::mqtt_runtime::{MqttRuntime, MqttServiceDeps};

use crate::shared_config::{self, SharedAppConfig};

use crate::websocket::WebSocketServer;



pub struct App {

    shared_config: SharedAppConfig,

    ws_server: WebSocketServer,

    chat_registry: Arc<ChatManagerRegistry>,

    config_provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,

    mcp_manager: Arc<McpManager>,

    openclaw: Arc<OpenClawManager>,

    history: Arc<HistoryClient>,

    _hook_manager: HookManager,

    bridge: Arc<BridgeDispatcher>,

    resource_pools: Arc<SharedResourcePools>,

    manager_client: Arc<ManagerWsClient>,

    mqtt_runtime: Arc<MqttRuntime>,

}



impl App {

    pub async fn new(config: AppConfig) -> anyhow::Result<Self> {

        let shared_config = shared_config::new_shared(config.clone());

        let config_provider = create_provider(&config);

        shared_config::load_from_manager(&shared_config, config_provider.as_ref()).await;
        shared_config::spawn_periodic_config_refresh(
            shared_config.clone(),
            config_provider.clone(),
        );

        let runtime_config = shared_config.read().await.clone();

        let history = Arc::new(HistoryClient::new(

            runtime_config.manager.backend_url.clone(),

            runtime_config.manager.auth_token.clone(),

            true,

        ));

        let openclaw = Arc::new(OpenClawManager::new());

        let global_mcp = GlobalMcpHub::start(&runtime_config.mcp.global);

        let mcp_manager = Arc::new(McpManager::new(

            runtime_config.mcp.global.servers.clone(),

            Some(global_mcp),

        ));

        local_tools::register_default_tools(&mcp_manager);
        mcp_manager.reload_local_mcp(&runtime_config.local_mcp);



        let chat_registry = Arc::new(ChatManagerRegistry::new());
        {
            let openclaw_handler = openclaw.clone();
            let chat_registry_handler = chat_registry.clone();
            openclaw_handler.set_response_handler(Arc::new(move |event| {
                let registry = chat_registry_handler.clone();
                tokio::spawn(async move {
                    if let Some(mgr) = registry.get(&event.device_id) {
                        if let Err(e) = mgr.inject_openclaw_response(event).await {
                            tracing::warn!(
                                device_id = %mgr.device_id(),
                                "OpenClaw 实时消息注入失败: {e}"
                            );
                        }
                    }
                });
            }));
        }

        let hook_manager = HookManager::new(&config.chat_hooks);



        let resource_pools = SharedResourcePools::new(config.resource_pools.clone());

        resource_pools.register_stats_collectors();



        let knowledge_client = Arc::new(KnowledgeClient::new(

            runtime_config.manager.backend_url.clone(),

            runtime_config.manager.auth_token.clone(),

        ));



        let mqtt_runtime = Arc::new(MqttRuntime::new());



        let bridge = Arc::new(BridgeDispatcher {

            config: shared_config.clone(),

            mcp_manager: mcp_manager.clone(),

            openclaw: openclaw.clone(),

            chat_registry: chat_registry.clone(),

            config_provider: config_provider.clone(),

            history: history.clone(),

            resource_pools: resource_pools.clone(),

            mqtt_runtime: mqtt_runtime.clone(),

        });



        let manager_client = ManagerWsClient::new(&config, bridge.clone());

        let ws_notifier: WsDeviceEventNotifier = {
            let mc = manager_client.clone();
            Arc::new(move |event_type, event_data| {
                let mc = mc.clone();
                let body = Value::Object(event_data.into_iter().collect());
                tokio::spawn(async move {
                    match mc
                        .send_request("POST", &event_type, body, Duration::from_secs(5))
                        .await
                    {
                        Ok(resp) if resp.status >= 400 => {
                            let msg = if resp.error.is_empty() {
                                format!("HTTP {}", resp.status)
                            } else {
                                resp.error
                            };
                            warn!(
                                event_type = %event_type,
                                "WS 设备事件被拒绝: {msg}"
                            );
                        }
                        Err(e) => {
                            warn!(
                                event_type = %event_type,
                                "WS 设备事件发送失败: {e}"
                            );
                        }
                        _ => {}
                    }
                });
            })
        };
        config_provider.attach_ws_notifier(ws_notifier);
        config_provider.register_message_event_handler(
            events::HANDLE_MESSAGE_INJECT,
            Arc::new(|event_type, data| {
                tracing::debug!(event = %event_type, ?data, "设备消息注入事件");
            }),
        );



        let ws_server = WebSocketServer::new(

            shared_config.clone(),

            chat_registry.clone(),

            config_provider.clone(),

            history.clone(),

            openclaw.clone(),

            mcp_manager.clone(),

            knowledge_client.clone(),

            resource_pools.clone(),

        );



        Ok(Self {

            shared_config,

            ws_server,

            chat_registry,

            config_provider,

            mcp_manager,

            openclaw,

            history,

            _hook_manager: hook_manager,

            bridge,

            resource_pools,

            manager_client,

            mqtt_runtime,

        })

    }



    pub async fn run(self) -> anyhow::Result<()> {

        let runtime_cfg = self.shared_config.read().await.clone();

        // 先启动 MQTT+UDP（使用 App::new 已从 DB 合并的配置），再连 Manager WS，
        // 避免 WS system_config 推送与 start_all 并发导致 MQTT 订阅被热重载打断。
        self.mqtt_runtime
            .start_all(MqttServiceDeps {
                config: runtime_cfg,
                chat_registry: self.chat_registry.clone(),
                config_provider: self.config_provider.clone(),
                history: self.history.clone(),
                openclaw: self.openclaw.clone(),
                mcp_manager: self.mcp_manager.clone(),
                resource_pools: self.resource_pools.clone(),
            })
            .await;

        let manager_client = self.manager_client.clone();
        tokio::spawn(async move {
            manager_client.run_forever().await;
        });



        xiaozhi_pool::register_collector(Arc::new(OnlineDevicesCollector {

            registry: self.chat_registry.clone(),

        }));



        let history = self.history.clone();

        xiaozhi_pool::StatsReporter::start_pool_reporter(5, move |payload| {

            let history = history.clone();

            async move {

                if let Err(e) = history.report_pool_stats(&payload).await {

                    tracing::warn!("资源池统计上报失败: {e}");

                }

            }

        });



        self.ws_server.start().await

    }

}



struct OnlineDevicesCollector {

    registry: Arc<ChatManagerRegistry>,

}



impl xiaozhi_pool::PoolStatsCollector for OnlineDevicesCollector {

    fn collect(&self) -> std::collections::HashMap<String, serde_json::Value> {

        let count = self.registry.online_count();

        let mut map = std::collections::HashMap::new();

        map.insert(

            "chat_sessions".to_string(),

            serde_json::json!({

                "total_resources": count,

                "available_resources": 0,

                "in_use_resources": count,

                "max_size": count,

                "min_size": 0,

                "max_idle": 0,

                "is_closed": false,

            }),

        );

        map

    }

}


