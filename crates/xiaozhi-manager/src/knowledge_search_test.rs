//! 知识检索配置连通性测试（Manager 本地执行，无需 xiaozhi-server WS）

use std::time::Instant;

use reqwest::Client;
use serde_json::{json, Value};

const TEST_TIMEOUT_SECS: u64 = 15;

pub async fn test_knowledge_search(provider: &str, config: &Value) -> Value {
    let start = Instant::now();
    let provider = provider.trim().to_lowercase();
    let result = match provider.as_str() {
        "dify" => test_dify(config).await,
        "ragflow" => test_ragflow(config).await,
        "weknora" => test_weknora(config).await,
        other => Err(format!("不支持的 knowledge_search provider: {other}")),
    };
    let ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(detail) => json!({
            "ok": true,
            "message": detail,
            "first_packet_ms": ms,
        }),
        Err(message) => json!({
            "ok": false,
            "message": message,
            "first_packet_ms": ms,
        }),
    }
}

fn str_field(config: &Value, key: &str) -> Result<String, String> {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("缺少 {key}"))
}

fn http_client() -> Client {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(TEST_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn build_dify_url(base: &str, path: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.to_lowercase().ends_with("/v1") {
        format!("{trimmed}{path}")
    } else {
        format!("{trimmed}/v1{path}")
    }
}

fn build_ragflow_url(base: &str, path: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    let lower = trimmed.to_lowercase();
    if lower.ends_with("/api/v1") {
        format!("{trimmed}{path}")
    } else if lower.ends_with("/api") {
        format!("{trimmed}/v1{path}")
    } else {
        format!("{trimmed}/api/v1{path}")
    }
}

async fn test_dify(config: &Value) -> Result<String, String> {
    let base_url = str_field(config, "base_url")?;
    let api_key = str_field(config, "api_key")?;
    let url = format!(
        "{}?page=1&limit=1",
        build_dify_url(&base_url, "/datasets")
    );
    let resp = http_client()
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| format!("Dify 连接失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Dify 拒绝请求: HTTP {status} {text}"));
    }
    Ok("Dify 连接成功".into())
}

async fn test_ragflow(config: &Value) -> Result<String, String> {
    let base_url = str_field(config, "base_url")?;
    let api_key = str_field(config, "api_key")?;
    let url = format!(
        "{}?page=1&page_size=1",
        build_ragflow_url(&base_url, "/datasets")
    );
    let resp = http_client()
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| format!("RAGFlow 连接失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("RAGFlow 拒绝请求: HTTP {status} {text}"));
    }
    if let Ok(body) = serde_json::from_str::<Value>(&text) {
        if let Some(code) = body.get("code").and_then(|c| c.as_i64()) {
            if code != 0 {
                return Err(format!("RAGFlow 返回错误: code={code} {text}"));
            }
        }
    }
    Ok("RAGFlow 连接成功".into())
}

async fn test_weknora(config: &Value) -> Result<String, String> {
    let base_url = str_field(config, "base_url")?;
    let api_key = str_field(config, "api_key")?;
    let lists = crate::weknora_models::fetch_weknora_models(&base_url, &api_key).await?;
    let total = lists.embedding_models.len()
        + lists.llm_models.len()
        + lists.rerank_models.len();
    Ok(format!("WeKnora 连接成功（模型 {total} 个）"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dify_url_builder_handles_v1_suffix() {
        assert_eq!(
            build_dify_url("https://dify.example/v1", "/datasets"),
            "https://dify.example/v1/datasets"
        );
        assert_eq!(
            build_dify_url("https://dify.example", "/datasets"),
            "https://dify.example/v1/datasets"
        );
    }
}
