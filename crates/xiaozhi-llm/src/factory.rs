use std::sync::Arc;

use serde_json::{json, Value};
use xiaozhi_core::{Error, Result, llm as llm_const};

use crate::traits::LlmProvider;
use crate::{coze, dify, openai};

const KNOWN_LLM_PROVIDERS: &[&str] = &[
    "openai", "ollama", "azure", "anthropic", "zhipu", "aliyun", "doubao", "siliconflow",
    "deepseek", "dify", "coze",
];

pub fn create_llm(provider: &str, config: &serde_json::Value) -> Result<Arc<dyn LlmProvider>> {
    let mut cfg = config.clone();
    let pool_key = provider.trim();
    let stored = cfg
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let provider_name = normalize_llm_provider(pool_key, stored, &cfg);

    if let Some(obj) = cfg.as_object_mut() {
        obj.insert("provider".into(), json!(provider_name));
    }

    let llm_type = resolve_llm_type(&provider_name, &cfg);
    if let Some(obj) = cfg.as_object_mut() {
        obj.insert("type".into(), json!(llm_type));
    }

    let provider_key = resolve_llm_provider_name(&provider_name, &cfg, &llm_type);
    apply_default_base_url(&provider_key, &mut cfg);

    match llm_type.as_str() {
        llm_const::OPENAI | llm_const::OLLAMA | llm_const::EINO | llm_const::EINO_LLM => {
            Ok(Arc::new(openai::OpenAiLlmProvider::from_config(&cfg)?))
        }
        llm_const::DIFY => Ok(Arc::new(dify::DifyLlmProvider::from_config(&cfg)?)),
        llm_const::COZE => Ok(Arc::new(coze::CozeLlmProvider::from_config(&cfg)?)),
        other => Err(Error::Unsupported(format!("不支持的 LLM 类型: {other}"))),
    }
}

/// 与 Golang `configprovider.NormalizeProvider` 对齐，并在有 `base_url` 时优先按地址推断，
/// 避免 config_id 为 `deepseek` 但走硅基流动代理时被误判为官方 DeepSeek。
pub fn normalize_llm_provider(config_id: &str, stored_provider: &str, config: &Value) -> String {
    if let Some(provider) = infer_llm_provider_from_base_url(config) {
        return provider;
    }

    for candidate in [
        config.get("provider").and_then(|v| v.as_str()).unwrap_or(""),
        stored_provider,
        config_id,
    ] {
        if let Some(provider) = known_llm_provider(candidate) {
            return provider;
        }
    }

    let llm_type = config
        .get("type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_lowercase();
    if let Some(provider) = known_llm_provider(&llm_type) {
        if provider != "openai" {
            return provider.to_string();
        }
    }

    if llm_type == "openai" || llm_type.is_empty() {
        return "openai".to_string();
    }

    let fallback = stored_provider.trim();
    if !fallback.is_empty() {
        return fallback.to_lowercase();
    }
    config_id.trim().to_lowercase()
}

fn known_llm_provider(provider: &str) -> Option<String> {
    let provider = provider.trim().to_lowercase();
    if provider.is_empty() {
        return None;
    }
    KNOWN_LLM_PROVIDERS
        .iter()
        .find(|p| **p == provider)
        .map(|p| (*p).to_string())
}

fn infer_llm_provider_from_base_url(config: &Value) -> Option<String> {
    let base_url = config
        .get("base_url")
        .or_else(|| config.get("api_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();

    if base_url.is_empty() {
        return None;
    }
    if base_url.contains("openai.azure.com") {
        return Some("azure".into());
    }
    if base_url.contains("anthropic.com") {
        return Some("anthropic".into());
    }
    if base_url.contains("bigmodel.cn") {
        return Some("zhipu".into());
    }
    if base_url.contains("dashscope.aliyuncs.com") {
        return Some("aliyun".into());
    }
    if base_url.contains("volces.com")
        || base_url.contains("volcengineapi.com")
        || base_url.contains("ark.cn-")
    {
        return Some("doubao".into());
    }
    if base_url.contains("siliconflow.cn") {
        return Some("siliconflow".into());
    }
    if base_url.contains("deepseek.com") {
        return Some("deepseek".into());
    }
    if base_url.contains("localhost:11434") || base_url.contains("127.0.0.1:11434") {
        return Some("ollama".into());
    }
    None
}

fn resolve_llm_provider_name(provider_name: &str, config: &Value, llm_type: &str) -> String {
    let mut provider = provider_name.trim().to_lowercase();
    if provider.is_empty() {
        if let Some(p) = config.get("provider").and_then(|v| v.as_str()) {
            provider = p.trim().to_lowercase();
        }
    }

    if provider == "openai" {
        match llm_type {
            llm_const::OLLAMA => return llm_const::OLLAMA.to_string(),
            llm_const::DIFY => return llm_const::DIFY.to_string(),
            llm_const::COZE => return llm_const::COZE.to_string(),
            _ => {}
        }
    }

    provider
}

fn apply_default_base_url(provider: &str, config: &mut Value) {
    let has_url = config
        .get("base_url")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if has_url {
        return;
    }

    if let Some(url) = default_base_url(provider) {
        if let Some(obj) = config.as_object_mut() {
            obj.insert("base_url".into(), json!(url));
        }
        return;
    }

    if let Some(obj) = config.as_object_mut() {
        obj.remove("base_url");
    }
}

fn resolve_llm_type(provider: &str, config: &serde_json::Value) -> String {
    let mut provider_name = provider.trim().to_lowercase();
    if provider_name.is_empty() {
        if let Some(p) = config.get("provider").and_then(|v| v.as_str()) {
            provider_name = p.trim().to_lowercase();
        }
    }

    let llm_type = config
        .get("type")
        .and_then(|v| v.as_str())
        .map(|t| t.trim().to_lowercase())
        .unwrap_or_default();

    if provider_name == "openai" {
        match llm_type.as_str() {
            llm_const::OLLAMA => return llm_const::OLLAMA.to_string(),
            llm_const::DIFY => return llm_const::DIFY.to_string(),
            llm_const::COZE => return llm_const::COZE.to_string(),
            _ => {}
        }
    }

    match provider_name.as_str() {
        "ollama" => return llm_const::OLLAMA.to_string(),
        "dify" => return llm_const::DIFY.to_string(),
        "coze" => return llm_const::COZE.to_string(),
        "openai" | "azure" | "anthropic" | "zhipu" | "aliyun" | "doubao" | "siliconflow"
        | "deepseek" => return llm_const::OPENAI.to_string(),
        _ => {}
    }

    match llm_type.as_str() {
        llm_const::OLLAMA => llm_const::OLLAMA.to_string(),
        llm_const::DIFY => llm_const::DIFY.to_string(),
        llm_const::COZE => llm_const::COZE.to_string(),
        llm_const::OPENAI | llm_const::EINO | llm_const::EINO_LLM => llm_const::OPENAI.to_string(),
        _ if !llm_type.is_empty() => llm_const::OPENAI.to_string(),
        _ => llm_const::OPENAI.to_string(),
    }
}

pub fn default_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("https://api.anthropic.com/v1/"),
        "zhipu" => Some("https://open.bigmodel.cn/api/paas/v4"),
        "aliyun" => Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        "doubao" => Some("https://ark.cn-beijing.volces.com/api/v3"),
        "siliconflow" => Some("https://api.siliconflow.cn/v1"),
        "deepseek" => Some("https://api.deepseek.com/v1"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_siliconflow_from_yaml_preset() {
        let cfg = json!({
            "type": "openai",
            "base_url": "https://api.siliconflow.cn/v1",
            "model_name": "Qwen/Qwen2.5-72B-Instruct"
        });
        assert_eq!(
            normalize_llm_provider("qwen_72b", "qwen_72b", &cfg),
            "siliconflow"
        );
    }

    #[test]
    fn keeps_explicit_deepseek_provider() {
        let cfg = json!({
            "type": "openai",
            "base_url": "https://api.deepseek.com/v1",
            "model_name": "deepseek-chat"
        });
        assert_eq!(
            normalize_llm_provider("deepseek", "deepseek", &cfg),
            "deepseek"
        );
    }

    #[test]
    fn preserves_existing_base_url_for_siliconflow_preset_named_deepseek() {
        let mut cfg = json!({
            "type": "openai",
            "base_url": "https://api.siliconflow.cn/v1",
            "model_name": "Pro/deepseek-ai/DeepSeek-V3"
        });
        let provider = normalize_llm_provider("deepseek", "deepseek", &cfg);
        apply_default_base_url(&provider, &mut cfg);
        assert_eq!(provider, "siliconflow");
        assert_eq!(
            cfg["base_url"].as_str().unwrap(),
            "https://api.siliconflow.cn/v1"
        );
    }

    #[tokio::test]
    async fn reqwest_https_can_connect_without_dead_local_proxy() {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("client");
        let resp = client
            .get("https://api.siliconflow.cn/v1/models")
            .header("Authorization", "Bearer test")
            .send()
            .await
            .expect("https request should connect");
        assert!(resp.status().as_u16() == 401 || resp.status().is_success());
    }
}
