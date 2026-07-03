use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use reqwest::Client;
use serde_json::Value;
use xiaozhi_config::user::{ActivationPayload, UConfig};
use xiaozhi_config::AppConfig;
use xiaozhi_core::Result;

use crate::traits::{EventHandler, UserConfigProvider, WsDeviceEventNotifier};

pub struct ManagerConfigProvider {
    backend_url: String,
    auth_token: String,
    app_config: AppConfig,
    event_handlers: DashMap<String, Vec<EventHandler>>,
    ws_notifier: RwLock<Option<WsDeviceEventNotifier>>,
    client: Client,
}

fn should_bypass_proxy(base_url: &str) -> bool {
    let base = base_url.to_lowercase();
    [
        "localhost",
        "127.0.0.1",
        "192.168.",
        "10.",
        "172.16.",
        "172.17.",
        "172.18.",
        "172.19.",
        "172.2",
        "172.30.",
        "172.31.",
    ]
    .iter()
    .any(|host| base.contains(host))
}

pub fn build_http_client(base_url: &str) -> Client {
    let mut builder = Client::builder().connect_timeout(Duration::from_secs(10));
    if should_bypass_proxy(base_url) {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .unwrap_or_else(|_| Client::new())
}

impl ManagerConfigProvider {
    pub fn new(app_config: AppConfig) -> Self {
        let backend_url = app_config.manager.backend_url.clone();
        Self {
            auth_token: app_config.manager.auth_token.clone(),
            client: build_http_client(&backend_url),
            backend_url,
            app_config,
            event_handlers: DashMap::new(),
            ws_notifier: RwLock::new(None),
        }
    }
}

#[async_trait]
impl UserConfigProvider for ManagerConfigProvider {
    async fn is_device_activated(&self, device_id: &str, client_id: &str) -> Result<bool> {
        let url = format!(
            "{}/api/internal/device/activated?device_id={}&client_id={}",
            self.backend_url.trim_end_matches('/'),
            device_id,
            client_id
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("激活检查失败: {e}")))?;

        if !resp.status().is_success() {
            return Ok(!self.app_config.auth.enable);
        }

        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Ok(body["activated"].as_bool().unwrap_or(false))
    }

    async fn get_activation_info(
        &self,
        device_id: &str,
        client_id: &str,
    ) -> Result<(String, String, String, i32)> {
        let url = format!(
            "{}/api/internal/device/activation?device_id={}&client_id={}",
            self.backend_url.trim_end_matches('/'),
            device_id,
            client_id
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("获取激活信息失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(xiaozhi_core::Error::Http(format!(
                "获取激活信息失败: HTTP {status} {text}"
            )));
        }

        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let code = body["code"].as_str().unwrap_or("").to_string();
        if code.is_empty() || code == "000000" {
            return Err(xiaozhi_core::Error::Http(
                "管理后台未返回有效激活码".to_string(),
            ));
        }
        Ok((
            code,
            body["message"].as_str().unwrap_or("").to_string(),
            body["challenge"].as_str().unwrap_or("").to_string(),
            body["expires_in"].as_i64().unwrap_or(300) as i32,
        ))
    }

    async fn verify_challenge(
        &self,
        device_id: &str,
        client_id: &str,
        payload: ActivationPayload,
    ) -> Result<bool> {
        let url = format!(
            "{}/api/internal/device/activate",
            self.backend_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(&serde_json::json!({
                "device_id": device_id,
                "client_id": client_id,
                "payload": payload,
            }))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("激活验证失败: {e}")))?;

        Ok(resp.status().is_success())
    }

    async fn get_user_config(&self, device_id: &str) -> Result<UConfig> {
        let url = format!(
            "{}/api/internal/configs/{}",
            self.backend_url.trim_end_matches('/'),
            device_id
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let config: UConfig = r.json().await.unwrap_or_default();
                if config.llm.config.is_empty() && config.asr.config.is_empty() {
                    tracing::warn!(
                        device_id = %device_id,
                        "从 manager 拉取到的设备配置为空，对话可能失败"
                    );
                }
                Ok(config)
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                let message = format!("拉取设备配置失败: HTTP {status} {body}");
                if self.app_config.manager.fallback_to_local_config {
                    tracing::warn!(
                        device_id = %device_id,
                        status = %status,
                        "拉取设备配置失败，回退到 config.yaml 默认配置"
                    );
                    Ok(UConfig::from_app_config(&self.app_config))
                } else {
                    Err(xiaozhi_core::Error::Http(message))
                }
            }
            Err(e) => {
                if self.app_config.manager.fallback_to_local_config {
                    tracing::warn!(
                        device_id = %device_id,
                        "连接 manager 拉取配置失败: {e}，回退到 config.yaml 默认配置"
                    );
                    Ok(UConfig::from_app_config(&self.app_config))
                } else {
                    Err(xiaozhi_core::Error::Http(format!(
                        "连接 manager 拉取配置失败: {e}"
                    )))
                }
            }
        }
    }

    async fn switch_device_role_by_name(&self, device_id: &str, role_name: &str) -> Result<String> {
        let url = format!(
            "{}/api/internal/device/{}/role",
            self.backend_url.trim_end_matches('/'),
            device_id
        );
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(&serde_json::json!({"role_name": role_name}))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("切换角色失败: {e}")))?;

        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        Ok(body["role_name"].as_str().unwrap_or(role_name).to_string())
    }

    async fn restore_device_default_role(&self, device_id: &str) -> Result<()> {
        let url = format!(
            "{}/api/internal/device/{}/role/default",
            self.backend_url.trim_end_matches('/'),
            device_id
        );
        let _ = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await;
        Ok(())
    }

    async fn get_system_config(&self) -> Result<String> {
        let url = format!(
            "{}/api/internal/system/configs",
            self.backend_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("获取系统配置失败: {e}")))?;

        Ok(resp.text().await.unwrap_or_default())
    }

    async fn touch_device_activity(&self, device_id: &str) -> Result<()> {
        let url = format!(
            "{}/api/internal/device/touch",
            self.backend_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(&serde_json::json!({ "device_id": device_id }))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("更新设备活跃时间失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::debug!(
                device_id = %device_id,
                "OTA 更新设备活跃时间未成功: HTTP {status} {body}"
            );
        }
        Ok(())
    }

    async fn report_device_presence(&self, device_id: &str, online: bool) -> Result<()> {
        let url = format!(
            "{}/api/internal/device/presence",
            self.backend_url.trim_end_matches('/')
        );
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .json(&serde_json::json!({
                "device_id": device_id,
                "online": online,
            }))
            .send()
            .await
            .map_err(|e| xiaozhi_core::Error::Http(format!("上报设备在线状态失败: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                device_id = %device_id,
                online = online,
                "上报设备在线状态被拒绝: HTTP {status} {body}"
            );
            return Err(xiaozhi_core::Error::Http(format!(
                "上报设备在线状态失败: HTTP {status} {body}"
            )));
        }
        Ok(())
    }

    fn notify_device_event(&self, event_type: &str, event_data: HashMap<String, Value>) {
        let guard = self.ws_notifier.read().unwrap_or_else(|e| e.into_inner());
        if let Some(notifier) = guard.as_ref() {
            notifier(event_type.to_string(), event_data);
        } else {
            tracing::debug!(
                event_type = %event_type,
                ?event_data,
                "设备事件（未附着 WS notifier）"
            );
        }
    }

    fn register_message_event_handler(&self, event_type: &str, handler: EventHandler) {
        self.event_handlers
            .entry(event_type.to_string())
            .or_default()
            .push(handler);
    }

    fn attach_ws_notifier(&self, notifier: WsDeviceEventNotifier) {
        *self.ws_notifier.write().unwrap_or_else(|e| e.into_inner()) = Some(notifier);
    }

    fn invoke_message_handlers(&self, event_type: &str, event_data: &HashMap<String, Value>) {
        if let Some(handlers) = self.event_handlers.get(event_type) {
            for handler in handlers.iter() {
                handler(event_type, event_data);
            }
        }
    }
}

pub fn create_provider(
    config: &AppConfig,
) -> Arc<dyn UserConfigProvider> {
    match config.config_provider.r#type.as_str() {
        "redis" => Arc::new(RedisConfigProvider::new(config.clone())),
        _ => Arc::new(ManagerConfigProvider::new(config.clone())),
    }
}

use crate::redis::RedisConfigProvider;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use xiaozhi_config::AppConfig;

    use crate::traits::events;

    #[test]
    fn invoke_message_handlers_runs_registered_callbacks() {
        let provider = ManagerConfigProvider::new(AppConfig::default());
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_cb = hits.clone();
        provider.register_message_event_handler(
            events::HANDLE_MESSAGE_INJECT,
            Arc::new(move |event_type, data| {
                assert_eq!(event_type, events::HANDLE_MESSAGE_INJECT);
                assert_eq!(data.get("device_id").and_then(|v| v.as_str()), Some("dev-1"));
                hits_cb.fetch_add(1, Ordering::SeqCst);
            }),
        );

        let mut data = HashMap::new();
        data.insert("device_id".to_string(), Value::String("dev-1".into()));
        provider.invoke_message_handlers(events::HANDLE_MESSAGE_INJECT, &data);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn notify_device_event_calls_ws_notifier() {
        let provider = ManagerConfigProvider::new(AppConfig::default());
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_cb = hits.clone();
        provider.attach_ws_notifier(Arc::new(move |event_type, data| {
            assert_eq!(event_type, events::DEVICE_ONLINE);
            assert_eq!(data.get("device_id").and_then(|v| v.as_str()), Some("dev-2"));
            hits_cb.fetch_add(1, Ordering::SeqCst);
        }));

        let mut data = HashMap::new();
        data.insert("device_id".to_string(), Value::String("dev-2".into()));
        provider.notify_device_event(events::DEVICE_ONLINE, data);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
