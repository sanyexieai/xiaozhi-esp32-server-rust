use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use xiaozhi_config::{apply_system_config_bundle, AppConfig};

pub type SharedAppConfig = Arc<RwLock<AppConfig>>;

pub fn new_shared(config: AppConfig) -> SharedAppConfig {
    Arc::new(RwLock::new(config))
}

pub async fn apply_system_config(shared: &SharedAppConfig, data: &Value) {
    let mut cfg = shared.write().await;
    match apply_system_config_bundle(&mut cfg, data) {
        Ok(()) => tracing::info!("已应用管理后台下发的系统配置"),
        Err(e) => tracing::warn!("应用系统配置失败: {e:#}"),
    }
}

pub async fn load_from_manager(
    shared: &SharedAppConfig,
    provider: &dyn xiaozhi_config_provider::UserConfigProvider,
) {
    match provider.get_system_config().await {
        Ok(text) if !text.trim().is_empty() => {
            match serde_json::from_str::<Value>(&text) {
                Ok(data) => {
                    let had_mcp = data.get("mcp").is_some();
                    apply_system_config(shared, &data).await;
                    if !had_mcp {
                        let mut cfg = shared.write().await;
                        cfg.mcp.global.enabled = false;
                        cfg.mcp.global.servers.clear();
                        tracing::info!(
                            "管理后台未配置 MCP，已禁用全局 MCP（不使用 config.yaml 默认 endpoint）"
                        );
                    } else {
                        let cfg = shared.read().await;
                        tracing::info!(
                            "全局 MCP: enabled={}, servers={}",
                            cfg.mcp.global.enabled,
                            cfg.mcp.global.servers.len()
                        );
                    }
                }
                Err(e) => tracing::warn!("解析系统配置 JSON 失败: {e}"),
            }
        }
        Ok(_) => {
            let mut cfg = shared.write().await;
            cfg.mcp.global.enabled = false;
            cfg.mcp.global.servers.clear();
            tracing::info!("管理后台系统配置为空，已禁用全局 MCP");
        }
        Err(e) => {
            tracing::warn!("拉取系统配置失败: {e}");
            let mut cfg = shared.write().await;
            cfg.mcp.global.enabled = false;
            cfg.mcp.global.servers.clear();
        }
    }
}

/// 对齐 Go `config_provider.enable_periodic_update`：周期拉取系统配置
pub fn spawn_periodic_config_refresh(
    shared: SharedAppConfig,
    provider: Arc<dyn xiaozhi_config_provider::UserConfigProvider>,
) {
    tokio::spawn(async move {
        loop {
            let (enabled, secs) = {
                let cfg = shared.read().await;
                (
                    cfg.config_provider.enable_periodic_update,
                    parse_update_interval_secs(&cfg.config_provider.update_interval),
                )
            };
            if !enabled {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            load_from_manager(&shared, provider.as_ref()).await;
        }
    });
}

fn parse_update_interval_secs(raw: &str) -> u64 {
    let s = raw.trim();
    if s.is_empty() {
        return 300;
    }
    if let Some(num) = s.strip_suffix('m') {
        return num.trim().parse::<u64>().unwrap_or(5).saturating_mul(60);
    }
    if let Some(num) = s.strip_suffix('s') {
        return num.trim().parse().unwrap_or(300);
    }
    s.parse().unwrap_or(300)
}
