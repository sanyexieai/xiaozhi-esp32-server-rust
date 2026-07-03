//! 云 API 连接通用工具（与 Go 侧 dashscope / volces 等用法对齐）

/// 对已知云厂商域名禁用 HTTP 代理，避免本地代理导致 401/超时。
pub fn should_bypass_proxy(url: &str) -> bool {
    let url = url.to_lowercase();
    [
        "dashscope.aliyuncs.com",
        "dashscope-intl.aliyuncs.com",
        "openspeech.bytedance.com",
        "bytedance.com",
        "volces.com",
        "volcengine",
        "ark.cn-",
        "siliconflow.cn",
        "bigmodel.cn",
        "minimaxi.com",
        "api.coze.cn",
        "deepseek.com",
        "api.openai.com",
        "localhost",
        "127.0.0.1",
    ]
    .iter()
    .any(|host| url.contains(host))
}

pub fn trimmed_config_string(config: &serde_json::Value, key: &str) -> String {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// DashScope：`api_key` trim 后为空则读 `DASHSCOPE_API_KEY`。
pub fn dashscope_api_key(config: &serde_json::Value) -> String {
    let mut key = trimmed_config_string(config, "api_key");
    if key.is_empty() {
        key = std::env::var("DASHSCOPE_API_KEY").unwrap_or_default();
    }
    key.trim().to_string()
}

pub fn normalize_doubao_asr_ws_url(url: &str) -> String {
    if url.contains("bigmodel_nostream") {
        url.replace("bigmodel_nostream", "bigmodel_async")
    } else {
        url.to_string()
    }
}

/// Qwen3 / HTTP 类 DashScope 接口应使用 `sk-` 密钥（非 FunASR 的 `sk-ws-`）。
pub fn dashscope_http_api_key_issue(key: &str) -> Option<&'static str> {
    let key = key.trim();
    if key.is_empty() {
        return Some("api_key 未配置（或设置环境变量 DASHSCOPE_API_KEY）");
    }
    if key.starts_with("sk-ws-") {
        return Some("Qwen3 ASR 需 sk- 开头的 DashScope API Key；sk-ws- 密钥仅适用于 FunASR WebSocket");
    }
    if !key.starts_with("sk-") {
        return Some("api_key 格式无效，请在百炼控制台创建 DashScope API Key（sk- 开头）");
    }
    None
}

pub fn dashscope_ws_auth_error_hint(key: &str, product: &str) -> String {
    if let Some(issue) = dashscope_http_api_key_issue(key) {
        return issue.to_string();
    }
    format!(
        "{product} 认证失败 (401)：请确认 API Key 有效、账户余额正常，且 ws_url 区域与密钥一致（北京: dashscope.aliyuncs.com，新加坡: dashscope-intl.aliyuncs.com）"
    )
}
