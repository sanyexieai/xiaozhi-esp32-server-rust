use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::app::{json_data, json_error, json_ok, AppState};
use crate::db::{
    AdminChatMessageQuery, AdminChatSessionQuery, ChatMessageInput, ChatMessageQuery,
    ChatMessageRow, ChatSessionQuery,
};
use crate::extractors::{AdminUser, AuthUser};

fn enrich_chat_message_input(state: &AppState, input: &mut ChatMessageInput) {
    let candidates = [
        input.device_id.as_str(),
        xiaozhi_core::constants::simulator::resolve_physical_device_id(&input.device_id),
    ];
    for device_id in candidates {
        if device_id.is_empty() {
            continue;
        }
        let Ok(Some(device)) = state.db.find_device_by_device_id(device_id) else {
            continue;
        };
        if input.user_id.is_none() {
            input.user_id = device.user_id;
        }
        if input.agent_id.is_none() {
            input.agent_id = device.agent_id;
        }
        break;
    }
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    pub agent_id: Option<i64>,
    pub device_id: Option<String>,
    pub session_id: Option<String>,
    pub role: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    20
}

#[derive(Deserialize)]
pub struct AdminHistoryQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    pub user_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub device_id: Option<String>,
    pub session_id: Option<String>,
    pub role: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

fn query_from_history(q: &HistoryQuery, user_id: i64) -> ChatMessageQuery {
    ChatMessageQuery {
        user_id,
        agent_id: q.agent_id,
        device_id: q.device_id.clone(),
        session_id: q.session_id.clone(),
        role: q.role.clone(),
        start_date: q.start_date.clone(),
        end_date: q.end_date.clone(),
        page: Some(q.page),
        page_size: Some(q.page_size),
    }
}

fn session_query_from_history(q: &HistoryQuery, user_id: i64) -> ChatSessionQuery {
    ChatSessionQuery {
        user_id,
        agent_id: q.agent_id,
        device_id: q.device_id.clone(),
        start_date: q.start_date.clone(),
        end_date: q.end_date.clone(),
        page: Some(q.page),
        page_size: Some(q.page_size),
    }
}

pub async fn list_messages(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Query(q): Query<HistoryQuery>,
) -> Json<Value> {
    let filter = query_from_history(&q, claims.sub);
    let (total, messages) = match state.db.query_chat_messages(filter) {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("查询用户消息列表失败: {e:#}");
            (0, vec![])
        }
    };
    Json(json!({
        "total": total,
        "page": q.page,
        "page_size": q.page_size,
        "data": messages,
    }))
}

fn enrich_chat_messages(state: &AppState, messages: Vec<ChatMessageRow>) -> Vec<Value> {
    let users = state.db.list_users().unwrap_or_default();
    let user_map: std::collections::HashMap<i64, String> =
        users.into_iter().map(|u| (u.id, u.username)).collect();
    let mut agent_names: std::collections::HashMap<i64, String> =
        std::collections::HashMap::new();

    messages
        .into_iter()
        .map(|msg| {
            let mut value = serde_json::to_value(&msg).unwrap_or_else(|_| json!({}));
            if let Some(uid) = msg.user_id {
                if let Some(name) = user_map.get(&uid) {
                    value["username"] = json!(name);
                }
            }
            if let Some(agent_id) = msg.agent_id {
                let agent_name = agent_names.entry(agent_id).or_insert_with(|| {
                    state
                        .db
                        .get_agent_by_id(agent_id)
                        .ok()
                        .flatten()
                        .map(|a| a.name)
                        .unwrap_or_else(|| format!("智能体 #{agent_id}"))
                });
                value["agent_name"] = json!(agent_name);
            }
            value
        })
        .collect()
}

pub async fn admin_list_messages(
    State(state): State<AppState>,
    AdminUser(_claims): AdminUser,
    Query(q): Query<AdminHistoryQuery>,
) -> Json<Value> {
    let filter = AdminChatMessageQuery {
        user_id: q.user_id,
        agent_id: q.agent_id,
        device_id: q.device_id.clone(),
        session_id: q.session_id.clone(),
        role: q.role.clone(),
        start_date: q.start_date.clone(),
        end_date: q.end_date.clone(),
        page: Some(q.page),
        page_size: Some(q.page_size),
    };
    let (total, messages) = state
        .db
        .query_chat_messages_admin(filter)
        .unwrap_or((0, vec![]));
    Json(json!({
        "total": total,
        "page": q.page,
        "page_size": q.page_size,
        "data": enrich_chat_messages(&state, messages),
    }))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Query(q): Query<HistoryQuery>,
) -> Json<Value> {
    let filter = session_query_from_history(&q, claims.sub);
    let (total, sessions) = match state.db.query_chat_sessions(filter) {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("查询用户会话列表失败: {e:#}");
            (0, vec![])
        }
    };
    Json(json!({
        "total": total,
        "page": q.page,
        "page_size": q.page_size,
        "data": sessions,
    }))
}

pub async fn admin_list_sessions(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Query(q): Query<AdminHistoryQuery>,
) -> Json<Value> {
    let filter = AdminChatSessionQuery {
        user_id: q.user_id,
        agent_id: q.agent_id,
        device_id: q.device_id.clone(),
        start_date: q.start_date.clone(),
        end_date: q.end_date.clone(),
        page: Some(q.page),
        page_size: Some(q.page_size),
    };
    let (total, sessions) = state
        .db
        .query_chat_sessions_admin(filter)
        .unwrap_or_else(|e| {
            tracing::error!("查询管理端会话列表失败: {e:#}");
            (0, vec![])
        });
    Json(json!({
        "total": total,
        "page": q.page,
        "page_size": q.page_size,
        "data": sessions,
    }))
}

pub async fn get_session_messages(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(session_id): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Json<Value> {
    let mut filter = query_from_history(&q, claims.sub);
    filter.session_id = Some(session_id);
    let (total, messages) = state
        .db
        .query_chat_messages(filter)
        .unwrap_or((0, vec![]));
    Json(json!({
        "total": total,
        "page": q.page,
        "page_size": q.page_size,
        "data": messages,
    }))
}

pub async fn admin_get_session_messages(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(session_id): Path<String>,
    Query(q): Query<AdminHistoryQuery>,
) -> Json<Value> {
    let filter = AdminChatMessageQuery {
        user_id: q.user_id,
        agent_id: q.agent_id,
        device_id: q.device_id.clone(),
        session_id: Some(session_id),
        role: q.role.clone(),
        start_date: q.start_date.clone(),
        end_date: q.end_date.clone(),
        page: Some(q.page),
        page_size: Some(q.page_size),
    };
    let (total, messages) = state
        .db
        .query_chat_messages_admin(filter)
        .unwrap_or((0, vec![]));
    Json(json!({
        "total": total,
        "page": q.page,
        "page_size": q.page_size,
        "data": enrich_chat_messages(&state, messages),
    }))
}

pub async fn internal_session_dialogue(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if session_id.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "session_id 不能为空"));
    }
    let rows = state
        .db
        .list_session_dialogue(&session_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let messages: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "role": row.role,
                "content": row.content,
            })
        })
        .collect();
    let device_id = rows.first().map(|r| r.device_id.clone()).unwrap_or_default();
    Ok(json_ok(json!({
        "session_id": session_id,
        "device_id": device_id,
        "messages": messages,
    })))
}

pub async fn admin_delete_message(
    State(state): State<AppState>,
    AdminUser(_claims): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_chat_message_admin(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "消息不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn list_agent_messages(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(agent_id): Path<i64>,
    Query(q): Query<HistoryQuery>,
) -> Json<Value> {
    let mut filter = query_from_history(&q, claims.sub);
    filter.agent_id = Some(agent_id);
    let (total, messages) = state
        .db
        .query_chat_messages(filter)
        .unwrap_or((0, vec![]));
    Json(json!({
        "total": total,
        "page": q.page,
        "page_size": q.page_size,
        "data": messages,
    }))
}

pub async fn export_messages(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Query(q): Query<HistoryQuery>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let filter = query_from_history(&q, claims.sub);
    let messages = state
        .db
        .export_chat_messages(filter)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let payload = json!({
        "export_time": chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "total": messages.len(),
        "messages": messages,
    });
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"chat_history.json\"",
        )
        .body(Body::from(bytes))
        .unwrap())
}

pub async fn delete_message(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_chat_message(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "消息不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn get_message_audio(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let msg = state
        .db
        .get_chat_message(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "消息不存在"))?;
    let path = msg
        .audio_path
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "无音频"))?;
    let bytes = fs::read(&path)
        .await
        .map_err(|_| json_error(StatusCode::NOT_FOUND, "音频文件不存在"))?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(Body::from(bytes))
        .unwrap())
}

pub async fn save_message_internal(
    State(state): State<AppState>,
    Json(mut body): Json<ChatMessageInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.message_id.is_empty() {
        body.message_id = uuid::Uuid::new_v4().to_string();
    }
    enrich_chat_message_input(&state, &mut body);
    let id = state
        .db
        .save_chat_message(&body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_ok(json!({ "id": id, "message_id": body.message_id })))
}

#[derive(Deserialize)]
pub struct AudioUpdateBody {
    pub audio_data: String,
    #[serde(default = "default_wav")]
    pub audio_format: String,
}

fn default_wav() -> String {
    "wav".to_string()
}

pub async fn update_message_audio_internal(
    State(state): State<AppState>,
    Path(message_id): Path<String>,
    Json(body): Json<AudioUpdateBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        body.audio_data,
    )
    .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let dir = state.data_dir.join("audio").join("history");
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let path: PathBuf = dir.join(format!("{message_id}.{}", body.audio_format));
    fs::write(&path, &bytes)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    state
        .db
        .update_chat_message_audio(
            &message_id,
            &path.to_string_lossy(),
            &body.audio_format,
            bytes.len() as i64,
        )
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_ok(json!({ "message": "ok" })))
}

// 兼容旧路径
pub async fn save_chat_legacy(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut input = ChatMessageInput {
        message_id: body
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        device_id: body
            .get("device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        agent_id: body.get("agent_id").and_then(|v| v.as_i64()),
        user_id: body.get("user_id").and_then(|v| v.as_i64()),
        session_id: body
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        role: body
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user")
            .to_string(),
        content: body
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        tool_call_id: None,
        tool_calls_json: None,
        metadata: String::new(),
    };
    if input.message_id.is_empty() {
        input.message_id = uuid::Uuid::new_v4().to_string();
    }
    enrich_chat_message_input(&state, &mut input);
    let id = state
        .db
        .save_chat_message(&input)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_ok(json!({ "id": id, "message_id": input.message_id })))
}
