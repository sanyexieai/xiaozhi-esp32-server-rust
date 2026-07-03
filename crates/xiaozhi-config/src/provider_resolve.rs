//! 从 `config.yaml` 解析 provider 配置块（config_id → 引擎类型 + 参数）

use std::collections::HashMap;

use serde_json::Value;

use crate::system::AppConfig;
use crate::user::{ProviderConfig, UConfig};

fn value_to_map(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(m) => m.into_iter().collect(),
        _ => HashMap::new(),
    }
}

fn yaml_provider_map(app: &AppConfig, kind: &str, config_id: &str) -> HashMap<String, Value> {
    let providers = match kind {
        "llm" => &app.llm.providers,
        "asr" => &app.asr.providers,
        "tts" => &app.tts.providers,
        "vad" => &app.vad.providers,
        "memory" => &app.memory.providers,
        _ => return HashMap::new(),
    };
    providers
        .get(config_id)
        .cloned()
        .map(value_to_map)
        .unwrap_or_default()
}

fn is_known_provider(kind: &str, value: &str) -> bool {
    let v = value.trim().to_lowercase();
    match kind {
        "vad" => matches!(v.as_str(), "ten_vad" | "webrtc_vad" | "silero_vad" | "webrtc"),
        "asr" => matches!(
            v.as_str(),
            "funasr" | "aliyun_funasr" | "doubao" | "aliyun_qwen3" | "xunfei"
        ),
        "tts" => matches!(
            v.as_str(),
            "doubao"
                | "doubao_ws"
                | "cosyvoice"
                | "edge"
                | "edge_offline"
                | "xiaozhi"
                | "xunfei"
                | "xunfei_super_tts"
                | "openai"
                | "zhipu"
                | "minimax"
                | "aliyun_qwen"
                | "indextts_vllm"
        ),
        "memory" => matches!(v.as_str(), "nomemo" | "memobase" | "mem0" | "memos"),
        "llm" => matches!(
            v.as_str(),
            "openai"
                | "deepseek"
                | "qwen"
                | "qwen_72b"
                | "doubao"
                | "zhipu"
                | "ollama"
                | "coze"
                | "dify"
                | "siliconflow"
        ),
        _ => false,
    }
}

fn infer_provider_from_config(kind: &str, config: &Value) -> Option<String> {
    let obj = config.as_object()?;
    match kind {
        "vad" => {
            if obj.contains_key("hop_size") {
                return Some("ten_vad".into());
            }
            if obj.contains_key("model_path") || obj.contains_key("min_silence_duration_ms") {
                return Some("silero_vad".into());
            }
            if obj.contains_key("vad_mode")
                || obj.contains_key("vad_sample_rate")
                || obj.contains_key("pool_min_size")
            {
                return Some("webrtc_vad".into());
            }
        }
        "asr" => {
            let model = obj.get("model").and_then(|v| v.as_str()).unwrap_or("");
            let ws_url = obj.get("ws_url").and_then(|v| v.as_str()).unwrap_or("");
            if obj.contains_key("appid") && obj.contains_key("api_secret") {
                return Some("xunfei".into());
            }
            if model.contains("qwen3-asr") || ws_url.contains("/realtime") {
                return Some("aliyun_qwen3".into());
            }
            if model.contains("fun-asr") || ws_url.contains("/inference") {
                return Some("aliyun_funasr".into());
            }
            if obj.contains_key("access_token")
                && obj.contains_key("resource_id")
                && obj.contains_key("end_window_size")
            {
                return Some("doubao".into());
            }
            if obj.contains_key("host") && obj.contains_key("port") {
                return Some("funasr".into());
            }
        }
        "tts" => {
            let ws_url = obj
                .get("ws_url")
                .or_else(|| obj.get("api_url"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if obj.contains_key("spk_id") {
                return Some("cosyvoice".into());
            }
            if obj.contains_key("server_url") {
                return Some("edge_offline".into());
            }
            if ws_url.contains("openspeech.bytedance.com") {
                return Some("doubao_ws".into());
            }
            if ws_url.contains("dashscope.aliyuncs.com") {
                return Some("aliyun_qwen".into());
            }
        }
        _ => {}
    }
    None
}

fn normalize_engine_provider(
    kind: &str,
    config_id: &str,
    stored_provider: &str,
    config: &Value,
) -> String {
    for candidate in [
        stored_provider,
        config
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        config_id,
    ] {
        if is_known_provider(kind, candidate) {
            return candidate.trim().to_lowercase();
        }
    }
    infer_provider_from_config(kind, config).unwrap_or_else(|| match kind {
        "vad" => "ten_vad".to_string(),
        "asr" => "aliyun_funasr".to_string(),
        "tts" => "edge".to_string(),
        "memory" => "nomemo".to_string(),
        "llm" => "openai".to_string(),
        _ => stored_provider.to_string(),
    })
}

pub fn resolve_from_app(app: &AppConfig, kind: &str) -> ProviderConfig {
    let config_id = match kind {
        "llm" => app.llm.provider.clone(),
        "asr" => app.asr.provider.clone(),
        "tts" => app.tts.provider.clone(),
        "vad" => app.vad.provider.clone(),
        "memory" => app.memory.provider.clone(),
        _ => String::new(),
    };

    let mut config = if config_id.is_empty() {
        HashMap::new()
    } else {
        yaml_provider_map(app, kind, &config_id)
    };

    if config.is_empty() {
        if let Some(active) = match kind {
            "llm" => app.llm.active_config(),
            "asr" => app.asr.active_config(),
            "tts" => app.tts.active_config(),
            "vad" => app.vad.active_config(),
            "memory" => app.memory.active_config(),
            _ => None,
        } {
            config = value_to_map(active.clone());
        }
    }

    let cfg_value = Value::Object(
        config
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    let provider = normalize_engine_provider(kind, &config_id, &config_id, &cfg_value);
    config.insert("provider".into(), Value::String(provider.clone()));

    ProviderConfig { provider, config }
}

/// 将 Redis 中某模块的 JSON 覆盖项与 `config.yaml` 默认 provider 块合并（对齐 Go `getConfigByType`）。
pub fn merge_redis_provider_config(
    app: &AppConfig,
    kind: &str,
    redis_cfg: &HashMap<String, Value>,
) -> ProviderConfig {
    let app_config_id = match kind {
        "llm" => app.llm.provider.as_str(),
        "asr" => app.asr.provider.as_str(),
        "tts" => app.tts.provider.as_str(),
        "vad" => app.vad.provider.as_str(),
        "memory" => app.memory.provider.as_str(),
        _ => "",
    };

    let stored_provider = redis_cfg
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let lookup_id = if !stored_provider.is_empty() {
        stored_provider
    } else {
        app_config_id
    };

    let mut config = if lookup_id.is_empty() {
        HashMap::new()
    } else {
        yaml_provider_map(app, kind, lookup_id)
    };

    if config.is_empty() {
        if let Some(active) = match kind {
            "llm" => app.llm.active_config(),
            "asr" => app.asr.active_config(),
            "tts" => app.tts.active_config(),
            "vad" => app.vad.active_config(),
            "memory" => app.memory.active_config(),
            _ => None,
        } {
            config = value_to_map(active.clone());
        }
    }

    for (k, v) in redis_cfg {
        if k == "provider" {
            continue;
        }
        config.insert(k.clone(), v.clone());
    }

    let cfg_value = Value::Object(config.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    let provider = normalize_engine_provider(kind, lookup_id, stored_provider, &cfg_value);
    config.insert("provider".into(), Value::String(provider.clone()));

    ProviderConfig { provider, config }
}

/// Redis hash 字段 → 可写回的 JSON 字符串
pub fn provider_config_to_redis_json(p: &ProviderConfig) -> HashMap<String, Value> {
    let mut m = p.config.clone();
    m.insert("provider".into(), Value::String(p.provider.clone()));
    m
}

impl UConfig {
    /// manager 不可达或未绑定设备时，从 `config.yaml` 解析完整 UConfig（含 api_key 等参数）。
    pub fn from_app_config(app: &AppConfig) -> Self {
        Self {
            system_prompt: app.system_prompt.clone(),
            vad: resolve_from_app(app, "vad"),
            asr: resolve_from_app(app, "asr"),
            tts: resolve_from_app(app, "tts"),
            llm: resolve_from_app(app, "llm"),
            memory: resolve_from_app(app, "memory"),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_config;

    #[test]
    fn resolves_aliyun_funasr_default_config_id() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = format!("{manifest}/../../config/config.yaml");
        let app = load_config(&path).expect("load config");
        let u = UConfig::from_app_config(&app);
        assert_eq!(u.asr.provider, "aliyun_funasr");
        assert!(
            u.asr
                .config
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .starts_with("sk-ws-"),
            "expected api_key from aliyun_funasr_default block"
        );
    }
}
