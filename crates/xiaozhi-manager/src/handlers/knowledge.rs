use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{json_data, json_error, AppState};
use crate::db::KnowledgeBaseListItem;
use crate::extractors::{AdminUser, AuthUser};
use crate::knowledge_global::{build_knowledge_global_config_data, resolve_default_knowledge_provider};
use crate::knowledge_sync;
use crate::knowledge_upload::{
    build_upload_document_name, encode_upload_content, validate_upload_file,
};

#[derive(Deserialize)]
pub struct KnowledgeBaseInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub config_json: Value,
}

fn normalize_config_json(input: &Value) -> String {
    if input.is_null() {
        return "{}".to_string();
    }
    serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string())
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let rows = state.db.list_knowledge_bases(claims.sub).unwrap_or_default();
    let kb_ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let doc_counts = state
        .db
        .count_kb_documents_by_kb_ids(&kb_ids)
        .unwrap_or_default();
    let data: Vec<KnowledgeBaseListItem> = rows
        .into_iter()
        .map(|base| KnowledgeBaseListItem {
            doc_count: doc_counts.get(&base.id).copied().unwrap_or(0),
            base,
        })
        .collect();
    Json(json!({
        "data": data,
        "knowledge": build_knowledge_global_config_data(&state.db),
    }))
}

pub async fn admin_list_for_user(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(user_id): Path<i64>,
) -> Json<Value> {
    let rows = state.db.list_knowledge_bases(user_id).unwrap_or_default();
    json_data(rows)
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(req): Json<KnowledgeBaseInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if req.name.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "知识库名称不能为空"));
    }
    let provider = if req.provider.trim().is_empty() {
        resolve_default_knowledge_provider(&state.db)
    } else {
        req.provider.trim().to_lowercase()
    };
    let status = if req.status.trim().is_empty() {
        "active".to_string()
    } else {
        req.status.trim().to_string()
    };
    let config_json = normalize_config_json(&req.config_json);
    let id = state
        .db
        .create_knowledge_base(
            claims.sub,
            req.name.trim(),
            req.description.trim(),
            &provider,
            &status,
            &config_json,
        )
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if provider != "local" {
        knowledge_sync::spawn_knowledge_base_sync(state.db.clone(), id);
    }
    Ok(json_data(json!({
        "id": id,
        "message": if provider == "local" {
            "创建成功"
        } else {
            "知识库已保存，后台正在同步"
        }
    })))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Json(req): Json<KnowledgeBaseInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if req.name.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "知识库名称不能为空"));
    }
    let existing = state
        .db
        .get_owned_knowledge_base(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;
    let provider = if req.provider.trim().is_empty() {
        existing.provider
    } else {
        req.provider.trim().to_lowercase()
    };
    let status = if req.status.trim().is_empty() {
        existing.status
    } else {
        req.status.trim().to_string()
    };
    let config_json = if req.config_json.is_null() {
        existing.config_json
    } else {
        normalize_config_json(&req.config_json)
    };
    let ok = state
        .db
        .update_knowledge_base(
            id,
            claims.sub,
            req.name.trim(),
            req.description.trim(),
            &provider,
            &status,
            &config_json,
        )
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "知识库不存在"));
    }
    if provider != "local" && !provider.is_empty() {
        let _ = knowledge_sync::mark_kb_sync_pending(&state.db, id);
        knowledge_sync::spawn_knowledge_base_sync(state.db.clone(), id);
        return Ok(json_data(json!({ "message": "更新成功，后台正在同步" })));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kb = state
        .db
        .get_owned_knowledge_base(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;
    let docs = state.db.list_kb_documents(id).unwrap_or_default();
    let snapshot = knowledge_sync::KbSyncSnapshot::from_kb(&kb.provider, &kb.config_json);
    let doc_snapshots: Vec<_> = docs
        .iter()
        .map(|d| knowledge_sync::KbDocumentDeleteSnapshot {
            external_doc_id: d.external_doc_id.clone(),
        })
        .collect();

    let ok = state
        .db
        .delete_knowledge_base(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "知识库不存在"));
    }

    let provider = kb.provider.trim().to_lowercase();
    if provider != "local" && !provider.is_empty() {
        if doc_snapshots.is_empty() {
            knowledge_sync::spawn_knowledge_base_delete_sync(state.db.clone(), snapshot);
        } else {
            for doc_snap in doc_snapshots {
                knowledge_sync::spawn_knowledge_document_delete_sync(
                    state.db.clone(),
                    snapshot.clone(),
                    doc_snap,
                );
            }
        }
        return Ok(json_data(json!({ "message": "删除成功，后台正在清理知识库数据" })));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn sync(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kb = state
        .db
        .get_owned_knowledge_base(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;
    let provider = kb.provider.trim().to_lowercase();
    if provider == "local" || provider.is_empty() {
        return Ok(json_data(json!({
            "id": id,
            "message": "同步任务已提交（本地模式无需同步）"
        })));
    }
    knowledge_sync::mark_kb_sync_pending(&state.db, id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    knowledge_sync::spawn_knowledge_base_sync(state.db.clone(), id);
    Ok(json_data(json!({
        "id": id,
        "message": "知识库已保存，后台正在同步"
    })))
}

#[derive(Deserialize)]
pub struct TestSearchBody {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: i64,
}

fn default_top_k() -> i64 {
    3
}

pub async fn test_search(
    State(state): State<AppState>,
    Path(kb_id): Path<i64>,
    Json(body): Json<TestSearchBody>,
) -> Json<Value> {
    let hits = crate::knowledge_search::search_knowledge_bases(
        &state.db,
        &[kb_id],
        &body.query,
        body.top_k as usize,
        0.2,
    )
    .await;
    let results: Vec<Value> = hits
        .iter()
        .map(|h| {
            json!({
                "title": h.title,
                "content": h.content,
                "score": h.score,
                "document_id": h.document_id,
            })
        })
        .collect();
    json_data(json!({
        "query": body.query,
        "results": results,
    }))
}

pub async fn list_documents(
    State(state): State<AppState>,
    AuthUser(_claims): AuthUser,
    Path(kb_id): Path<i64>,
) -> Json<Value> {
    let docs = state.db.list_kb_documents(kb_id).unwrap_or_default();
    json_data(docs)
}

#[derive(Deserialize)]
pub struct DocumentInput {
    pub title: String,
    #[serde(default)]
    pub content: String,
}

pub async fn create_document(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(kb_id): Path<i64>,
    Json(body): Json<DocumentInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kb = state
        .db
        .get_owned_knowledge_base(kb_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;
    let provider = kb.provider.trim().to_lowercase();
    let (status, message) = if provider == "dify" || provider == "ragflow" || provider == "weknora" {
        ("pending", "文档已保存，后台正在同步")
    } else {
        ("ready", "创建成功")
    };
    let id = state
        .db
        .create_kb_document_with_meta(kb_id, &body.title, &body.content, "manual", status)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if status == "pending" {
        knowledge_sync::spawn_knowledge_document_sync(state.db.clone(), kb_id, id);
    }
    Ok(json_data(json!({ "id": id, "message": message })))
}

pub async fn delete_document(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((kb_id, doc_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kb = state
        .db
        .get_owned_knowledge_base(kb_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;
    let doc = state
        .db
        .get_kb_document(kb_id, doc_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "文档不存在"))?;
    let snapshot = knowledge_sync::KbSyncSnapshot::from_kb(&kb.provider, &kb.config_json);
    let doc_snapshot = knowledge_sync::KbDocumentDeleteSnapshot {
        external_doc_id: doc.external_doc_id.clone(),
    };

    let ok = state
        .db
        .delete_kb_document(kb_id, doc_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "文档不存在"));
    }

    let provider = kb.provider.trim().to_lowercase();
    if provider != "local" && !provider.is_empty() {
        knowledge_sync::spawn_knowledge_document_delete_sync(
            state.db.clone(),
            snapshot,
            doc_snapshot,
        );
        return Ok(json_data(json!({ "message": "删除成功，后台正在清理知识库文档" })));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn update_document(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((kb_id, doc_id)): Path<(i64, i64)>,
    Json(body): Json<DocumentInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kb = state
        .db
        .get_owned_knowledge_base(kb_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;
    let ok = state
        .db
        .update_kb_document(kb_id, doc_id, &body.title, &body.content)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "文档不存在"));
    }
    let provider = kb.provider.trim().to_lowercase();
    if provider == "dify" || provider == "ragflow" || provider == "weknora" {
        let _ = knowledge_sync::mark_document_sync_pending(&state.db, kb_id, doc_id);
        knowledge_sync::spawn_knowledge_document_sync(state.db.clone(), kb_id, doc_id);
        return Ok(json_data(json!({ "message": "更新成功，后台正在同步" })));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn sync_document(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((kb_id, doc_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kb = state
        .db
        .get_owned_knowledge_base(kb_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;
    let provider = kb.provider.trim().to_lowercase();
    if provider == "local" || provider.is_empty() {
        return Ok(json_data(json!({ "kb_id": kb_id, "doc_id": doc_id, "message": "已同步" })));
    }
    if body_requires_content_check(&state.db, kb_id, doc_id) {
        return Err(json_error(StatusCode::BAD_REQUEST, "文档内容为空，无法同步"));
    }
    knowledge_sync::mark_document_sync_pending(&state.db, kb_id, doc_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    knowledge_sync::spawn_knowledge_document_sync(state.db.clone(), kb_id, doc_id);
    Ok(json_data(json!({
        "kb_id": kb_id,
        "doc_id": doc_id,
        "message": "同步任务已提交"
    })))
}

fn body_requires_content_check(db: &crate::db::Database, kb_id: i64, doc_id: i64) -> bool {
    db.get_kb_document(kb_id, doc_id)
        .ok()
        .flatten()
        .is_some_and(|d| d.content.trim().is_empty())
}

pub async fn upload_document(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(kb_id): Path<i64>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kb = state
        .db
        .get_owned_knowledge_base(kb_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "知识库不存在"))?;

    let provider = kb.provider.trim().to_lowercase();
    if provider != "dify" && provider != "ragflow" && provider != "weknora" {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            &format!("当前知识库提供商为 {provider}，暂不支持文件上传创建文档"),
        ));
    }

    let mut display_name = String::new();
    let mut file_name = String::new();
    let mut file_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
    {
        match field.name().unwrap_or("") {
            "name" => display_name = field.text().await.unwrap_or_default(),
            "file" => {
                file_name = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "document.txt".to_string());
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    let data = file_data.ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "请上传文件(file)"))?;
    validate_upload_file(&provider, &file_name, &data)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;
    let content = encode_upload_content(&file_name, &data)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;
    let title = build_upload_document_name(&display_name, &file_name);

    let id = state
        .db
        .create_kb_document_with_meta(kb_id, &title, &content, "upload", "pending")
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    knowledge_sync::spawn_knowledge_document_sync(state.db.clone(), kb_id, id);

    Ok(json_data(json!({
        "id": id,
        "message": "文件上传成功，文档已创建并提交异步同步"
    })))
}
