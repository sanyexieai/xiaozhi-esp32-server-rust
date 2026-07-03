use std::path::Path;

use anyhow::Context;
use serde_json::Value;

use crate::system::AppConfig;

pub fn load_config(path: impl AsRef<Path>) -> anyhow::Result<AppConfig> {
    let content = std::fs::read_to_string(path.as_ref())
        .with_context(|| format!("读取配置文件失败: {}", path.as_ref().display()))?;
    let config: AppConfig = serde_yaml::from_str(&content)?;
    Ok(config)
}

/// 配置文件存在则加载；不存在或解析失败时返回内置默认值。
pub fn load_config_optional(path: impl AsRef<Path>) -> AppConfig {
    let path = path.as_ref();
    if !path.exists() {
        return AppConfig::default();
    }
    load_config(path).unwrap_or_else(|_| AppConfig::default())
}

pub fn merge_system_config(base: &mut AppConfig, yaml_str: &str) -> anyhow::Result<()> {
    let overlay: Value = serde_yaml::from_str(yaml_str)
        .map(|v: serde_yaml::Value| serde_json::to_value(v).unwrap_or(Value::Null))?;
    let base_value = serde_json::to_value(&*base)?;
    let merged = merge_json(base_value, overlay);
    *base = serde_json::from_value(merged)?;
    Ok(())
}

/// 将管理后台下发的系统配置块（ota / udp / mqtt 等）合并进运行时配置。
pub fn apply_system_config_bundle(base: &mut AppConfig, data: &Value) -> anyhow::Result<()> {
    let Some(obj) = data.as_object() else {
        return Ok(());
    };
    if obj.is_empty() {
        return Ok(());
    }
    let mut overlay = serde_json::Map::new();
    for key in [
        "ota",
        "udp",
        "mqtt",
        "mqtt_server",
        "auth",
        "chat",
        "voice_identify",
        "mcp",
        "local_mcp",
    ] {
        if let Some(v) = obj.get(key) {
            let normalized = normalize_system_config_block(key, v.clone());
            overlay.insert(key.to_string(), normalized);
        }
    }
    if let Some(mcp_raw) = obj.get("mcp") {
        if let Some(local) = mcp_raw.get("local_mcp") {
            overlay
                .entry("local_mcp".to_string())
                .or_insert_with(|| local.clone());
        }
    }
    if !overlay.is_empty() {
        let base_value = serde_json::to_value(&*base)?;
        let merged = merge_json(base_value, Value::Object(overlay));
        *base = serde_json::from_value(merged)?;
    }
    if let Some(prompt) = data
        .get("chat")
        .and_then(|v| v.get("global_system_prompt"))
        .and_then(|v| v.as_str())
    {
        base.system_prompt = prompt.to_string();
        base.chat.global_system_prompt = prompt.to_string();
    }
    if let Some(vb) = data.get("vision_base") {
        if let Some(v) = vb.get("enable_auth").and_then(|v| v.as_bool()) {
            base.vision.enable_auth = v;
        }
        if let Some(v) = vb.get("vision_url").and_then(|v| v.as_str()) {
            base.vision.vision_url = v.to_string();
        }
    }
    if let Some(sk) = data
        .get("ota")
        .and_then(|v| v.get("signature_key"))
        .and_then(|v| v.as_str())
    {
        if !sk.is_empty() {
            base.mqtt_server.signature_key = sk.to_string();
        }
    }
    Ok(())
}

fn normalize_system_config_block(kind: &str, parsed: Value) -> Value {
    match kind {
        "mcp" => parsed.get("mcp").cloned().unwrap_or(parsed),
        "local_mcp" => parsed.get("local_mcp").cloned().unwrap_or(parsed),
        _ => parsed,
    }
}

fn merge_json(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                let merged = if let Some(existing) = base_map.remove(&k) {
                    merge_json(existing, v)
                } else {
                    v
                };
                base_map.insert(k, merged);
            }
            Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}
