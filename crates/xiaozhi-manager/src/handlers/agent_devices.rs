use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{json_data, json_error, json_success, AppState};
use crate::extractors::{AdminUser, AuthUser};
use crate::handlers::agents::{agent_json, device_json};

#[derive(Deserialize)]
pub struct BindDeviceRequest {
    #[serde(default, alias = "device_id")]
    pub id: Option<i64>,
    #[serde(default, alias = "device_name")]
    pub device_name: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub device_mac: String,
    #[serde(default)]
    pub nick_name: String,
}

pub async fn admin_list(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Json<Value> {
    let agents = state.db.list_all_agents().unwrap_or_default();
    let data: Vec<Value> = agents
        .iter()
        .map(|a| {
            let count = state.db.count_devices_by_agent(a.id).unwrap_or(0);
            agent_json(&state, a, count)
        })
        .collect();
    json_data(data)
}

pub async fn list_agent_devices(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(agent_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let agent = state
        .db
        .get_agent(agent_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "智能体不存在"))?;
    let devices = state
        .db
        .list_devices_by_agent(agent.id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let data: Vec<Value> = devices.iter().map(|d| device_json(&state, d)).collect();
    Ok(json_data(data))
}

pub async fn add_device_to_agent(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(agent_id): Path<i64>,
    Json(body): Json<BindDeviceRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let agent = state
        .db
        .get_agent(agent_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "智能体不存在"))?;

    let code = body.code.trim();
    let device_mac = if body.device_mac.trim().is_empty() {
        body.device_name.trim()
    } else {
        body.device_mac.trim()
    };

    if !code.is_empty() || !device_mac.is_empty() {
        let device = state
            .db
            .bind_device_to_agent_by_identifier(
                agent.id,
                claims.sub,
                code,
                device_mac,
                body.nick_name.trim(),
            )
            .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;
        return Ok(json_success(device_json(&state, &device)));
    }

    let device_row_id = if let Some(id) = body.id {
        id
    } else if !body.device_name.is_empty() {
        let mac = crate::db::normalize_device_mac(&body.device_name);
        if let Ok(Some(device)) = state.db.find_device_by_device_id(&mac) {
            if device.user_id.is_none() || device.user_id == Some(claims.sub) {
                device.id
            } else {
                return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
            }
        } else if let Ok(Some(device)) = state.db.find_device_by_device_id(body.device_name.trim()) {
            if device.user_id.is_none() || device.user_id == Some(claims.sub) {
                device.id
            } else {
                return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
            }
        } else {
            let devices = state.db.list_devices(claims.sub).unwrap_or_default();
            devices
                .into_iter()
                .find(|d| d.device_id == body.device_name || d.device_id == mac)
                .map(|d| d.id)
                .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "设备不存在"))?
        }
    } else {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "请填写设备验证码或设备 MAC",
        ));
    };

    let device = state
        .db
        .get_device_by_id_admin(device_row_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "设备不存在"))?;

    let ok = if device.user_id.is_none() {
        state
            .db
            .assign_device_to_user_agent(
                device_row_id,
                claims.sub,
                agent.id,
                body.nick_name.trim(),
            )
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    } else if device.user_id == Some(claims.sub) {
        state
            .db
            .bind_device_to_agent(agent.id, claims.sub, device_row_id)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    } else {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
    };

    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备绑定失败"));
    }

    let device = state
        .db
        .get_device_by_id(device_row_id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "设备不存在"))?;
    Ok(json_success(device_json(&state, &device)))
}

pub async fn remove_device_from_agent(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((agent_id, device_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .unbind_device_from_agent(agent_id, claims.sub, device_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在或未绑定"));
    }
    Ok(Json(serde_json::json!({ "success": true, "message": "设备移除成功" })))
}
