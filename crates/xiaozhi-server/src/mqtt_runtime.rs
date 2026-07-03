//! MQTT Broker 与 MQTT+UDP 服务生命周期（对齐 Go 热重载粒度）

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;

use xiaozhi_chat::{ChatManagerRegistry, SharedResourcePools};
use xiaozhi_config::{AppConfig, MqttClientConfig, MqttServerConfig, UdpConfig};
use xiaozhi_config_provider::UserConfigProvider;
use xiaozhi_history::HistoryClient;
use xiaozhi_mcp::McpManager;
use xiaozhi_mqtt_server::MqttBroker;
use xiaozhi_openclaw::OpenClawManager;
use xiaozhi_rag::KnowledgeClient;

use crate::device_handler::DeviceRuntime;
use crate::mqtt_service::{MqttDeviceGateway, MqttUdpService};

pub struct MqttServiceDeps {
    pub config: AppConfig,
    pub chat_registry: Arc<ChatManagerRegistry>,
    pub config_provider: Arc<dyn UserConfigProvider>,
    pub history: Arc<HistoryClient>,
    pub openclaw: Arc<OpenClawManager>,
    pub mcp_manager: Arc<McpManager>,
    pub resource_pools: Arc<SharedResourcePools>,
}

pub struct MqttRuntime {
    apply_lock: AsyncMutex<()>,
    broker: Mutex<Option<Arc<MqttBroker>>>,
    broker_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    service_shutdown: Mutex<Arc<AtomicBool>>,
    service_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    last_mqtt_server: Mutex<Option<MqttServerConfig>>,
    last_mqtt: Mutex<Option<MqttClientConfig>>,
    last_udp: Mutex<Option<UdpConfig>>,
    gateway: Arc<MqttDeviceGateway>,
}

impl MqttRuntime {
    pub fn new() -> Self {
        Self {
            apply_lock: AsyncMutex::new(()),
            broker: Mutex::new(None),
            broker_task: Mutex::new(None),
            service_shutdown: Mutex::new(Arc::new(AtomicBool::new(false))),
            service_task: Mutex::new(None),
            last_mqtt_server: Mutex::new(None),
            last_mqtt: Mutex::new(None),
            last_udp: Mutex::new(None),
            gateway: Arc::new(MqttDeviceGateway::new()),
        }
    }

    pub fn device_gateway(&self) -> Arc<MqttDeviceGateway> {
        self.gateway.clone()
    }

    pub async fn prepare_hardware_wake(&self, device_id: &str) -> Result<(), String> {
        self.gateway.prepare_hardware_wake(device_id).await
    }

    pub async fn start_all(&self, deps: MqttServiceDeps) {
        // 与配置热更新共用 diff 逻辑，避免 WS 推送已启动 MQTT 后再次强制 reload 打断订阅
        self.apply_from_config_change(deps).await;
    }

    pub async fn apply_from_config_change(&self, deps: MqttServiceDeps) {
        let reload_broker = self
            .last_mqtt_server
            .lock()
            .unwrap()
            .as_ref()
            .map(|old| old != &deps.config.mqtt_server)
            .unwrap_or(true);
        let reload_mqtt = self
            .last_mqtt
            .lock()
            .unwrap()
            .as_ref()
            .map(|old| old != &effective_mqtt_client_config(&deps.config))
            .unwrap_or(true);
        let reload_udp = self
            .last_udp
            .lock()
            .unwrap()
            .as_ref()
            .map(|old| udp_service_changed(old, &deps.config.udp))
            .unwrap_or(true);

        self.apply(deps, reload_broker, reload_mqtt || reload_udp, reload_udp)
            .await;
    }

    async fn stop_broker(&self) {
        let task = self.broker_task.lock().unwrap().take();
        let broker = self.broker.lock().unwrap().take();
        if let Some(broker) = broker {
            broker.shutdown();
        }
        if let Some(task) = task {
            task.abort();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    async fn stop_service(&self) {
        let shutdown = self.service_shutdown.lock().unwrap().clone();
        shutdown.store(true, Ordering::Relaxed);
        let task = self.service_task.lock().unwrap().take();
        if let Some(task) = task {
            match tokio::time::timeout(Duration::from_secs(5), task).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!("MQTT+UDP 服务任务异常退出: {e}"),
                Err(_) => tracing::warn!("MQTT+UDP 服务停止超时，UDP 端口可能尚未释放"),
            }
        }
    }

    async fn apply(
        &self,
        deps: MqttServiceDeps,
        reload_broker: bool,
        reload_service: bool,
        _reload_udp_only: bool,
    ) {
        let _guard = self.apply_lock.lock().await;
        let cfg = deps.config.clone();

        if reload_broker {
            self.stop_broker().await;
            if cfg.mqtt_server.enable {
                let broker = Arc::new(MqttBroker::new(cfg.mqtt_server.clone()));
                match broker.clone().start().await {
                    Ok(task) => {
                        self.broker.lock().unwrap().replace(broker);
                        self.broker_task.lock().unwrap().replace(task);
                    }
                    Err(e) => tracing::error!("MQTT Broker 启动失败: {e}"),
                }
            } else {
                tracing::info!("MQTT Broker 未启用");
            }
            *self.last_mqtt_server.lock().unwrap() = Some(cfg.mqtt_server.clone());
        }

        if reload_service {
            self.stop_service().await;
            // 热重载后等待 UDP 端口释放（Windows 上 abort 后可能短暂占用）
            tokio::time::sleep(Duration::from_millis(600)).await;
            let new_shutdown = Arc::new(AtomicBool::new(false));
            *self.service_shutdown.lock().unwrap() = new_shutdown.clone();

            let mqtt_cfg = effective_mqtt_client_config(&cfg);
            if mqtt_cfg.enable {
                let runtime = DeviceRuntime {
                    config: cfg.clone(),
                    chat_registry: deps.chat_registry,
                    config_provider: deps.config_provider,
                    history: deps.history,
                    openclaw: deps.openclaw,
                    mcp_manager: deps.mcp_manager,
                    knowledge_client: Arc::new(KnowledgeClient::new(
                        cfg.manager.backend_url.clone(),
                        cfg.manager.auth_token.clone(),
                    )),
                    resource_pools: deps.resource_pools,
                };
                let mqtt_cfg_spawn = mqtt_cfg.clone();
                let udp_cfg = cfg.udp.clone();
                let shutdown_for_task = new_shutdown.clone();
                let gateway = self.gateway.clone();
                let handle = tokio::spawn(async move {
                    const MAX_START_ATTEMPTS: u32 = 3;
                    for attempt in 1..=MAX_START_ATTEMPTS {
                        if shutdown_for_task.load(Ordering::Relaxed) {
                            return;
                        }
                        let service =
                            MqttUdpService::new(runtime.clone(), mqtt_cfg_spawn.clone(), udp_cfg.clone());
                        match service
                            .start(shutdown_for_task.clone(), gateway.clone())
                            .await
                        {
                            Ok(()) => return,
                            Err(e) => {
                                tracing::error!(
                                    attempt,
                                    max = MAX_START_ATTEMPTS,
                                    "MQTT+UDP 服务启动失败: {e:#}"
                                );
                                if attempt < MAX_START_ATTEMPTS {
                                    tokio::time::sleep(Duration::from_millis(800 * attempt as u64))
                                        .await;
                                }
                            }
                        }
                    }
                });
                self.service_task.lock().unwrap().replace(handle);
            } else {
                tracing::info!("MQTT 客户端服务未启用");
            }
            *self.last_mqtt.lock().unwrap() = Some(mqtt_cfg);
            *self.last_udp.lock().unwrap() = Some(cfg.udp.clone());
        }
    }
}

fn udp_service_changed(old: &UdpConfig, new: &UdpConfig) -> bool {
    old.listen_host != new.listen_host
        || old.listen_port != new.listen_port
        || old.external_host != new.external_host
        || old.external_port != new.external_port
}

/// 内置 Broker 模式下必须启动 MQTT 客户端，否则设备连上 broker 也无法对话。
fn effective_mqtt_client_config(cfg: &AppConfig) -> MqttClientConfig {
    let mut mqtt = cfg.mqtt.clone();
    if !cfg.mqtt_server.enable {
        return mqtt;
    }
    if !mqtt.enable {
        tracing::info!(
            port = cfg.mqtt_server.listen_port,
            "内置 MQTT Broker 已启用，自动开启 MQTT 客户端服务"
        );
    }
    mqtt.enable = true;
    mqtt.broker = "127.0.0.1".to_string();
    mqtt.port = cfg.mqtt_server.listen_port;
    if mqtt.username.trim().is_empty() {
        mqtt.username = cfg.mqtt_server.username.clone();
    }
    if mqtt.password.trim().is_empty() {
        mqtt.password = cfg.mqtt_server.password.clone();
    }
    mqtt
}
