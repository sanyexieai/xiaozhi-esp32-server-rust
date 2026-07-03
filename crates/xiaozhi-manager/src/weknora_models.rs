//! WeKnora 模型列表（对齐 Go `ListWeknoraModels`）

use std::collections::HashSet;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct WeknoraModelOption {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub model_type: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeknoraModelLists {
    pub embedding_models: Vec<WeknoraModelOption>,
    pub llm_models: Vec<WeknoraModelOption>,
    pub rerank_models: Vec<WeknoraModelOption>,
    pub all_models: Vec<WeknoraModelOption>,
}

pub async fn fetch_weknora_models(base_url: &str, api_key: &str) -> Result<WeknoraModelLists, String> {
    let base_url = base_url.trim();
    let api_key = api_key.trim();
    if base_url.is_empty() || api_key.is_empty() {
        return Err("base_url 和 api_key 不能为空".into());
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let mut last_err = String::new();
    for endpoint in build_weknora_model_list_candidate_endpoints(base_url) {
        match do_weknora_get(&client, &endpoint, api_key).await {
            Ok(body) => {
                let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                let all_models = extract_weknora_model_options(&parsed);
                return Ok(partition_weknora_models(all_models));
            }
            Err(e) => {
                last_err = e;
            }
        }
    }
    Err(if last_err.is_empty() {
        "拉取 WeKnora 模型列表失败".into()
    } else {
        format!("拉取 WeKnora 模型列表失败: {last_err}")
    })
}

async fn do_weknora_get(client: &Client, endpoint: &str, api_key: &str) -> Result<String, String> {
    let resp = client
        .get(endpoint)
        .header("X-API-Key", api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {body}"));
    }
    Ok(body)
}

fn build_weknora_model_list_candidate_endpoints(base_url: &str) -> Vec<String> {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut candidates = vec![
        build_weknora_url(trimmed, "/models"),
        build_weknora_url(trimmed, "/model"),
        format!("{trimmed}/models"),
        format!("{trimmed}/model"),
    ];
    let lower = trimmed.to_lowercase();
    if !lower.ends_with("/v1") {
        candidates.push(format!("{trimmed}/v1/models"));
        candidates.push(format!("{trimmed}/v1/model"));
    }
    if !lower.ends_with("/api/v1") {
        candidates.push(format!("{trimmed}/api/v1/models"));
        candidates.push(format!("{trimmed}/api/v1/model"));
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

fn build_weknora_url(base_url: &str, path: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let lower = trimmed.to_lowercase();
    if lower.ends_with("/api/v1") {
        format!("{trimmed}{path}")
    } else if lower.ends_with("/api") {
        format!("{trimmed}/v1{path}")
    } else {
        format!("{trimmed}/api/v1{path}")
    }
}

fn extract_weknora_model_options(parsed: &Value) -> Vec<WeknoraModelOption> {
    let mut model_maps: Vec<serde_json::Map<String, Value>> = Vec::new();
    collect_weknora_model_maps(parsed, 0, &mut model_maps);
    let mut seen = HashSet::new();
    let mut options = Vec::new();
    for item in model_maps {
        let id = first_non_empty_json(&item, &["model_id", "id", "uid"]);
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let name = first_non_empty_json(
            &item,
            &["display_name", "name", "model_name", "model_id", "id"],
        );
        let name = if name.is_empty() { id.clone() } else { name };
        options.push(WeknoraModelOption {
            id: id.clone(),
            name,
            model_type: first_non_empty_json(
                &item,
                &["model_type", "type", "category", "task_type", "capability"],
            ),
            provider: first_non_empty_json(&item, &["provider", "vendor"]),
        });
    }
    options.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    options
}

fn collect_weknora_model_maps(
    raw: &Value,
    depth: usize,
    out: &mut Vec<serde_json::Map<String, Value>>,
) {
    if depth > 6 {
        return;
    }
    match raw {
        Value::Array(arr) => {
            for item in arr {
                collect_weknora_model_maps(item, depth + 1, out);
            }
        }
        Value::Object(map) => {
            if is_likely_weknora_model_record(map) {
                out.push(map.clone());
            }
            for key in ["data", "list", "items", "models", "rows", "records", "results", "model"]
            {
                if let Some(next) = map.get(key) {
                    collect_weknora_model_maps(next, depth + 1, out);
                }
            }
        }
        _ => {}
    }
}

fn is_likely_weknora_model_record(map: &serde_json::Map<String, Value>) -> bool {
    if !first_non_empty_json(map, &["model_id"]).is_empty() {
        return true;
    }
    if first_non_empty_json(map, &["id"]).is_empty() {
        return false;
    }
    !first_non_empty_json(map, &["name", "model_name", "display_name"]).is_empty()
        || !first_non_empty_json(map, &["model_type", "type", "category", "task_type"]).is_empty()
}

fn first_non_empty_json(map: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = map.get(*key) {
            if let Some(s) = v.as_str() {
                let s = s.trim();
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    String::new()
}

fn partition_weknora_models(all_models: Vec<WeknoraModelOption>) -> WeknoraModelLists {
    let mut embedding_models = Vec::new();
    let mut llm_models = Vec::new();
    let mut rerank_models = Vec::new();
    for item in &all_models {
        if is_weknora_embedding_model(item) {
            embedding_models.push(item.clone());
        } else if is_weknora_rerank_model(item) {
            rerank_models.push(item.clone());
        } else if is_weknora_llm_model(item) {
            llm_models.push(item.clone());
        }
    }
    WeknoraModelLists {
        embedding_models,
        llm_models,
        rerank_models,
        all_models,
    }
}

fn is_weknora_embedding_model(model: &WeknoraModelOption) -> bool {
    let corpus = format!("{} {} {}", model.id, model.name, model.model_type).to_lowercase();
    corpus.contains("embedding") || corpus.contains("embed")
}

fn is_weknora_rerank_model(model: &WeknoraModelOption) -> bool {
    let corpus = format!("{} {} {}", model.id, model.name, model.model_type).to_lowercase();
    corpus.contains("rerank") || corpus.contains("re-rank")
}

fn is_weknora_llm_model(model: &WeknoraModelOption) -> bool {
    !is_weknora_embedding_model(model) && !is_weknora_rerank_model(model)
}
