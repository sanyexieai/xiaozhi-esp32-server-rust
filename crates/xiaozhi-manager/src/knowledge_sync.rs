//! 外部知识库同步（对齐 Go `knowledge_sync.go`）

use std::sync::Arc;
use std::time::Duration;

use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use urlencoding::encode;

use crate::db::Database;
use crate::knowledge_search::external_kb_id_from_json;
use crate::knowledge_upload::{
    decode_upload_content, sanitize_upload_file_name, KNOWLEDGE_DOCUMENT_UPLOAD_MAX_BYTES,
};

const DIFY_HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const DIFY_FILE_TIMEOUT: Duration = Duration::from_secs(90);
const DIFY_FILE_UPLOAD_MAX_ATTEMPTS: u32 = 3;
const DIFY_FILE_UPLOAD_RETRY_STEP: Duration = Duration::from_secs(2);
const RAGFLOW_HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const WEKNORA_HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const WEKNORA_FILE_TIMEOUT: Duration = Duration::from_secs(90);
const WEKNORA_PARSE_POLL: Duration = Duration::from_millis(1000);
const WEKNORA_PARSE_TIMEOUT: Duration = Duration::from_millis(120_000);

pub const SYNC_STATUS_PENDING: &str = "pending";
pub const SYNC_STATUS_SYNCED: &str = "synced";
pub const SYNC_STATUS_UPLOADING: &str = "uploading";
pub const SYNC_STATUS_UPLOAD_FAILED: &str = "upload_failed";
pub const SYNC_STATUS_PARSE_FAILED: &str = "parse_failed";
pub const SYNC_STATUS_FAILED: &str = "failed";

#[derive(Clone, Debug)]
pub struct KbSyncSnapshot {
    pub provider: String,
    pub config_json: String,
}

#[derive(Clone, Debug)]
pub struct KbDocumentDeleteSnapshot {
    pub external_doc_id: String,
}

impl KbSyncSnapshot {
    pub fn from_kb(provider: &str, config_json: &str) -> Self {
        Self {
            provider: provider.trim().to_lowercase(),
            config_json: config_json.to_string(),
        }
    }
}

pub fn mark_kb_sync_pending(db: &Database, kb_id: i64) -> Result<(), String> {
    db.merge_kb_config_json(
        kb_id,
        &json!({
            "sync_status": SYNC_STATUS_PENDING,
            "sync_error": "",
        }),
    )
    .map_err(|e| e.to_string())
}

pub fn mark_document_sync_pending(db: &Database, kb_id: i64, doc_id: i64) -> Result<(), String> {
    db.update_kb_document_sync_state(kb_id, doc_id, None, SYNC_STATUS_PENDING, Some(""))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn spawn_knowledge_base_sync(db: Arc<Database>, kb_id: i64) {
    tokio::spawn(async move {
        if let Err(e) = sync_knowledge_base(&db, kb_id).await {
            let _ = persist_kb_sync_failure(&db, kb_id, &e);
            tracing::warn!(kb_id, "知识库同步失败: {e}");
        }
    });
}

pub fn spawn_knowledge_document_sync(db: Arc<Database>, kb_id: i64, doc_id: i64) {
    tokio::spawn(async move {
        if let Err(e) = sync_knowledge_document(&db, kb_id, doc_id).await {
            tracing::warn!(kb_id, doc_id, "知识库文档同步失败: {e}");
        }
    });
}

pub fn spawn_knowledge_base_delete_sync(db: Arc<Database>, snapshot: KbSyncSnapshot) {
    tokio::spawn(async move {
        if let Err(e) = sync_delete_knowledge_base(&db, &snapshot).await {
            tracing::warn!("知识库删除同步失败: {e}");
        }
    });
}

pub fn spawn_knowledge_document_delete_sync(
    db: Arc<Database>,
    kb: KbSyncSnapshot,
    doc: KbDocumentDeleteSnapshot,
) {
    tokio::spawn(async move {
        if let Err(e) = sync_delete_document(&db, &kb, &doc).await {
            tracing::warn!("知识库文档删除同步失败: {e}");
        }
    });
}

pub async fn sync_knowledge_base(db: &Database, kb_id: i64) -> Result<(), String> {
    let kb = db
        .get_knowledge_base(kb_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "知识库不存在".to_string())?;
    let provider = kb.provider.trim().to_lowercase();
    if provider == "local" || provider.is_empty() {
        return Ok(());
    }
    let (_, provider_data) = load_provider_config(db, &provider)?;
    match provider.as_str() {
        "dify" => {
            let cfg = parse_dify_config(&provider_data)?;
            ensure_dify_dataset(db, kb_id, &kb.name, &kb.description, &cfg).await?;
        }
        "ragflow" => {
            let cfg = parse_ragflow_config(&provider_data)?;
            ensure_ragflow_dataset(db, kb_id, &kb.name, &kb.description, &cfg).await?;
        }
        "weknora" => {
            let cfg = parse_weknora_config(&provider_data)?;
            ensure_weknora_dataset(db, kb_id, &kb.name, &kb.description, &cfg).await?;
        }
        other => return Err(format!("知识库同步暂不支持 provider: {other}")),
    }
    persist_kb_sync_success(db, kb_id)?;
    Ok(())
}

pub async fn sync_knowledge_document(db: &Database, kb_id: i64, doc_id: i64) -> Result<(), String> {
    let kb_row = db
        .get_knowledge_base(kb_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "知识库不存在".to_string())?;
    let doc = db
        .get_kb_document(kb_id, doc_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "文档不存在".to_string())?;

    let provider = kb_row.provider.trim().to_lowercase();
    if provider == "local" || provider.is_empty() {
        let _ = db.update_kb_document_sync_state(kb_id, doc_id, None, SYNC_STATUS_SYNCED, Some(""));
        return Ok(());
    }

    let fail_upload = |db: &Database, external_id: &str, err: String| {
        let _ = db.update_kb_document_sync_state(
            kb_id,
            doc_id,
            if external_id.is_empty() {
                None
            } else {
                Some(external_id)
            },
            SYNC_STATUS_UPLOAD_FAILED,
            Some(&truncate_error(&err)),
        );
        Err(err)
    };

    let _ = db.update_kb_document_sync_state(
        kb_id,
        doc_id,
        if doc.external_doc_id.is_empty() {
            None
        } else {
            Some(&doc.external_doc_id)
        },
        SYNC_STATUS_UPLOADING,
        Some(""),
    );

    let (upload_name, upload_data, is_upload) = match decode_upload_content(&doc.content) {
        Ok(v) => v,
        Err(e) => return fail_upload(db, &doc.external_doc_id, e),
    };
    let text_content = doc.content.trim().to_string();

    let (_, provider_data) = load_provider_config(db, &provider)?;
    match provider.as_str() {
        "dify" => {
            let cfg = parse_dify_config(&provider_data).map_err(|e| {
                fail_upload(db, &doc.external_doc_id, e).unwrap_err()
            })?;
            let client = http_client(if is_upload {
                DIFY_FILE_TIMEOUT
            } else {
                DIFY_HTTP_TIMEOUT
            });
            let dataset_id = ensure_dify_dataset(db, kb_id, &kb_row.name, &kb_row.description, &cfg)
                .await
                .map_err(|e| fail_upload(db, &doc.external_doc_id, e).unwrap_err())?;
            let old_id = doc.external_doc_id.trim().to_string();
            let fail_id = old_id.clone();
            let document_id = if is_upload {
                if old_id.is_empty() {
                    dify_create_document_by_file(&client, &cfg, &dataset_id, &upload_name, &upload_data)
                        .await
                } else {
                    dify_replace_document_by_file(
                        &client,
                        &cfg,
                        &dataset_id,
                        &old_id,
                        &upload_name,
                        &upload_data,
                    )
                    .await
                }
            } else {
                if text_content.is_empty() {
                    return fail_upload(db, &old_id, "文档内容为空，无法同步".to_string());
                }
                if old_id.is_empty() {
                    dify_create_document_by_text(&client, &cfg, &dataset_id, &doc.title, &text_content)
                        .await
                } else {
                    dify_update_document_by_text(
                        &client,
                        &cfg,
                        &dataset_id,
                        &old_id,
                        &doc.title,
                        &text_content,
                    )
                    .await
                    .map(|_| old_id.clone())
                }
            }
            .map_err(|e| fail_upload(db, &fail_id, e).unwrap_err())?;
            db.update_kb_document_sync_state(
                kb_id,
                doc_id,
                Some(&document_id),
                SYNC_STATUS_SYNCED,
                Some(""),
            )
            .map_err(|e| e.to_string())?;
        }
        "ragflow" => {
            if !is_upload && text_content.is_empty() {
                return fail_upload(db, &doc.external_doc_id, "文档内容为空，无法同步".to_string());
            }
            let cfg = parse_ragflow_config(&provider_data).map_err(|e| {
                fail_upload(db, &doc.external_doc_id, e).unwrap_err()
            })?;
            let client = http_client(RAGFLOW_HTTP_TIMEOUT);
            let dataset_id =
                ensure_ragflow_dataset(db, kb_id, &kb_row.name, &kb_row.description, &cfg)
                    .await
                    .map_err(|e| fail_upload(db, &doc.external_doc_id, e).unwrap_err())?;
            let old_id = doc.external_doc_id.trim().to_string();
            let (file_name, file_data) = if is_upload {
                (upload_name, upload_data)
            } else {
                (ragflow_text_file_name(&doc.title), text_content.into_bytes())
            };
            let document_id = ragflow_upload_document(&client, &cfg, &dataset_id, &file_name, &file_data)
                .await
                .map_err(|e| fail_upload(db, &old_id, e).unwrap_err())?;
            ragflow_parse_documents(&client, &cfg, &dataset_id, &[&document_id])
                .await
                .map_err(|e| {
                    let _ = db.update_kb_document_sync_state(
                        kb_id,
                        doc_id,
                        Some(&document_id),
                        SYNC_STATUS_PARSE_FAILED,
                        Some(&truncate_error(&e)),
                    );
                    e
                })?;
            if !old_id.is_empty() && old_id != document_id {
                let _ = ragflow_delete_document(&client, &cfg, &dataset_id, &old_id).await;
            }
            db.update_kb_document_sync_state(
                kb_id,
                doc_id,
                Some(&document_id),
                SYNC_STATUS_SYNCED,
                Some(""),
            )
            .map_err(|e| e.to_string())?;
        }
        "weknora" => {
            if !is_upload && text_content.is_empty() {
                return fail_upload(db, &doc.external_doc_id, "文档内容为空，无法同步".to_string());
            }
            let cfg = parse_weknora_config(&provider_data).map_err(|e| {
                fail_upload(db, &doc.external_doc_id, e).unwrap_err()
            })?;
            let client = http_client(if is_upload {
                WEKNORA_FILE_TIMEOUT
            } else {
                WEKNORA_HTTP_TIMEOUT
            });
            let dataset_id =
                ensure_weknora_dataset(db, kb_id, &kb_row.name, &kb_row.description, &cfg)
                    .await
                    .map_err(|e| fail_upload(db, &doc.external_doc_id, e).unwrap_err())?;
            let old_id = doc.external_doc_id.trim().to_string();
            let (file_name, file_data) = if is_upload {
                (upload_name, upload_data)
            } else {
                (weknora_text_file_name(&doc.title), text_content.into_bytes())
            };
            let document_id =
                weknora_create_knowledge_by_file(&client, &cfg, &dataset_id, &file_name, &file_data)
                    .await
                    .map_err(|e| fail_upload(db, &old_id, e).unwrap_err())?;
            weknora_wait_parsed(&client, &cfg, &document_id)
                .await
                .map_err(|e| {
                    let _ = db.update_kb_document_sync_state(
                        kb_id,
                        doc_id,
                        Some(&document_id),
                        SYNC_STATUS_PARSE_FAILED,
                        Some(&truncate_error(&e)),
                    );
                    e
                })?;
            if !old_id.is_empty() && old_id != document_id {
                let _ = weknora_delete_knowledge(&client, &cfg, &old_id).await;
            }
            db.update_kb_document_sync_state(
                kb_id,
                doc_id,
                Some(&document_id),
                SYNC_STATUS_SYNCED,
                Some(""),
            )
            .map_err(|e| e.to_string())?;
            let _ = db.merge_kb_config_json(
                kb_id,
                &json!({
                    "external_doc_id": document_id,
                    "sync_status": SYNC_STATUS_SYNCED,
                    "sync_error": "",
                    "last_synced_at": chrono::Utc::now().to_rfc3339(),
                }),
            );
        }
        other => {
            return fail_upload(
                db,
                &doc.external_doc_id,
                format!("知识库文档同步暂不支持 provider: {other}"),
            );
        }
    }
    Ok(())
}

#[derive(Clone)]
struct DifyConfig {
    base_url: String,
    api_key: String,
    dataset_permission: String,
    dataset_provider: String,
    indexing_technique: String,
}

#[derive(Clone)]
struct RagflowConfig {
    base_url: String,
    api_key: String,
    dataset_permission: String,
    chunk_method: String,
}

#[derive(Clone)]
struct WeknoraConfig {
    base_url: String,
    api_key: String,
    chunk_size: i64,
    chunk_overlap: i64,
    separators: Vec<String>,
    enable_multimodal: bool,
    embedding_model_id: String,
    summary_model_id: String,
    rerank_model_id: String,
    vlm_model_id: String,
    parse_poll: Duration,
    parse_timeout: Duration,
}

fn load_provider_config(db: &Database, provider: &str) -> Result<(String, Value), String> {
    let row = db
        .find_knowledge_search_config(provider)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("未找到已启用的知识库 provider 配置: {provider}"))?;
    let mut data: Value = serde_json::from_str(&row.json_data).unwrap_or(json!({}));
    flatten_config(&mut data);
    Ok((provider.to_string(), data))
}

fn flatten_config(cfg: &mut Value) {
    if let Value::Object(ref mut map) = cfg {
        if let Some(Value::Object(inner)) = map.remove("config") {
            for (k, v) in inner {
                map.entry(k).or_insert(v);
            }
        }
    }
}

fn parse_dify_config(data: &Value) -> Result<DifyConfig, String> {
    let base_url = str_field(data, "base_url")?;
    let api_key = str_field(data, "api_key")?;
    Ok(DifyConfig {
        base_url,
        api_key,
        dataset_permission: optional_str(data, "dataset_permission").unwrap_or_else(|| "only_me".into()),
        dataset_provider: optional_str(data, "dataset_provider").unwrap_or_else(|| "vendor".into()),
        indexing_technique: optional_str(data, "dataset_indexing_technique")
            .unwrap_or_else(|| "high_quality".into()),
    })
}

fn parse_ragflow_config(data: &Value) -> Result<RagflowConfig, String> {
    Ok(RagflowConfig {
        base_url: str_field(data, "base_url")?,
        api_key: str_field(data, "api_key")?,
        dataset_permission: optional_str(data, "dataset_permission").unwrap_or_else(|| "me".into()),
        chunk_method: optional_str(data, "dataset_chunk_method").unwrap_or_else(|| "naive".into()),
    })
}

fn parse_weknora_config(data: &Value) -> Result<WeknoraConfig, String> {
    let embedding_model_id = str_field(data, "embedding_model_id")?;
    let chunk_size = int_field(data, "chunk_size").unwrap_or(1000);
    let mut chunk_overlap = int_field(data, "chunk_overlap").unwrap_or(200);
    if chunk_overlap >= chunk_size {
        chunk_overlap = chunk_size / 2;
    }
    let separators = data
        .get("separators")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| {
            vec![
                "\n\n".into(),
                "\n".into(),
                "。".into(),
                "！".into(),
                "？".into(),
                ";".into(),
                "；".into(),
            ]
        });
    Ok(WeknoraConfig {
        base_url: str_field(data, "base_url")?,
        api_key: str_field(data, "api_key")?,
        chunk_size,
        chunk_overlap,
        separators,
        enable_multimodal: bool_field(data, "enable_multimodal", true),
        embedding_model_id,
        summary_model_id: optional_str(data, "summary_model_id").unwrap_or_default(),
        rerank_model_id: optional_str(data, "rerank_model_id").unwrap_or_default(),
        vlm_model_id: optional_str(data, "vlm_model_id").unwrap_or_default(),
        parse_poll: Duration::from_millis(
            int_field(data, "parse_poll_interval_ms").unwrap_or(WEKNORA_PARSE_POLL.as_millis() as i64)
                as u64,
        ),
        parse_timeout: Duration::from_millis(
            int_field(data, "parse_timeout_ms").unwrap_or(WEKNORA_PARSE_TIMEOUT.as_millis() as i64)
                as u64,
        ),
    })
}

fn str_field(v: &Value, key: &str) -> Result<String, String> {
    let s = v
        .get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if s.is_empty() {
        return Err(format!("{key} 不能为空"));
    }
    Ok(s)
}

fn optional_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn int_field(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| match x {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

fn bool_field(v: &Value, key: &str, default: bool) -> bool {
    v.get(key)
        .and_then(|x| x.as_bool().or_else(|| x.as_str().map(|s| s == "true" || s == "1")))
        .unwrap_or(default)
}

fn http_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
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

fn build_weknora_url(base: &str, path: &str) -> String {
    build_ragflow_url(base, path)
}

fn build_auto_dataset_name(kb_id: i64, name: &str) -> String {
    let mut n = name.trim().replace(['\n', '\r'], " ");
    if n.is_empty() {
        n = "knowledge-base".to_string();
    }
    let ret = format!("kb-{kb_id}-{n}");
    ret.chars().take(100).collect()
}

fn extract_id(body: &Value) -> Option<String> {
    body.get("id")
        .or_else(|| body.pointer("/data/id"))
        .or_else(|| body.pointer("/document/id"))
        .or_else(|| body.pointer("/data/document/id"))
        .or_else(|| body.pointer("/data/document_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn dify_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    api_key: &str,
    payload: Option<Value>,
) -> Result<Value, String> {
    let mut req = client
        .request(method, url)
        .header("Authorization", format!("Bearer {api_key}"));
    if let Some(body) = payload {
        req = req.json(&body);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("status={status} body={text}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text })))
}

async fn dify_multipart_file(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    api_key: &str,
    fields: Vec<(&str, String)>,
    file_name: &str,
    file_data: &[u8],
) -> Result<Value, String> {
    if file_data.len() > KNOWLEDGE_DOCUMENT_UPLOAD_MAX_BYTES {
        return Err("文件过大".to_string());
    }
    let mut form = Form::new();
    for (k, v) in fields {
        form = form.text(k.to_string(), v);
    }
    let part = Part::bytes(file_data.to_vec())
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    form = form.part("file", part);
    let resp = client
        .request(method, url)
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("status={status} body={text}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text })))
}

async fn ensure_dify_dataset(
    db: &Database,
    kb_id: i64,
    name: &str,
    description: &str,
    cfg: &DifyConfig,
) -> Result<String, String> {
    let detail = db
        .get_knowledge_base(kb_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "知识库不存在".to_string())?;
    let existing = external_kb_id_from_json(&detail.config_json, kb_id);
    if !existing.is_empty() && existing != kb_id.to_string() {
        return Ok(existing);
    }
    let client = http_client(DIFY_HTTP_TIMEOUT);
    let payload = json!({
        "name": build_auto_dataset_name(kb_id, name),
        "description": description.trim(),
        "permission": cfg.dataset_permission,
        "provider": cfg.dataset_provider,
        "indexing_technique": cfg.indexing_technique,
    });
    let body = dify_json(
        &client,
        reqwest::Method::POST,
        &build_dify_url(&cfg.base_url, "/datasets"),
        &cfg.api_key,
        Some(payload),
    )
    .await?;
    let dataset_id = extract_id(&body).ok_or_else(|| format!("创建 Dify dataset 失败: {body}"))?;
    db.merge_kb_config_json(
        kb_id,
        &json!({
            "external_kb_id": dataset_id,
            "auto_dataset": true,
            "sync_provider": "dify",
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(dataset_id)
}

async fn dify_create_document_by_file(
    client: &reqwest::Client,
    cfg: &DifyConfig,
    dataset_id: &str,
    file_name: &str,
    file_data: &[u8],
) -> Result<String, String> {
    let meta = json!({
        "name": sanitize_upload_file_name(file_name),
        "process_rule": { "mode": "automatic", "rules": {} },
        "indexing_technique": cfg.indexing_technique,
    });
    let url = build_dify_url(
        &cfg.base_url,
        &format!("/datasets/{}/document/create-by-file", encode(dataset_id)),
    );
    let mut last_err = String::new();
    for attempt in 1..=DIFY_FILE_UPLOAD_MAX_ATTEMPTS {
        match dify_multipart_file(
            client,
            reqwest::Method::POST,
            &url,
            &cfg.api_key,
            vec![("data", meta.to_string())],
            file_name,
            file_data,
        )
        .await
        {
            Ok(body) => {
                return extract_id(&body)
                    .ok_or_else(|| format!("创建 Dify 文件文档失败: {body}"));
            }
            Err(e) => {
                last_err = e.clone();
                if attempt == DIFY_FILE_UPLOAD_MAX_ATTEMPTS || !should_retry_dify_request(&e) {
                    return Err(format!("创建 Dify 文件文档失败(dataset_id={dataset_id}): {e}"));
                }
                tracing::warn!(
                    dataset_id,
                    attempt,
                    max = DIFY_FILE_UPLOAD_MAX_ATTEMPTS,
                    "Dify create-by-file 重试: {e}"
                );
                tokio::time::sleep(DIFY_FILE_UPLOAD_RETRY_STEP * attempt).await;
            }
        }
    }
    Err(format!(
        "创建 Dify 文件文档失败(dataset_id={dataset_id}): {last_err}"
    ))
}

fn should_retry_dify_request(err: &str) -> bool {
    let msg = err.to_lowercase();
    [
        "context deadline exceeded",
        "client.timeout exceeded",
        "timeout",
        "connection reset",
        "broken pipe",
        "unexpected eof",
        "tls handshake timeout",
        "server closed idle connection",
        "no such host",
        "status=408",
        "status=429",
        "status=500",
        "status=502",
        "status=503",
        "status=504",
    ]
    .iter()
    .any(|needle| msg.contains(needle))
}

async fn dify_replace_document_by_file(
    client: &reqwest::Client,
    cfg: &DifyConfig,
    dataset_id: &str,
    old_document_id: &str,
    file_name: &str,
    file_data: &[u8],
) -> Result<String, String> {
    let new_id =
        dify_create_document_by_file(client, cfg, dataset_id, file_name, file_data).await?;
    if !old_document_id.trim().is_empty() && old_document_id != new_id {
        let _ = dify_delete_document(client, cfg, dataset_id, old_document_id).await;
    }
    Ok(new_id)
}

async fn dify_create_document_by_text(
    client: &reqwest::Client,
    cfg: &DifyConfig,
    dataset_id: &str,
    title: &str,
    content: &str,
) -> Result<String, String> {
    let payload = json!({
        "name": title.trim(),
        "text": content,
        "indexing_technique": cfg.indexing_technique,
    });
    let url = build_dify_url(
        &cfg.base_url,
        &format!("/datasets/{}/document/create-by-text", encode(dataset_id)),
    );
    let body = dify_json(
        client,
        reqwest::Method::POST,
        &url,
        &cfg.api_key,
        Some(payload),
    )
    .await?;
    extract_id(&body).ok_or_else(|| format!("创建 Dify 文本文档失败: {body}"))
}

async fn dify_update_document_by_text(
    client: &reqwest::Client,
    cfg: &DifyConfig,
    dataset_id: &str,
    document_id: &str,
    title: &str,
    content: &str,
) -> Result<(), String> {
    let payload = json!({
        "name": title.trim(),
        "text": content,
        "indexing_technique": cfg.indexing_technique,
    });
    let url = build_dify_url(
        &cfg.base_url,
        &format!(
            "/datasets/{}/documents/{}/update-by-text",
            encode(dataset_id),
            encode(document_id)
        ),
    );
    dify_json(
        client,
        reqwest::Method::POST,
        &url,
        &cfg.api_key,
        Some(payload),
    )
    .await?;
    Ok(())
}

async fn dify_delete_document(
    client: &reqwest::Client,
    cfg: &DifyConfig,
    dataset_id: &str,
    document_id: &str,
) -> Result<(), String> {
    let url = build_dify_url(
        &cfg.base_url,
        &format!(
            "/datasets/{}/documents/{}",
            encode(dataset_id),
            encode(document_id)
        ),
    );
    dify_json(client, reqwest::Method::DELETE, &url, &cfg.api_key, None).await?;
    Ok(())
}

async fn ensure_ragflow_dataset(
    db: &Database,
    kb_id: i64,
    name: &str,
    description: &str,
    cfg: &RagflowConfig,
) -> Result<String, String> {
    let detail = db
        .get_knowledge_base(kb_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "知识库不存在".to_string())?;
    let existing = external_kb_id_from_json(&detail.config_json, kb_id);
    if !existing.is_empty() && existing != kb_id.to_string() {
        return Ok(existing);
    }
    let client = http_client(RAGFLOW_HTTP_TIMEOUT);
    let payload = json!({
        "name": build_auto_dataset_name(kb_id, name),
        "description": description.trim(),
        "permission": cfg.dataset_permission,
        "chunk_method": cfg.chunk_method,
    });
    let body = ragflow_json(
        &client,
        reqwest::Method::POST,
        &build_ragflow_url(&cfg.base_url, "/datasets"),
        &cfg.api_key,
        Some(payload),
    )
    .await?;
    let dataset_id = extract_id(&body).ok_or_else(|| format!("创建 RAGFlow dataset 失败: {body}"))?;
    db.merge_kb_config_json(
        kb_id,
        &json!({
            "external_kb_id": dataset_id,
            "auto_dataset": true,
            "sync_provider": "ragflow",
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(dataset_id)
}

async fn ragflow_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    api_key: &str,
    payload: Option<Value>,
) -> Result<Value, String> {
    let mut req = client
        .request(method, url)
        .header("Authorization", format!("Bearer {api_key}"));
    if let Some(body) = payload {
        req = req.json(&body);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("status={status} body={text}"));
    }
    let body: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    if let Some(code) = body.get("code").and_then(|c| c.as_i64()) {
        if code != 0 {
            return Err(format!("code={code} body={text}"));
        }
    }
    Ok(body)
}

async fn ragflow_multipart_file(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    file_name: &str,
    file_data: &[u8],
) -> Result<Value, String> {
    let part = Part::bytes(file_data.to_vec())
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let form = Form::new().part("file", part);
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("status={status} body={text}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text })))
}

async fn ragflow_upload_document(
    client: &reqwest::Client,
    cfg: &RagflowConfig,
    dataset_id: &str,
    file_name: &str,
    file_data: &[u8],
) -> Result<String, String> {
    let url = build_ragflow_url(
        &cfg.base_url,
        &format!("/datasets/{}/documents", encode(dataset_id)),
    );
    let body = ragflow_multipart_file(client, &url, &cfg.api_key, file_name, file_data).await?;
    if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
        if let Some(first) = arr.first() {
            if let Some(id) = extract_id(first) {
                return Ok(id);
            }
        }
    }
    extract_id(&body).ok_or_else(|| format!("上传 RAGFlow 文档失败: {body}"))
}

async fn ragflow_parse_documents(
    client: &reqwest::Client,
    cfg: &RagflowConfig,
    dataset_id: &str,
    document_ids: &[&str],
) -> Result<(), String> {
    let ids: Vec<&str> = document_ids.iter().copied().filter(|s| !s.is_empty()).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let url = build_ragflow_url(
        &cfg.base_url,
        &format!("/datasets/{}/chunks", encode(dataset_id)),
    );
    ragflow_json(
        client,
        reqwest::Method::POST,
        &url,
        &cfg.api_key,
        Some(json!({ "document_ids": ids })),
    )
    .await?;
    Ok(())
}

async fn ragflow_delete_document(
    client: &reqwest::Client,
    cfg: &RagflowConfig,
    dataset_id: &str,
    document_id: &str,
) -> Result<(), String> {
    let url = build_ragflow_url(
        &cfg.base_url,
        &format!("/datasets/{}/documents", encode(dataset_id)),
    );
    let _ = ragflow_json(
        client,
        reqwest::Method::DELETE,
        &url,
        &cfg.api_key,
        Some(json!({ "ids": [document_id] })),
    )
    .await;
    Ok(())
}

fn ragflow_text_file_name(name: &str) -> String {
    let mut file_name = sanitize_upload_file_name(name);
    if file_name.is_empty() {
        file_name = "document".to_string();
    }
    if !file_name.contains('.') {
        file_name.push_str(".txt");
    }
    file_name
}

async fn ensure_weknora_dataset(
    db: &Database,
    kb_id: i64,
    name: &str,
    description: &str,
    cfg: &WeknoraConfig,
) -> Result<String, String> {
    let detail = db
        .get_knowledge_base(kb_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "知识库不存在".to_string())?;
    let existing = external_kb_id_from_json(&detail.config_json, kb_id);
    if !existing.is_empty() && existing != kb_id.to_string() {
        let client = http_client(WEKNORA_HTTP_TIMEOUT);
        let _ = weknora_update_kb(&client, cfg, &existing, name, description, kb_id).await;
        return Ok(existing);
    }
    let client = http_client(WEKNORA_HTTP_TIMEOUT);
    let payload = weknora_kb_payload(cfg, kb_id, name, description);
    let body = weknora_json(
        &client,
        reqwest::Method::POST,
        &build_weknora_url(&cfg.base_url, "/knowledge-bases"),
        &cfg.api_key,
        Some(payload),
    )
    .await?;
    let dataset_id = extract_id(&body).ok_or_else(|| format!("创建 WeKnora 知识库失败: {body}"))?;
    db.merge_kb_config_json(
        kb_id,
        &json!({
            "external_kb_id": dataset_id,
            "auto_dataset": true,
            "sync_provider": "weknora",
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(dataset_id)
}

fn weknora_kb_payload(cfg: &WeknoraConfig, kb_id: i64, name: &str, description: &str) -> Value {
    let mut payload = json!({
        "name": build_auto_dataset_name(kb_id, name),
        "description": description.trim(),
        "embedding_model_id": cfg.embedding_model_id,
        "chunking_config": {
            "chunk_size": cfg.chunk_size,
            "chunk_overlap": cfg.chunk_overlap,
            "separators": cfg.separators,
            "enable_multimodal": cfg.enable_multimodal,
        },
        "image_processing_config": { "model_id": cfg.vlm_model_id },
    });
    if !cfg.summary_model_id.is_empty() {
        payload["summary_model_id"] = json!(cfg.summary_model_id);
    }
    if !cfg.rerank_model_id.is_empty() {
        payload["rerank_model_id"] = json!(cfg.rerank_model_id);
    }
    if !cfg.vlm_model_id.is_empty() {
        payload["vlm_config"] = json!({ "enabled": true, "model_id": cfg.vlm_model_id });
    }
    payload
}

async fn weknora_update_kb(
    client: &reqwest::Client,
    cfg: &WeknoraConfig,
    kb_external_id: &str,
    name: &str,
    description: &str,
    local_id: i64,
) -> Result<(), String> {
    let payload = json!({
        "name": build_auto_dataset_name(local_id, name),
        "description": description.trim(),
        "config": weknora_kb_payload(cfg, local_id, name, description).get("chunking_config"),
    });
    let url = build_weknora_url(
        &cfg.base_url,
        &format!("/knowledge-bases/{}", encode(kb_external_id)),
    );
    let _ = weknora_json(
        client,
        reqwest::Method::PUT,
        &url,
        &cfg.api_key,
        Some(payload),
    )
    .await;
    Ok(())
}

async fn weknora_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    api_key: &str,
    payload: Option<Value>,
) -> Result<Value, String> {
    let mut req = client.request(method, url).header("X-API-Key", api_key);
    if let Some(body) = payload {
        req = req.json(&body);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("status={status} body={text}"));
    }
    let body: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return Err(format!("success=false body={text}"));
    }
    if let Some(code) = body.get("code").and_then(|c| c.as_i64()) {
        if code != 0 {
            return Err(format!("code={code} body={text}"));
        }
    }
    Ok(body)
}

async fn weknora_multipart_file(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    enable_multimodal: bool,
    file_name: &str,
    file_data: &[u8],
) -> Result<Value, String> {
    let part = Part::bytes(file_data.to_vec())
        .file_name(file_name.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    let form = Form::new()
        .text("enable_multimodel", enable_multimodal.to_string())
        .part("file", part);
    let resp = client
        .post(url)
        .header("X-API-Key", api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("status={status} body={text}"));
    }
    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text })))
}

async fn weknora_create_knowledge_by_file(
    client: &reqwest::Client,
    cfg: &WeknoraConfig,
    kb_external_id: &str,
    file_name: &str,
    file_data: &[u8],
) -> Result<String, String> {
    let url = build_weknora_url(
        &cfg.base_url,
        &format!(
            "/knowledge-bases/{}/knowledge/file",
            encode(kb_external_id)
        ),
    );
    let body = weknora_multipart_file(
        client,
        &url,
        &cfg.api_key,
        cfg.enable_multimodal,
        file_name,
        file_data,
    )
    .await?;
    extract_id(&body).ok_or_else(|| format!("创建 WeKnora 文档失败: {body}"))
}

async fn weknora_delete_knowledge(
    client: &reqwest::Client,
    cfg: &WeknoraConfig,
    knowledge_id: &str,
) -> Result<(), String> {
    let url = build_weknora_url(
        &cfg.base_url,
        &format!("/knowledge/{}", encode(knowledge_id)),
    );
    let _ = weknora_json(client, reqwest::Method::DELETE, &url, &cfg.api_key, None).await;
    Ok(())
}

async fn weknora_wait_parsed(
    client: &reqwest::Client,
    cfg: &WeknoraConfig,
    knowledge_id: &str,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + cfg.parse_timeout;
    loop {
        let url = build_weknora_url(
            &cfg.base_url,
            &format!("/knowledge/{}", encode(knowledge_id)),
        );
        let body = weknora_json(client, reqwest::Method::GET, &url, &cfg.api_key, None).await?;
        let status = body
            .pointer("/data/parse_status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        match status.as_str() {
            "completed" => return Ok(()),
            "failed" => {
                let msg = body
                    .pointer("/data/error_message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                return Err(format!("Weknora 文档解析失败: {msg}"));
            }
            "pending" | "processing" | "" => {
                if std::time::Instant::now() >= deadline {
                    return Err(format!("等待 WeKnora 文档解析超时(knowledge_id={knowledge_id})"));
                }
                tokio::time::sleep(cfg.parse_poll).await;
            }
            _ => {
                if std::time::Instant::now() >= deadline {
                    return Err(format!(
                        "等待 WeKnora 文档解析超时(knowledge_id={knowledge_id} status={status})"
                    ));
                }
                tokio::time::sleep(cfg.parse_poll).await;
            }
        }
    }
}

fn weknora_text_file_name(name: &str) -> String {
    let mut file_name = sanitize_upload_file_name(name);
    if file_name.is_empty() {
        file_name = "document.md".to_string();
    }
    if !file_name.contains('.') {
        file_name.push_str(".md");
    }
    file_name
}

fn persist_kb_sync_success(db: &Database, kb_id: i64) -> Result<(), String> {
    db.merge_kb_config_json(
        kb_id,
        &json!({
            "sync_status": SYNC_STATUS_SYNCED,
            "sync_error": "",
            "last_synced_at": chrono::Utc::now().to_rfc3339(),
        }),
    )
    .map_err(|e| e.to_string())
}

fn persist_kb_sync_failure(db: &Database, kb_id: i64, err: &str) -> Result<(), String> {
    db.merge_kb_config_json(
        kb_id,
        &json!({
            "sync_status": SYNC_STATUS_FAILED,
            "sync_error": truncate_error(err),
        }),
    )
    .map_err(|e| e.to_string())
}

fn auto_dataset_from_config(config_json: &str) -> bool {
    serde_json::from_str::<Value>(config_json)
        .ok()
        .and_then(|v| v.get("auto_dataset").and_then(|x| x.as_bool()))
        .unwrap_or(false)
}

fn kb_external_doc_id_from_config(config_json: &str) -> String {
    serde_json::from_str::<Value>(config_json)
        .ok()
        .and_then(|v| {
            v.get("external_doc_id")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

fn is_external_provider(provider: &str) -> bool {
    matches!(
        provider.trim().to_lowercase().as_str(),
        "dify" | "ragflow" | "weknora"
    )
}

pub async fn sync_delete_knowledge_base(db: &Database, snapshot: &KbSyncSnapshot) -> Result<(), String> {
    if !is_external_provider(&snapshot.provider) {
        return Ok(());
    }
    let dataset_id = external_kb_id_from_json(&snapshot.config_json, 0);
    if dataset_id.is_empty() || dataset_id == "0" {
        return Ok(());
    }
    let (_, provider_data) = load_provider_config(db, &snapshot.provider)?;
    let kb_doc_id = kb_external_doc_id_from_config(&snapshot.config_json);
    let auto_dataset = auto_dataset_from_config(&snapshot.config_json);

    match snapshot.provider.as_str() {
        "dify" => {
            let cfg = parse_dify_config(&provider_data)?;
            let client = http_client(DIFY_HTTP_TIMEOUT);
            if !kb_doc_id.is_empty() {
                let _ = dify_delete_document(&client, &cfg, &dataset_id, &kb_doc_id).await;
            }
            if auto_dataset && dify_dataset_is_empty(&client, &cfg, &dataset_id).await? {
                dify_delete_dataset(&client, &cfg, &dataset_id).await?;
            }
        }
        "ragflow" => {
            let cfg = parse_ragflow_config(&provider_data)?;
            let client = http_client(RAGFLOW_HTTP_TIMEOUT);
            if auto_dataset && ragflow_dataset_is_empty(&client, &cfg, &dataset_id).await? {
                ragflow_delete_dataset(&client, &cfg, &dataset_id).await?;
            }
        }
        "weknora" => {
            let cfg = parse_weknora_config(&provider_data)?;
            let client = http_client(WEKNORA_HTTP_TIMEOUT);
            if auto_dataset && weknora_kb_is_empty(&client, &cfg, &dataset_id).await? {
                weknora_delete_kb(&client, &cfg, &dataset_id).await?;
            }
        }
        other => return Err(format!("知识库删除同步暂不支持 provider: {other}")),
    }
    Ok(())
}

pub async fn sync_delete_document(
    db: &Database,
    kb: &KbSyncSnapshot,
    doc: &KbDocumentDeleteSnapshot,
) -> Result<(), String> {
    if !is_external_provider(&kb.provider) {
        return Ok(());
    }
    let dataset_id = external_kb_id_from_json(&kb.config_json, 0);
    if dataset_id.is_empty() || dataset_id == "0" {
        return Ok(());
    }
    let doc_id = doc.external_doc_id.trim();
    let (_, provider_data) = load_provider_config(db, &kb.provider)?;
    let auto_dataset = auto_dataset_from_config(&kb.config_json);

    match kb.provider.as_str() {
        "dify" => {
            let cfg = parse_dify_config(&provider_data)?;
            let client = http_client(DIFY_HTTP_TIMEOUT);
            if !doc_id.is_empty() {
                dify_delete_document(&client, &cfg, &dataset_id, doc_id).await?;
            }
            if auto_dataset && dify_dataset_is_empty(&client, &cfg, &dataset_id).await? {
                dify_delete_dataset(&client, &cfg, &dataset_id).await?;
            }
        }
        "ragflow" => {
            let cfg = parse_ragflow_config(&provider_data)?;
            let client = http_client(RAGFLOW_HTTP_TIMEOUT);
            if !doc_id.is_empty() {
                ragflow_delete_document(&client, &cfg, &dataset_id, doc_id).await?;
            }
            if auto_dataset && ragflow_dataset_is_empty(&client, &cfg, &dataset_id).await? {
                ragflow_delete_dataset(&client, &cfg, &dataset_id).await?;
            }
        }
        "weknora" => {
            let cfg = parse_weknora_config(&provider_data)?;
            let client = http_client(WEKNORA_HTTP_TIMEOUT);
            if !doc_id.is_empty() {
                weknora_delete_knowledge(&client, &cfg, doc_id).await?;
            }
            if auto_dataset && weknora_kb_is_empty(&client, &cfg, &dataset_id).await? {
                weknora_delete_kb(&client, &cfg, &dataset_id).await?;
            }
        }
        other => return Err(format!("知识库文档删除同步暂不支持 provider: {other}")),
    }
    Ok(())
}

async fn dify_dataset_is_empty(
    client: &reqwest::Client,
    cfg: &DifyConfig,
    dataset_id: &str,
) -> Result<bool, String> {
    let url = build_dify_url(
        &cfg.base_url,
        &format!(
            "/datasets/{}/documents?page=1&limit=1",
            encode(dataset_id)
        ),
    );
    let body = dify_json(client, reqwest::Method::GET, &url, &cfg.api_key, None).await?;
    if body.get("total").and_then(|v| v.as_i64()).unwrap_or(0) > 0 {
        return Ok(false);
    }
    if body
        .get("data")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
    {
        return Ok(false);
    }
    Ok(true)
}

async fn dify_delete_dataset(
    client: &reqwest::Client,
    cfg: &DifyConfig,
    dataset_id: &str,
) -> Result<(), String> {
    let url = build_dify_url(
        &cfg.base_url,
        &format!("/datasets/{}", encode(dataset_id)),
    );
    let _ = dify_json(client, reqwest::Method::DELETE, &url, &cfg.api_key, None).await;
    Ok(())
}

async fn ragflow_dataset_is_empty(
    client: &reqwest::Client,
    cfg: &RagflowConfig,
    dataset_id: &str,
) -> Result<bool, String> {
    let url = build_ragflow_url(
        &cfg.base_url,
        &format!(
            "/datasets/{}/documents?page=1&page_size=1",
            encode(dataset_id)
        ),
    );
    let body = ragflow_json(client, reqwest::Method::GET, &url, &cfg.api_key, None).await?;
    if body.get("total").and_then(|v| v.as_i64()).unwrap_or(0) > 0 {
        return Ok(false);
    }
    if body
        .get("data")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
    {
        return Ok(false);
    }
    Ok(true)
}

async fn ragflow_delete_dataset(
    client: &reqwest::Client,
    cfg: &RagflowConfig,
    dataset_id: &str,
) -> Result<(), String> {
    let url = build_ragflow_url(&cfg.base_url, "/datasets");
    let _ = ragflow_json(
        client,
        reqwest::Method::DELETE,
        &url,
        &cfg.api_key,
        Some(json!({ "ids": [dataset_id] })),
    )
    .await;
    Ok(())
}

async fn weknora_kb_is_empty(
    client: &reqwest::Client,
    cfg: &WeknoraConfig,
    kb_id: &str,
) -> Result<bool, String> {
    let url = build_weknora_url(
        &cfg.base_url,
        &format!(
            "/knowledge-bases/{}/knowledge?page=1&page_size=1",
            encode(kb_id)
        ),
    );
    let body = weknora_json(client, reqwest::Method::GET, &url, &cfg.api_key, None).await?;
    if body
        .pointer("/data/total")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        > 0
    {
        return Ok(false);
    }
    if body
        .pointer("/data/list")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
    {
        return Ok(false);
    }
    Ok(true)
}

async fn weknora_delete_kb(
    client: &reqwest::Client,
    cfg: &WeknoraConfig,
    kb_id: &str,
) -> Result<(), String> {
    let url = build_weknora_url(
        &cfg.base_url,
        &format!("/knowledge-bases/{}", encode(kb_id)),
    );
    let _ = weknora_json(client, reqwest::Method::DELETE, &url, &cfg.api_key, None).await;
    Ok(())
}

fn truncate_error(msg: &str) -> String {
    let msg = msg.trim();
    if msg.len() <= 800 {
        return msg.to_string();
    }
    format!("{}...", &msg[..800])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_provider_urls() {
        assert!(build_dify_url("http://dify", "/datasets").ends_with("/v1/datasets"));
        assert!(build_ragflow_url("http://rag", "/datasets").contains("/api/v1/datasets"));
    }
}
