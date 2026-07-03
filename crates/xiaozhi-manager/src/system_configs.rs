use serde_json::{json, Value};

use xiaozhi_config::{apply_system_config_bundle, AppConfig};

use crate::app::AppState;
use crate::db::{ConfigInput, ConfigRow, Database};
use crate::mcp_imported_merge::merge_mcp_with_enabled_imported_services;

const SYSTEM_CONFIG_TYPES: &[&str] = &[
    "mqtt",
    "mqtt_server",
    "udp",
    "ota",
    "auth",
    "chat",
    "vision_base",
    "voice_identify",
    "mcp",
    "local_mcp",
];

fn parse_config_json(row: &ConfigRow) -> Option<Value> {
    if row.json_data.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&row.json_data).ok()
}

/// 管理后台 MCP 页保存的是 `{ "mcp": { global, device? }, "local_mcp": ... }`，
/// 合并进 AppConfig 时需要展开为 `{ global, device }`。
fn normalize_system_config_block(kind: &str, parsed: Value) -> Value {
    match kind {
        "mcp" => parsed
            .get("mcp")
            .cloned()
            .unwrap_or(parsed),
        "local_mcp" => parsed
            .get("local_mcp")
            .cloned()
            .unwrap_or(parsed),
        _ => parsed,
    }
}

/// 从 DB 聚合系统级配置（与 Go `getSystemConfigsData` 一致，供 server 拉取 / WS 推送）。
pub fn build_system_configs_data(db: &Database) -> anyhow::Result<Value> {
    let mut bundle = serde_json::Map::new();
    for kind in SYSTEM_CONFIG_TYPES {
        let rows = db.list_configs(kind).unwrap_or_default();
        let selected = rows
            .iter()
            .find(|r| r.is_default)
            .or_else(|| rows.first());
        let Some(row) = selected else { continue };
        let Some(parsed) = parse_config_json(row) else { continue };
        if *kind == "mcp" {
            if let Some(local) = parsed.get("local_mcp") {
                bundle
                    .entry("local_mcp".to_string())
                    .or_insert_with(|| local.clone());
            }
        }
        let normalized_base = normalize_system_config_block(kind, parsed);
        let normalized = if *kind == "mcp" {
            match merge_mcp_with_enabled_imported_services(db, normalized_base.clone()) {
                Ok((merged, warnings)) => {
                    if !warnings.is_empty() {
                        tracing::warn!(
                            "聚合市场 MCP 服务告警: {}",
                            warnings.join(" | ")
                        );
                    }
                    merged
                }
                Err(e) => {
                    tracing::warn!("聚合市场 MCP 服务失败，回退为人工配置: {e}");
                    normalized_base
                }
            }
        } else {
            normalized_base
        };
        bundle.insert(kind.to_string(), normalized);
    }
    Ok(Value::Object(bundle))
}

/// 写入或更新某类系统配置的默认项（type + config_id=default）。
pub fn upsert_default_system_config(
    db: &Database,
    config_type: &str,
    name: &str,
    json_data: &Value,
) -> anyhow::Result<()> {
    let json_str = serde_json::to_string(json_data)?;
    let rows = db.list_configs(config_type).unwrap_or_default();
    let existing = rows
        .iter()
        .find(|r| r.config_id == "default")
        .or_else(|| rows.iter().find(|r| r.is_default))
        .or_else(|| rows.first());
    if let Some(row) = existing {
        let input = ConfigInput {
            r#type: config_type.to_string(),
            name: name.to_string(),
            config_id: "default".to_string(),
            provider: String::new(),
            json_data: json_str,
            enabled: true,
            is_default: true,
        };
        db.update_config(row.id, &input)?;
    } else {
        db.create_config(&ConfigInput {
            r#type: config_type.to_string(),
            name: name.to_string(),
            config_id: "default".to_string(),
            provider: String::new(),
            json_data: json_str,
            enabled: true,
            is_default: true,
        })?;
    }
    Ok(())
}

/// 将 AppConfig 中的系统级块写入 DB（导入 yaml / 批量迁移用）。
pub fn import_system_blocks_from_app_config(db: &Database, cfg: &AppConfig) -> anyhow::Result<()> {
    upsert_default_system_config(
        db,
        "auth",
        "默认认证配置",
        &serde_json::to_value(&cfg.auth)?,
    )?;
    let mut chat = cfg.chat.clone();
    if chat.global_system_prompt.is_empty() && !cfg.system_prompt.is_empty() {
        chat.global_system_prompt = cfg.system_prompt.clone();
    }
    upsert_default_system_config(
        db,
        "chat",
        "默认聊天配置",
        &serde_json::to_value(&chat)?,
    )?;
    upsert_default_system_config(
        db,
        "vision_base",
        "默认 Vision 基础配置",
        &json!({
            "enable_auth": cfg.vision.enable_auth,
            "vision_url": cfg.vision.vision_url,
        }),
    )?;
    for (kind, value) in [
        ("mqtt", serde_json::to_value(&cfg.mqtt)?),
        ("mqtt_server", serde_json::to_value(&cfg.mqtt_server)?),
        ("udp", serde_json::to_value(&cfg.udp)?),
        ("ota", serde_json::to_value(&cfg.ota)?),
        ("voice_identify", serde_json::to_value(&cfg.voice_identify)?),
        ("mcp", serde_json::to_value(&cfg.mcp)?),
        ("local_mcp", serde_json::to_value(&cfg.local_mcp)?),
    ] {
        upsert_default_system_config(
            db,
            kind,
            &format!("默认{kind}配置"),
            &value,
        )?;
    }
    Ok(())
}

/// 将 DB 中的系统配置合并进 manager 内存 `app_config`（不写 yaml）。
pub fn sync_app_config_from_db(state: &AppState) -> anyhow::Result<()> {
    let bundle = build_system_configs_data(&state.db)?;
    let mut cfg = state.app_config.write();
    apply_system_config_bundle(&mut cfg, &bundle)?;
    Ok(())
}

/// 配置保存后：刷新内存并 WS 推送给已连接的 xiaozhi-server。
pub async fn notify_system_config_changed(state: &AppState) {
    let bundle = match build_system_configs_data(&state.db) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("聚合系统配置失败: {e:#}");
            return;
        }
    };

    {
        let mut cfg = state.app_config.write();
        if let Err(e) = apply_system_config_bundle(&mut cfg, &bundle) {
            tracing::warn!("合并系统配置到内存失败: {e:#}");
        }
    }

    state.ws_hub.broadcast_system_config(bundle).await;
}
