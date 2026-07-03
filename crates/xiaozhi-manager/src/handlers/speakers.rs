use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::app::{json_data, json_error, json_ok, AppState};
use crate::db::SpeakerGroupInput;
use crate::extractors::{AdminUser, AuthUser};
use crate::speaker_client::{load_speaker_service, verify_message, SpeakerClient};

fn find_user_group(
    state: &AppState,
    user_id: i64,
    group_id: i64,
) -> Result<crate::db::SpeakerGroupRow, (StatusCode, Json<Value>)> {
    let groups = state
        .db
        .list_speaker_groups(user_id, None)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    groups
        .into_iter()
        .find(|g| g.id == group_id)
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "声纹组不存在"))
}

#[derive(Deserialize)]
pub struct GroupQuery {
    pub agent_id: Option<i64>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Query(q): Query<GroupQuery>,
) -> Json<Value> {
    list_groups_inner(&state, claims.sub, q.agent_id, q.page, q.page_size).await
}

async fn list_groups_inner(
    state: &AppState,
    user_id: i64,
    agent_id: Option<i64>,
    page: Option<i64>,
    page_size: Option<i64>,
) -> Json<Value> {
    let groups = state
        .db
        .list_speaker_groups(user_id, agent_id)
        .unwrap_or_default();
    let total = groups.len() as i64;
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(10).clamp(1, 200);
    let offset = ((page - 1) * page_size) as usize;
    let agents = state.db.list_agents(user_id).unwrap_or_default();
    let agent_names: std::collections::HashMap<i64, String> =
        agents.into_iter().map(|a| (a.id, a.name)).collect();
    let items: Vec<Value> = groups
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .map(|g| {
            json!({
                "id": g.id,
                "agent_id": g.agent_id,
                "agent_name": agent_names.get(&g.agent_id).cloned().unwrap_or_default(),
                "name": g.name,
                "prompt": g.prompt,
                "description": g.description,
                "tts_config_id": g.tts_config_id,
                "voice": g.voice,
                "sample_count": g.sample_count,
                "created_at": g.created_at,
                "updated_at": g.updated_at,
            })
        })
        .collect();
    json_ok(json!({
        "data": items,
        "total": total,
        "page": page,
        "page_size": page_size,
    }))
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let group = find_user_group(&state, claims.sub, id)?;
    let agent_name = state
        .db
        .get_agent(group.agent_id, claims.sub)
        .ok()
        .flatten()
        .map(|a| a.name)
        .unwrap_or_default();
    let samples = state
        .db
        .list_speaker_samples(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({
        "id": group.id,
        "agent_id": group.agent_id,
        "agent_name": agent_name,
        "name": group.name,
        "prompt": group.prompt,
        "description": group.description,
        "tts_config_id": group.tts_config_id,
        "voice": group.voice,
        "sample_count": group.sample_count,
        "samples": samples,
        "created_at": group.created_at,
        "updated_at": group.updated_at,
    })))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body): Json<SpeakerGroupInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.name.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "名称不能为空"));
    }
    if body.agent_id <= 0 {
        return Err(json_error(StatusCode::BAD_REQUEST, "请选择智能体"));
    }
    state
        .db
        .get_agent(body.agent_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "智能体不存在或无权限访问"))?;
    if state
        .db
        .speaker_group_name_exists(claims.sub, body.name.trim(), None)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "该声纹组名称已存在，请使用其他名称",
        ));
    }
    let id = state
        .db
        .create_speaker_group(claims.sub, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "success": true, "data": { "id": id } })))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<SpeakerGroupInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let existing = find_user_group(&state, claims.sub, id)?;
    if !body.name.trim().is_empty() && body.name.trim() != existing.name {
        if state
            .db
            .speaker_group_name_exists(claims.sub, body.name.trim(), Some(id))
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        {
            return Err(json_error(
                StatusCode::BAD_REQUEST,
                "该声纹组名称已存在，请使用其他名称",
            ));
        }
    }
    if body.agent_id != existing.agent_id {
        state
            .db
            .get_agent(body.agent_id, claims.sub)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
            .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "智能体不存在或无权限访问"))?;
    }
    let ok = state
        .db
        .update_speaker_group(id, claims.sub, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "声纹组不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let group = find_user_group(&state, claims.sub, id)?;
    let samples = state
        .db
        .list_speaker_samples(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    if let Ok(Some(svc)) = load_speaker_service(&state.db) {
        if svc.enabled && !svc.base_url.is_empty() {
            let client = SpeakerClient::new(svc);
            if let Err(e) = client
                .delete_group(&id.to_string(), group.agent_id, claims.sub)
                .await
            {
                tracing::warn!("asr_server 删除声纹组失败 (speaker_id: {id}): {e}");
            }
        }
    }

    for sample in &samples {
        let _ = fs::remove_file(&sample.file_path).await;
    }

    let ok = state
        .db
        .delete_speaker_group(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "声纹组不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn list_samples(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(group_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    find_user_group(&state, claims.sub, group_id)?;
    let samples = state
        .db
        .list_speaker_samples(group_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(samples))
}

pub async fn add_sample(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(group_id): Path<i64>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let group = find_user_group(&state, claims.sub, group_id)?;
    let mut audio_bytes = Vec::new();
    let mut audio_name = "sample.wav".to_string();
    let mut message_id: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
    {
        if field.name() == Some("message_id") {
            message_id = Some(
                field
                    .text()
                    .await
                    .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?,
            );
        } else if field.name() == Some("audio") || field.name() == Some("audio_file") {
            audio_name = field.file_name().unwrap_or("sample.wav").to_string();
            audio_bytes = field
                .bytes()
                .await
                .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
                .to_vec();
        }
    }

    if let Some(mid) = message_id {
        let msg = state
            .db
            .get_chat_message_by_message_id(&mid, claims.sub)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
            .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "历史消息不存在"))?;
        let src = msg
            .audio_path
            .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "该消息没有音频"))?;
        audio_bytes = fs::read(&src)
            .await
            .map_err(|_| json_error(StatusCode::NOT_FOUND, "音频文件不存在"))?;
        audio_name = std::path::Path::new(&src)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("sample.wav")
            .to_string();
    } else if audio_bytes.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "请上传音频或选择历史消息"));
    }
    let dir = state
        .data_dir
        .join("audio")
        .join("speaker_samples")
        .join(group_id.to_string());
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let path = dir.join(&audio_name);
    fs::write(&path, &audio_bytes)
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let id = state
        .db
        .add_speaker_sample(group_id, &path.to_string_lossy(), &audio_name)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    if let Ok(Some(svc)) = load_speaker_service(&state.db) {
        if svc.enabled && !svc.base_url.is_empty() {
            let client = SpeakerClient::new(svc);
            let sample_uuid = id.to_string();
            if let Err(e) = client
                .register(
                    &group_id.to_string(),
                    &group.name,
                    &sample_uuid,
                    group.agent_id,
                    claims.sub,
                    audio_bytes,
                    &audio_name,
                )
                .await
            {
                tracing::warn!("声纹样本远程注册失败: {e}");
            }
        }
    }

    Ok(json_data(json!({ "id": id, "file_name": audio_name })))
}

pub async fn sample_file(
    State(state): State<AppState>,
    Path((group_id, sample_id)): Path<(i64, i64)>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let sample = state
        .db
        .get_speaker_sample(sample_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "样本不存在"))?;
    if sample.group_id != group_id {
        return Err(json_error(StatusCode::NOT_FOUND, "样本不存在"));
    }
    let bytes = fs::read(&sample.file_path)
        .await
        .map_err(|_| json_error(StatusCode::NOT_FOUND, "文件不存在"))?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .body(bytes.into())
        .unwrap())
}

pub async fn delete_sample(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((group_id, sample_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let group = find_user_group(&state, claims.sub, group_id)?;
    let sample = state
        .db
        .get_speaker_sample(sample_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .filter(|s| s.group_id == group_id)
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "样本不存在"))?;

    if let Ok(Some(svc)) = load_speaker_service(&state.db) {
        if svc.enabled && !svc.base_url.is_empty() {
            let client = SpeakerClient::new(svc);
            let _ = client
                .delete_sample(
                    &group_id.to_string(),
                    &sample_id.to_string(),
                    group.agent_id,
                    claims.sub,
                )
                .await;
        }
    }

    let ok = state
        .db
        .delete_speaker_sample(group_id, sample_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "样本不存在"));
    }
    let _ = fs::remove_file(&sample.file_path).await;
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn verify(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(group_id): Path<i64>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let group = find_user_group(&state, claims.sub, group_id)?;

    let svc = load_speaker_service(&state.db)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| {
            json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "声纹验证服务未配置，请先在 admin/speaker-configs 中配置",
            )
        })?;

    if !svc.enabled || svc.base_url.is_empty() {
        return Err(json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "声纹验证服务未启用或未配置 base_url",
        ));
    }

    let mut audio_bytes = Vec::new();
    let mut audio_name = "verify.wav".to_string();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
    {
        if field.name() == Some("audio") || field.name() == Some("audio_file") {
            audio_name = field.file_name().unwrap_or("verify.wav").to_string();
            audio_bytes = field
                .bytes()
                .await
                .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e.to_string()))?
                .to_vec();
        }
    }
    if audio_bytes.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "请上传音频文件"));
    }

    let client = SpeakerClient::new(svc);
    let result = client
        .verify(
            &group_id.to_string(),
            group.agent_id,
            claims.sub,
            audio_bytes,
            &audio_name,
        )
        .await
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    Ok(json_data(json!({
        "verified": result.verified,
        "confidence": result.confidence,
        "threshold": result.threshold,
        "speaker_id": group_id.to_string(),
        "speaker_name": group.name,
        "message": verify_message(result.verified, result.confidence),
    })))
}

pub async fn inject_message(
    State(state): State<AppState>,
    AuthUser(_claims): AuthUser,
    Json(body): Json<serde_json::Value>,
) -> Json<Value> {
    inject_message_impl(&state, body).await
}

pub async fn admin_inject_message(
    State(state): State<AppState>,
    AdminUser(_claims): AdminUser,
    Json(body): Json<serde_json::Value>,
) -> Json<Value> {
    inject_message_impl(&state, body).await
}

async fn inject_message_impl(state: &AppState, body: serde_json::Value) -> Json<Value> {
    let device_id = body
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if device_id.is_empty() || message.is_empty() {
        return json_data(json!({ "error": "device_id 和 message 不能为空" }));
    }
    let skip_llm = body.get("skip_llm").and_then(|v| v.as_bool()).unwrap_or(false);
    let auto_listen = body.get("auto_listen").and_then(|v| v.as_bool()).unwrap_or(false);
    let target = body.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let mut payload = json!({
        "device_id": device_id,
        "message": message,
        "skip_llm": skip_llm,
        "auto_listen": auto_listen,
    });
    if !target.trim().is_empty() {
        payload["target"] = json!(target.trim());
    }
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            "/api/device/inject_msg",
            payload,
            std::time::Duration::from_secs(30),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => json_data(resp.body),
        Ok(resp) => json_data(json!({ "error": resp.error, "success": false })),
        Err(e) => json_data(json!({ "error": e, "success": false })),
    }
}
