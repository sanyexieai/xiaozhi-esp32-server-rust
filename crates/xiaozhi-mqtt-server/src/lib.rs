//! 内置 MQTT Broker（RMQTT + DeviceHook/AuthHook 对齐 Go）

mod device;
mod hooks;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rmqtt::context::ServerContext;
use rmqtt::net::Builder;
use rmqtt::server::MqttServer;
use xiaozhi_config::MqttServerConfig;
use xiaozhi_core::Result;

use hooks::{parse_listen_addr, register_hooks, HookState};

pub struct MqttBroker {
    config: MqttServerConfig,
    shutdown: Arc<AtomicBool>,
}

impl MqttBroker {
    pub fn new(config: MqttServerConfig) -> Self {
        Self {
            config,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub async fn start(self: Arc<Self>) -> Result<tokio::task::JoinHandle<()>> {
        let cfg = self.config.clone();
        let shutdown = self.shutdown.clone();

        let hook_state = Arc::new(HookState::new(cfg.clone()));
        let scx = ServerContext::new().busy_check_enable(false).build().await;
        register_hooks(&scx, hook_state).await;

        let tcp_addr = parse_listen_addr(&cfg.listen_host, cfg.listen_port);
        tracing::info!(
            "MQTT Broker (RMQTT) 监听: {}:{} (auth={}, device_hook=on)",
            cfg.listen_host,
            cfg.listen_port,
            cfg.enable_auth
        );

        let tcp_listener = Builder::new()
            .name("xiaozhi/tcp")
            .laddr(tcp_addr)
            .allow_anonymous(false)
            .bind()
            .map_err(|e| xiaozhi_core::Error::Mqtt(format!("MQTT TCP 绑定 {tcp_addr} 失败: {e}")))?
            .tcp()
            .map_err(|e| xiaozhi_core::Error::Mqtt(format!("MQTT TCP listener 创建失败: {e}")))?;

        let mut server_builder = MqttServer::new(scx).listener(tcp_listener);

        if cfg.tls.enable {
            let pem = cfg.tls.pem.trim();
            let key = cfg.tls.key.trim();
            if pem.is_empty() || key.is_empty() {
                return Err(xiaozhi_core::Error::Mqtt(
                    "MQTT TLS 已启用但未配置 pem/key".into(),
                ));
            }
            let tls_addr = parse_listen_addr("0.0.0.0", cfg.tls.port);
            tracing::info!("MQTT Broker TLS 监听: 0.0.0.0:{}", cfg.tls.port);
            let tls_listener = Builder::new()
                .name("xiaozhi/tls")
                .laddr(tls_addr)
                .allow_anonymous(false)
                .tls_cert(Some(pem))
                .tls_key(Some(key))
                .bind()
                .map_err(|e| {
                    xiaozhi_core::Error::Mqtt(format!("MQTT TLS 绑定 {tls_addr} 失败: {e}"))
                })?
                .tls()
                .map_err(|e| xiaozhi_core::Error::Mqtt(format!("MQTT TLS listener 创建失败: {e}")))?;
            server_builder = server_builder.listener_by_id(tls_listener, cfg.tls.port);
        }

        let server = server_builder.build();
        let handle = tokio::spawn(async move {
            tokio::select! {
                res = server.run() => {
                    if let Err(e) = res {
                        tracing::error!("MQTT Broker (RMQTT) 异常退出: {e}");
                    }
                }
                _ = wait_shutdown(shutdown) => {
                    tracing::info!("MQTT Broker (RMQTT) 已停止");
                }
            }
        });

        Ok(handle)
    }
}

async fn wait_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}
