use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::app::{json_data, json_error, AppState};
use crate::db::{AgentInput, AgentRow, DeviceRow};
use crate::extractors::{AdminUser, AuthUser};

fn config_brief(state: &AppState, kind: &str, config_id: &str) -> Value {
    if config_id.is_empty() {
        return json!(null);
    }
    if let Ok(Some(row)) = state.db.find_config_by_type_and_id(kind, config_id) {
        return json!({
            "id": row.id,
            "name": row.name,
            "config_id": row.config_id,
            "provider": row.provider,
        });
    }
    json!({ "config_id": config_id, "name": config_id, "provider": config_id })
}

fn parse_extra(agent: &AgentRow) -> Value {
    serde_json::from_str(&agent.extra_json).unwrap_or(json!({}))
}

pub fn agent_json(state: &AppState, agent: &AgentRow, device_count: i64) -> Value {
    let extra = parse_extra(agent);
    let username = state
        .db
        .find_user_by_id(agent.user_id)
        .ok()
        .flatten()
        .map(|u| u.username)
        .unwrap_or_default();
    json!({
        "id": agent.id,
        "user_id": agent.user_id,
        "username": username,
        "name": agent.name,
        "nickname": extra.get("nickname").and_then(|v| v.as_str()).unwrap_or(&agent.name),
        "custom_prompt": agent.system_prompt,
        "llm_config_id": if agent.llm_provider.is_empty() { Value::Null } else { json!(agent.llm_provider) },
        "tts_config_id": if agent.tts_provider.is_empty() { Value::Null } else { json!(agent.tts_provider) },
        "llm_config": config_brief(state, "llm", &agent.llm_provider),
        "tts_config": config_brief(state, "tts", &agent.tts_provider),
        "voice": extra.get("voice").cloned().unwrap_or(Value::Null),
        "asr_speed": extra.get("asr_speed").and_then(|v| v.as_str()).unwrap_or("normal"),
        "memory_mode": extra.get("memory_mode").and_then(|v| v.as_str()).unwrap_or("short"),
        "speaker_chat_mode": extra.get("speaker_chat_mode").and_then(|v| v.as_str()).unwrap_or("off"),
        "mcp_service_names": extra.get("mcp_service_names").and_then(|v| v.as_str()).unwrap_or(""),
        "openclaw": extra.get("openclaw").cloned().unwrap_or(json!({})),
        "openclaw_config": extra.get("openclaw").cloned().unwrap_or(json!({})).to_string(),
        "created_at": agent.created_at,
        "updated_at": agent.created_at,
        "device_count": device_count,
        "knowledge_base_ids": extra.get("knowledge_base_ids").cloned().unwrap_or(json!([])),
    })
}

pub fn device_json(state: &AppState, device: &DeviceRow) -> Value {
    let agent_name = device.agent_id.and_then(|aid| {
        state
            .db
            .get_agent_by_id(aid)
            .ok()
            .flatten()
            .map(|a| a.name)
    });
    json!({
        "id": device.id,
        "user_id": device.user_id,
        "bind_pending": device.user_id.is_none(),
        "agent_id": device.agent_id.unwrap_or(0),
        "agent_name": agent_name,
        "nick_name": device.name,
        "device_code": device.activation_code,
        "device_name": device.device_id,
        "activated": device.activated,
        "online": device.online,
        "last_active_at": if device.last_active_at.is_empty() {
            if device.online {
                device.created_at.clone()
            } else {
                String::new()
            }
        } else {
            device.last_active_at.clone()
        },
        "created_at": device.created_at,
        "updated_at": device.created_at,
    })
}

pub fn pending_device_json(device: &DeviceRow) -> Value {
    json!({
        "id": device.id,
        "device_name": device.device_id,
        "client_id": device.client_id,
        "created_at": device.created_at,
        "bind_pending": true,
    })
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let agents = state.db.list_agents(claims.sub).unwrap_or_default();
    let data: Vec<Value> = agents
        .iter()
        .map(|a| {
            let count = state.db.count_devices_by_agent(a.id).unwrap_or(0);
            agent_json(&state, a, count)
        })
        .collect();
    json_data(data)
}

pub async fn get(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let agent = state
        .db
        .get_agent(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "智能体不存在"))?;
    let count = state.db.count_devices_by_agent(agent.id).unwrap_or(0);
    Ok(json_data(agent_json(&state, &agent, count)))
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body): Json<AgentInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = state
        .db
        .create_agent(claims.sub, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<AgentInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .update_agent(id, claims.sub, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "智能体不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_agent(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "智能体不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn admin_create(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Json(body): Json<AgentInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user_id = body.user_id.filter(|id| *id > 0).ok_or_else(|| {
        json_error(StatusCode::BAD_REQUEST, "user_id 不能为空")
    })?;
    let id = state
        .db
        .create_agent(user_id, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn admin_update(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
    Json(body): Json<AgentInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .update_agent_by_id(id, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "智能体不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功" })))
}

pub async fn admin_delete(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_agent_by_id(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "智能体不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}
