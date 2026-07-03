use std::collections::HashMap;

use async_trait::async_trait;
use redis::AsyncCommands;
use serde_json::Value;
use xiaozhi_config::provider_resolve::{
    merge_redis_provider_config, provider_config_to_redis_json, resolve_from_app,
};
use xiaozhi_config::user::{ActivationPayload, UConfig};
use xiaozhi_config::AppConfig;
use xiaozhi_core::Result;

use crate::manager::ManagerConfigProvider;
use crate::traits::{EventHandler, UserConfigProvider, WsDeviceEventNotifier};

pub struct RedisConfigProvider {
    inner: ManagerConfigProvider,
    app_config: AppConfig,
    redis_url: String,
    key_prefix: String,
}

impl RedisConfigProvider {
    pub fn new(config: AppConfig) -> Self {
        let redis_url = format!(
            "redis://:{}@{}:{}/{}",
            config.redis.password,
            config.redis.host,
            config.redis.port,
            config.redis.db
        );
        Self {
            inner: ManagerConfigProvider::new(config.clone()),
            app_config: config.clone(),
            redis_url,
            key_prefix: config.redis.key_prefix.clone(),
        }
    }

    fn user_config_key(&self, device_id: &str) -> String {
        format!("{}:userconfig:{}", self.key_prefix, device_id)
    }

    fn system_prompt_key(&self, device_id: &str) -> String {
        format!("{}:llm:system:{}", self.key_prefix, device_id)
    }

    async fn redis_conn(
        &self,
    ) -> Result<redis::aio::MultiplexedConnection> {
        let client = redis::Client::open(self.redis_url.as_str())
            .map_err(|e| xiaozhi_core::Error::Other(format!("Redis 连接失败: {e}")))?;
        client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| xiaozhi_core::Error::Other(format!("Redis 连接失败: {e}")))
    }

    async fn read_system_prompt(&self, device_id: &str) -> Result<String> {
        let mut conn = self.redis_conn().await?;
        let key = self.system_prompt_key(device_id);
        let prompt: Option<String> = conn.get(&key).await.ok().flatten();
        Ok(prompt.unwrap_or_else(|| self.app_config.system_prompt.clone()))
    }

    async fn load_from_redis(&self, device_id: &str) -> Result<Option<UConfig>> {
        let mut conn = self.redis_conn().await?;
        let key = self.user_config_key(device_id);
        let hash: HashMap<String, String> = conn.hgetall(&key).await.unwrap_or_default();
        if hash.is_empty() {
            return Ok(None);
        }

        let app = &self.app_config;
        let mut uconfig = UConfig {
            system_prompt: self.read_system_prompt(device_id).await?,
            memory_mode: "short".into(),
            speaker_chat_mode: "off".into(),
            vad: resolve_from_app(app, "vad"),
            ..Default::default()
        };

        for (kind, assign) in [
            ("llm", &mut uconfig.llm),
            ("asr", &mut uconfig.asr),
            ("tts", &mut uconfig.tts),
            ("memory", &mut uconfig.memory),
        ] {
            if let Some(raw) = hash.get(kind).filter(|s| !s.is_empty()) {
                if let Ok(redis_cfg) = serde_json::from_str::<HashMap<String, Value>>(raw) {
                    *assign = merge_redis_provider_config(app, kind, &redis_cfg);
                }
            } else {
                *assign = merge_redis_provider_config(app, kind, &HashMap::new());
            }
        }

        Ok(Some(uconfig))
    }

    async fn write_cache(&self, device_id: &str, config: &UConfig) -> Result<()> {
        let mut conn = self.redis_conn().await?;
        let key = self.user_config_key(device_id);

        for (field, provider) in [
            ("llm", &config.llm),
            ("asr", &config.asr),
            ("tts", &config.tts),
            ("memory", &config.memory),
        ] {
            let json = serde_json::to_string(&provider_config_to_redis_json(provider))
                .map_err(|e| xiaozhi_core::Error::Other(format!("Redis 序列化失败: {e}")))?;
            let _: () = conn
                .hset(&key, field, json)
                .await
                .map_err(|e| xiaozhi_core::Error::Other(format!("Redis 写入失败: {e}")))?;
        }

        if !config.system_prompt.is_empty() {
            let sp_key = self.system_prompt_key(device_id);
            let _: () = conn
                .set(&sp_key, &config.system_prompt)
                .await
                .map_err(|e| xiaozhi_core::Error::Other(format!("Redis 写入 system_prompt 失败: {e}")))?;
        }
        Ok(())
    }
}

#[async_trait]
impl UserConfigProvider for RedisConfigProvider {
    async fn is_device_activated(&self, device_id: &str, client_id: &str) -> Result<bool> {
        self.inner
            .is_device_activated(device_id, client_id)
            .await
    }

    async fn get_activation_info(
        &self,
        device_id: &str,
        client_id: &str,
    ) -> Result<(String, String, String, i32)> {
        self.inner.get_activation_info(device_id, client_id).await
    }

    async fn verify_challenge(
        &self,
        device_id: &str,
        client_id: &str,
        payload: ActivationPayload,
    ) -> Result<bool> {
        self.inner
            .verify_challenge(device_id, client_id, payload)
            .await
    }

    async fn get_user_config(&self, device_id: &str) -> Result<UConfig> {
        if let Some(config) = self.load_from_redis(device_id).await? {
            return Ok(config);
        }

        let config = self.inner.get_user_config(device_id).await?;
        if let Err(e) = self.write_cache(device_id, &config).await {
            tracing::warn!(
                device_id = %device_id,
                "Manager 配置写回 Redis 失败: {e}"
            );
        }
        Ok(config)
    }

    async fn switch_device_role_by_name(&self, device_id: &str, role_name: &str) -> Result<String> {
        self.invalidate_user_config_cache(device_id).await?;
        self.inner
            .switch_device_role_by_name(device_id, role_name)
            .await
    }

    async fn restore_device_default_role(&self, device_id: &str) -> Result<()> {
        self.invalidate_user_config_cache(device_id).await?;
        self.inner.restore_device_default_role(device_id).await
    }

    async fn get_system_config(&self) -> Result<String> {
        self.inner.get_system_config().await
    }

    async fn touch_device_activity(&self, device_id: &str) -> Result<()> {
        self.inner.touch_device_activity(device_id).await
    }

    async fn report_device_presence(&self, device_id: &str, online: bool) -> Result<()> {
        self.inner.report_device_presence(device_id, online).await
    }

    fn notify_device_event(&self, event_type: &str, event_data: HashMap<String, Value>) {
        self.inner.notify_device_event(event_type, event_data);
    }

    fn register_message_event_handler(&self, event_type: &str, handler: EventHandler) {
        self.inner
            .register_message_event_handler(event_type, handler);
    }

    fn attach_ws_notifier(&self, notifier: WsDeviceEventNotifier) {
        self.inner.attach_ws_notifier(notifier);
    }

    fn invoke_message_handlers(&self, event_type: &str, event_data: &HashMap<String, Value>) {
        self.inner.invoke_message_handlers(event_type, event_data);
    }

    async fn invalidate_user_config_cache(&self, device_id: &str) -> Result<()> {
        let mut conn = self.redis_conn().await?;
        let key = self.user_config_key(device_id);
        let sp_key = self.system_prompt_key(device_id);
        let _: () = conn
            .del(&[key.as_str(), sp_key.as_str()])
            .await
            .map_err(|e| xiaozhi_core::Error::Other(format!("Redis 失效缓存失败: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaozhi_config::loader::load_config;

    #[test]
    fn user_config_key_matches_go() {
        let mut config = AppConfig::default();
        config.redis.key_prefix = "xiaozhi".into();
        let provider = RedisConfigProvider::new(config);
        assert_eq!(provider.user_config_key("dev-1"), "xiaozhi:userconfig:dev-1");
        assert_eq!(
            provider.system_prompt_key("dev-1"),
            "xiaozhi:llm:system:dev-1"
        );
    }

    #[test]
    fn merge_redis_llm_provider_override() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = format!("{manifest}/../../config/config.yaml");
        let app = load_config(&path).expect("load config");
        let mut redis_cfg = HashMap::new();
        redis_cfg.insert("provider".into(), Value::String("deepseek".into()));
        let merged = merge_redis_provider_config(&app, "llm", &redis_cfg);
        assert_eq!(merged.provider, "deepseek");
        assert!(
            merged
                .config
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .starts_with("sk-"),
            "应合并 config.yaml 中 deepseek 块的 api_key"
        );
    }
}
