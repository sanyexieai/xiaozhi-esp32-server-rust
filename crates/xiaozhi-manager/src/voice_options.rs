use axum::http::StatusCode;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashSet;

use crate::app::AppState;
use crate::db::ConfigRow;
use crate::voice_constants::{
    get_aliyun_qwen_voices_by_model, get_voice_options_by_provider, VoiceOption,
};

const INDEX_TTS_VOICES_ENDPOINT: &str = "/audio/voices";

#[derive(Debug, Clone)]
pub struct VoiceOptionsQuery {
    pub provider: String,
    pub config_id: Option<String>,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
}

pub async fn resolve_voice_options(
    state: &AppState,
    target_user_id: Option<i64>,
    query: VoiceOptionsQuery,
) -> Result<Vec<Value>, (StatusCode, String)> {
    let provider = query.provider.trim();
    if provider.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "provider参数必填".into()));
    }

    let mut system_voices = if provider == "indextts_vllm" {
        fetch_indextts_voices(state, &query)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e))?
    } else if provider == "aliyun_qwen" {
        resolve_aliyun_qwen_voices(state, &query)?
    } else {
        get_voice_options_by_provider(provider)
    };

    if system_voices.is_empty() && provider == "edge_tts" {
        system_voices = get_voice_options_by_provider("edge");
    }

    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for voice in system_voices {
        let key = voice.value.trim();
        if key.is_empty() || !seen.insert(key.to_string()) {
            continue;
        }
        result.push(voice.to_json());
    }

    if let (Some(user_id), Some(config_id)) = (target_user_id, query.config_id.as_deref()) {
        let config_id = config_id.trim();
        if !config_id.is_empty() {
            merge_voice_clones(state, user_id, provider, config_id, &mut result, &mut seen);
            merge_admin_shared_voice_clones(
                state,
                user_id,
                provider,
                config_id,
                &mut result,
                &mut seen,
            );
        }
    }

    Ok(result)
}

fn resolve_aliyun_qwen_voices(
    state: &AppState,
    query: &VoiceOptionsQuery,
) -> Result<Vec<VoiceOption>, (StatusCode, String)> {
    let config_id = query.config_id.as_deref().unwrap_or("").trim();
    if config_id.is_empty() {
        return Ok(get_voice_options_by_provider("aliyun_qwen"));
    }
    let cfg = find_tts_config(state, config_id)?
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "未找到对应的TTS配置".into()))?;
    let model = parse_tts_model(&cfg.json_data).unwrap_or_else(|| "qwen3-tts-flash".to_string());
    Ok(get_aliyun_qwen_voices_by_model(&model))
}

fn find_tts_config(
    state: &AppState,
    config_id: &str,
) -> Result<Option<ConfigRow>, (StatusCode, String)> {
    state
        .db
        .list_configs("tts")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        .map(|rows| rows.into_iter().find(|row| row.config_id == config_id))
}

fn parse_tts_model(json_data: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json_data).ok()?;
    value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn merge_voice_clones(
    state: &AppState,
    user_id: i64,
    provider: &str,
    config_id: &str,
    result: &mut Vec<Value>,
    seen: &mut HashSet<String>,
) {
    if let Ok(clones) = state.db.list_voice_clones(user_id) {
        for clone in clones {
            if clone.provider != provider
                || clone.tts_config_id != config_id
                || !crate::voice_clone_api::is_clone_active_status(&clone.status)
            {
                continue;
            }
            let voice_id = clone.voice_id.clone().unwrap_or_default();
            let key = voice_id.trim();
            if key.is_empty() {
                continue;
            }
            if seen.contains(key) {
                result.retain(|item| item.get("value").and_then(|v| v.as_str()) != Some(key));
            }
            seen.insert(key.to_string());
            result.push(serde_json::json!({
                "value": key,
                "label": format!("[我的复刻] {} ({})", clone.name, key),
            }));
        }
    }
}

fn merge_admin_shared_voice_clones(
    state: &AppState,
    user_id: i64,
    provider: &str,
    config_id: &str,
    result: &mut Vec<Value>,
    seen: &mut HashSet<String>,
) {
    if let Ok(clones) = state
        .db
        .list_admin_shared_voice_clones(user_id, provider, config_id)
    {
        for clone in clones {
            let voice_id = clone.voice_id.clone().unwrap_or_default();
            let key = voice_id.trim();
            if key.is_empty() || seen.contains(key) {
                continue;
            }
            seen.insert(key.to_string());
            result.push(serde_json::json!({
                "value": key,
                "label": format!("[管理员共享] {} ({})", clone.name, key),
            }));
        }
    }
}

fn normalize_indextts_base_url(raw: &str) -> String {
    let mut base = raw.trim().trim_end_matches('/').to_string();
    for suffix in ["/audio/speech", "/audio/voices"] {
        if base.to_ascii_lowercase().ends_with(suffix) {
            base = base[..base.len() - suffix.len()].trim_end_matches('/').to_string();
        }
    }
    if base.is_empty() {
        "http://127.0.0.1:7860".to_string()
    } else {
        base
    }
}

async fn fetch_indextts_voices(
    state: &AppState,
    query: &VoiceOptionsQuery,
) -> Result<Vec<VoiceOption>, String> {
    let mut base_url = "http://127.0.0.1:7860".to_string();
    let mut api_key = String::new();

    if let Some(config_id) = query.config_id.as_deref() {
        let config_id = config_id.trim();
        if !config_id.is_empty() {
            if let Ok(Some(cfg)) = state.db.list_configs("tts").map(|rows| {
                rows.into_iter().find(|row| row.config_id == config_id)
            }) {
                if let Ok(value) = serde_json::from_str::<Value>(&cfg.json_data) {
                    if let Some(url) = value.get("api_url").and_then(|v| v.as_str()) {
                        if !url.trim().is_empty() {
                            base_url = url.trim().to_string();
                        }
                    }
                    if let Some(key) = value.get("api_key").and_then(|v| v.as_str()) {
                        api_key = key.trim().to_string();
                    }
                }
            }
        }
    }

    if let Some(url) = query.api_url.as_deref() {
        if !url.trim().is_empty() {
            base_url = url.trim().to_string();
        }
    }
    if let Some(key) = query.api_key.as_deref() {
        if !key.trim().is_empty() {
            api_key = key.trim().to_string();
        }
    }

    base_url = normalize_indextts_base_url(&base_url);
    let url = format!("{base_url}{INDEX_TTS_VOICES_ENDPOINT}");

    let mut builder = Client::builder().connect_timeout(std::time::Duration::from_secs(10));
    if should_bypass_proxy(&base_url) {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let mut req = client.get(&url).header("Accept", "application/json");
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("IndexTTS 获取音色失败: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("IndexTTS 读取响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "IndexTTS 获取音色失败: status={} body={}",
            status.as_u16(),
            body.trim()
        ));
    }

    let voice_map: Value =
        serde_json::from_str(&body).map_err(|e| format!("IndexTTS 响应解析失败: {e}"))?;
    let Some(obj) = voice_map.as_object() else {
        return Ok(Vec::new());
    };

    let config_prefix = query
        .config_id
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let normalized_prefix = if config_prefix.is_empty() {
        String::new()
    } else {
        format!("{config_prefix}_")
    };

    let mut voices = Vec::new();
    for key in obj.keys() {
        let voice = key.trim();
        if voice.is_empty() {
            continue;
        }
        if !normalized_prefix.is_empty()
            && voice.to_ascii_lowercase().starts_with(&normalized_prefix)
        {
            continue;
        }
        voices.push(VoiceOption {
            value: voice.to_string(),
            label: voice.to_string(),
        });
    }
    voices.sort_by(|a, b| a.value.cmp(&b.value));
    Ok(voices)
}

fn should_bypass_proxy(base_url: &str) -> bool {
    let base = base_url.to_ascii_lowercase();
    [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "siliconflow.cn",
        "bigmodel.cn",
        "dashscope.aliyuncs.com",
        "volces.com",
        "volcengine",
        "deepseek.com",
    ]
    .iter()
    .any(|host| base.contains(host))
}
