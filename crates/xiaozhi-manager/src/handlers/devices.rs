use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use crate::app::{json_data, json_error, json_success, AppState};
use crate::db::DeviceInput;
use crate::extractors::{AdminUser, AuthUser};
use crate::handlers::agents::{device_json, pending_device_json};

#[derive(Debug, Deserialize)]
pub struct ClaimDeviceBody {
    pub code: String,
    pub agent_id: Option<i64>,
    #[serde(default)]
    pub nick_name: String,
}

#[derive(Debug, Deserialize)]
pub struct DeviceUpdateBody {
    #[serde(default)]
    pub nick_name: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub client_id: String,
    pub agent_id: Option<i64>,
    #[serde(default)]
    pub user_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct DeviceSpeakBody {
    pub text: String,
    #[serde(default)]
    pub message: String,
    #[serde(default = "default_speak_target")]
    pub target: String,
    #[serde(default)]
    pub auto_listen: bool,
}

fn default_speak_target() -> String {
    "hardware_first".to_string()
}

fn resolved_speak_text(body: &DeviceSpeakBody) -> String {
    if !body.text.trim().is_empty() {
        body.text.trim().to_string()
    } else {
        body.message.trim().to_string()
    }
}

fn resolve_device_id(
    state: &AppState,
    db_id: i64,
    user_id: Option<i64>,
) -> Result<String, (StatusCode, Json<Value>)> {
    let device = if let Some(uid) = user_id {
        state
            .db
            .get_device_by_id(db_id, uid)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    } else {
        state
            .db
            .get_device_by_id_admin(db_id)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    };
    let device = device.ok_or_else(|| json_error(StatusCode::NOT_FOUND, "设备不存在"))?;
    Ok(device.device_id)
}

async fn fetch_device_endpoints(
    state: &AppState,
    device_id: &str,
) -> Json<Value> {
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            "/api/device/endpoints",
            json!({ "device_id": device_id }),
            Duration::from_secs(10),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => json_data(resp.body),
        Ok(resp) => json_data(json!({
            "error": resp.error,
            "device_id": device_id,
            "online": false,
            "endpoint_count": 0,
            "endpoints": [],
        })),
        Err(e) => json_data(json!({
            "error": e,
            "device_id": device_id,
            "online": false,
            "endpoint_count": 0,
            "endpoints": [],
        })),
    }
}

pub async fn user_device_endpoints(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, Some(claims.sub))?;
    Ok(fetch_device_endpoints(&state, &device_id).await)
}

pub async fn admin_device_endpoints(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, None)?;
    Ok(fetch_device_endpoints(&state, &device_id).await)
}

#[derive(Debug, Deserialize)]
pub struct DeviceSignalsQuery {
    #[serde(default)]
    pub after_id: u64,
    #[serde(default)]
    pub clear: bool,
}

async fn fetch_device_signals(
    state: &AppState,
    device_id: &str,
    after_id: u64,
    clear: bool,
) -> Json<Value> {
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            "/api/device/signals",
            json!({
                "device_id": device_id,
                "after_id": after_id,
                "clear": clear,
            }),
            Duration::from_secs(8),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => json_data(resp.body),
        Ok(resp) => json_data(json!({
            "error": resp.error,
            "device_id": device_id,
            "signals": [],
        })),
        Err(e) => json_data(json!({
            "error": e,
            "device_id": device_id,
            "signals": [],
        })),
    }
}

pub async fn user_device_signals(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    axum::extract::Query(query): axum::extract::Query<DeviceSignalsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, Some(claims.sub))?;
    Ok(fetch_device_signals(&state, &device_id, query.after_id, query.clear).await)
}

pub async fn admin_device_signals(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
    axum::extract::Query(query): axum::extract::Query<DeviceSignalsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, None)?;
    Ok(fetch_device_signals(&state, &device_id, query.after_id, query.clear).await)
}

async fn device_speak_impl(
    state: &AppState,
    device_id: &str,
    body: DeviceSpeakBody,
) -> Json<Value> {
    let text = resolved_speak_text(&body);
    if text.is_empty() {
        return json_data(json!({ "error": "text 不能为空", "success": false }));
    }
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            "/api/device/speak",
            json!({
                "device_id": device_id,
                "text": text,
                "target": body.target,
                "auto_listen": body.auto_listen,
            }),
            Duration::from_secs(30),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => json_data(resp.body),
        Ok(resp) => json_data(json!({ "error": resp.error, "success": false })),
        Err(e) => json_data(json!({ "error": e, "success": false })),
    }
}

pub async fn user_device_speak(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<DeviceSpeakBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, Some(claims.sub))?;
    Ok(device_speak_impl(&state, &device_id, body).await)
}

pub async fn admin_device_speak(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
    Json(body): Json<DeviceSpeakBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, None)?;
    Ok(device_speak_impl(&state, &device_id, body).await)
}

async fn device_control_impl(state: &AppState, device_id: &str, action: &str) -> Json<Value> {
    let path = match action {
        "abort" => "/api/device/abort",
        "goodbye" => "/api/device/goodbye",
        _ => return json_data(json!({ "error": "未知操作", "success": false })),
    };
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            path,
            json!({ "device_id": device_id }),
            Duration::from_secs(15),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => json_data(resp.body),
        Ok(resp) => json_data(json!({ "error": resp.error, "success": false })),
        Err(e) => json_data(json!({ "error": e, "success": false })),
    }
}

pub async fn user_device_abort(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, Some(claims.sub))?;
    Ok(device_control_impl(&state, &device_id, "abort").await)
}

pub async fn admin_device_abort(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, None)?;
    Ok(device_control_impl(&state, &device_id, "abort").await)
}

pub async fn user_device_goodbye(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, Some(claims.sub))?;
    Ok(device_control_impl(&state, &device_id, "goodbye").await)
}

pub async fn admin_device_goodbye(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let device_id = resolve_device_id(&state, id, None)?;
    Ok(device_control_impl(&state, &device_id, "goodbye").await)
}

#[derive(Debug, Deserialize, Default)]
pub struct DeviceLiveStatusBody {
    #[serde(default)]
    pub device_ids: Vec<String>,
}

fn collect_device_ids(devices: &[crate::db::DeviceRow]) -> Vec<String> {
    devices
        .iter()
        .map(|d| d.device_id.clone())
        .filter(|id| !id.is_empty())
        .collect()
}

async fn fetch_devices_live_status(
    state: &AppState,
    device_ids: Vec<String>,
) -> Json<Value> {
    if device_ids.is_empty() {
        return json_data(json!({
            "server_connected": state.ws_hub.client_count().await > 0,
            "devices": {},
        }));
    }
    let server_connected = state.ws_hub.client_count().await > 0;
    if !server_connected {
        return json_data(json!({
            "server_connected": false,
            "error": "没有已连接的主服务客户端，请确认 xiaozhi-server 已启动",
            "devices": {},
        }));
    }
    match state
        .ws_hub
        .broadcast_request(
            "POST",
            "/api/device/endpoints/batch",
            json!({ "device_ids": device_ids }),
            Duration::from_secs(8),
        )
        .await
    {
        Ok(resp) if resp.status < 400 => {
            let mut body = resp.body;
            if let Some(obj) = body.as_object_mut() {
                obj.insert("server_connected".to_string(), json!(true));
            } else {
                body = json!({ "server_connected": true, "devices": {} });
            }
            json_data(body)
        }
        Ok(resp) => json_data(json!({
            "server_connected": true,
            "error": resp.error,
            "devices": {},
        })),
        Err(e) => json_data(json!({
            "server_connected": true,
            "error": e,
            "devices": {},
        })),
    }
}

pub async fn user_live_status(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body): Json<DeviceLiveStatusBody>,
) -> Json<Value> {
    let user_devices = state.db.list_devices(claims.sub).unwrap_or_default();
    let allowed: std::collections::HashSet<String> =
        collect_device_ids(&user_devices).into_iter().collect();
    let device_ids = if body.device_ids.is_empty() {
        allowed.iter().cloned().collect()
    } else {
        body.device_ids
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty() && allowed.contains(id))
            .collect()
    };
    fetch_devices_live_status(&state, device_ids).await
}

pub async fn admin_live_status(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Json(body): Json<DeviceLiveStatusBody>,
) -> Json<Value> {
    let device_ids = if body.device_ids.is_empty() {
        let devices = state.db.list_all_devices().unwrap_or_default();
        collect_device_ids(&devices)
    } else {
        body.device_ids
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect()
    };
    fetch_devices_live_status(&state, device_ids).await
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let devices = state.db.list_devices(claims.sub).unwrap_or_default();
    let data: Vec<Value> = devices.iter().map(|d| device_json(&state, d)).collect();
    json_data(data)
}

pub async fn list_pending(
    State(state): State<AppState>,
    AuthUser(_): AuthUser,
) -> Json<Value> {
    let devices = state.db.list_pending_devices().unwrap_or_default();
    let data: Vec<Value> = devices.iter().map(pending_device_json).collect();
    json_data(data)
}

pub async fn claim(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body): Json<ClaimDeviceBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let agent_id = body.agent_id.filter(|id| *id > 0);
    let device = state
        .db
        .claim_device_by_code(claims.sub, &body.code, agent_id, &body.nick_name)
        .map_err(|e| json_error(StatusCode::BAD_REQUEST, &e))?;
    Ok(json_success(device_json(&state, &device)))
}

pub async fn admin_list(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Json<Value> {
    let devices = state.db.list_all_devices().unwrap_or_default();
    let data: Vec<Value> = devices.iter().map(|d| device_json(&state, d)).collect();
    json_data(data)
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body): Json<DeviceInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.resolved_device_id().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "device_id 不能为空"));
    }
    let id = state
        .db
        .create_device(claims.sub, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn admin_create(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Json(body): Json<DeviceInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user_id = body.user_id.filter(|id| *id > 0).ok_or_else(|| {
        json_error(StatusCode::BAD_REQUEST, "user_id 不能为空")
    })?;
    if body.resolved_device_id().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "device_id 不能为空"));
    }
    let id = state
        .db
        .create_device(user_id, &body)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({ "id": id, "message": "创建成功" })))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<DeviceUpdateBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let input = device_body_to_input(&body);
    let ok = state
        .db
        .update_device(id, claims.sub, &input)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
    }
    Ok(json_data(json!({ "message": "更新成功", "nick_name": input.name })))
}

pub async fn admin_update(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
    Json(body): Json<DeviceUpdateBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let input = device_body_to_input(&body);
    let ok = state
        .db
        .update_device_by_id(id, &input)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
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
        .delete_device(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn admin_delete(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_device_by_id(id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

fn device_body_to_input(body: &DeviceUpdateBody) -> DeviceInput {
    DeviceInput {
        device_id: if body.device_name.is_empty() {
            String::new()
        } else {
            body.device_name.clone()
        },
        client_id: body.client_id.clone(),
        name: body.nick_name.clone(),
        agent_id: crate::db::normalize_agent_id(body.agent_id),
        user_id: body.user_id,
    }
}
