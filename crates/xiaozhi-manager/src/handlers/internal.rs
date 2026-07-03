use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use xiaozhi_config::user::UConfig;

use crate::app::{json_error, json_ok, AppState};
use crate::db::activation_bind_message;

#[derive(Deserialize)]
pub struct DeviceQuery {
    pub device_id: String,
    pub client_id: Option<String>,
}

pub async fn device_activated(
    State(state): State<AppState>,
    Query(q): Query<DeviceQuery>,
) -> Json<serde_json::Value> {
    let client_id = q.client_id.unwrap_or_default();
    if xiaozhi_core::constants::ota_test::is_probe_device(&q.device_id)
        || xiaozhi_core::constants::simulator::is_simulator_device(&q.device_id)
    {
        return Json(serde_json::json!({
            "activated": true,
            "device_id": q.device_id,
            "client_id": client_id
        }));
    }
    let activated = if !state.app_config.read().auth.enable {
        true
    } else {
        state
            .db
            .find_device_by_device_id(&q.device_id)
            .ok()
            .flatten()
            .map(|d| d.activated)
            .unwrap_or(false)
    };
    if activated && !client_id.is_empty() {
        let _ = state
            .db
            .sync_activated_client_id(&q.device_id, &client_id);
    }
    Json(serde_json::json!({ "activated": activated, "device_id": q.device_id, "client_id": client_id }))
}

pub async fn device_activation(
    State(state): State<AppState>,
    Query(q): Query<DeviceQuery>,
) -> Json<serde_json::Value> {
    let client_id = q.client_id.unwrap_or_default();

    if xiaozhi_core::constants::ota_test::is_probe_device(&q.device_id)
        || xiaozhi_core::constants::simulator::is_simulator_device(&q.device_id)
    {
        return Json(serde_json::json!({
            "code": "000000",
            "message": "OTA 测试设备",
            "challenge": "",
            "expires_in": 0
        }));
    }

    if let Ok(Some(device)) = state.db.find_device_by_device_id(&q.device_id) {
        if device.activated {
            return Json(serde_json::json!({
                "code": "000000",
                "message": "设备已激活",
                "challenge": "",
                "expires_in": 0
            }));
        }

        let code = state
            .db
            .ensure_device_activation_code(&q.device_id)
            .unwrap_or(device.activation_code);
        let challenge = state
            .db
            .refresh_activation_challenge(&q.device_id, &client_id, &code)
            .unwrap_or_default();
        let message = activation_bind_message(&code);
        return Json(serde_json::json!({
            "code": code,
            "message": message,
            "challenge": challenge,
            "expires_in": 300
        }));
    }

    match state.db.ensure_pending_device(&q.device_id, &client_id) {
        Ok(device) => {
            let challenge = state
                .db
                .get_activation_challenge(&q.device_id, &client_id)
                .ok()
                .flatten()
                .map(|(_, _, c, _)| c)
                .unwrap_or_default();
            let message = activation_bind_message(&device.activation_code);
            Json(serde_json::json!({
                "code": device.activation_code,
                "message": message,
                "challenge": challenge,
                "expires_in": 300
            }))
        }
        Err(e) => Json(serde_json::json!({
            "code": "000000",
            "message": format!("设备注册失败: {e}"),
            "challenge": "",
            "expires_in": 0
        })),
    }
}

#[derive(Deserialize)]
pub struct ActivateBody {
    pub device_id: String,
    pub client_id: String,
    pub payload: serde_json::Value,
}

pub async fn device_activate(
    State(state): State<AppState>,
    Json(body): Json<ActivateBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let code = body
        .payload
        .get("code")
        .or_else(|| body.payload.get("activation_code"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let challenge = body.payload.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
    let hmac = body.payload.get("hmac").and_then(|v| v.as_str()).unwrap_or("");

    let Some(device) = state
        .db
        .find_device_by_device_id(&body.device_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    else {
        return Ok(json_ok(serde_json::json!({ "message": "设备未注册" })));
    };

    if device.activated {
        let _ = state
            .db
            .sync_activated_client_id(&body.device_id, &body.client_id);
        return Ok(json_ok(serde_json::json!({ "message": "设备已激活" })));
    }

    if crate::db::Database::is_unbound_device(&device) {
        return Err(json_error(
            StatusCode::ACCEPTED,
            "设备尚未绑定，请先在控制台输入验证码完成绑定",
        ));
    }

    if !code.is_empty() {
        if device.activation_code == code {
            state
                .db
                .activate_device(&body.device_id, &body.client_id)
                .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            return Ok(json_ok(serde_json::json!({ "message": "激活成功" })));
        }
        return Err(json_error(StatusCode::BAD_REQUEST, "激活码错误"));
    }

    if !hmac.is_empty() && !challenge.is_empty() {
        let stored = state
            .db
            .get_activation_challenge(&body.device_id, &body.client_id)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        if let Some((_, _, stored_challenge, _)) = stored {
            if stored_challenge != challenge {
                return Err(json_error(StatusCode::BAD_REQUEST, "挑战码错误"));
            }
        }
        state
            .db
            .activate_device(&body.device_id, &body.client_id)
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        return Ok(json_ok(serde_json::json!({ "message": "激活成功" })));
    }

    Err(json_error(
        StatusCode::ACCEPTED,
        "等待用户在控制台完成绑定",
    ))
}

pub async fn device_config(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Json<UConfig> {
    Json(crate::uconfig_builder::build_for_device_id(
        &state.db,
        &state.app_config.read(),
        &device_id,
    ))
}

pub async fn system_config(State(state): State<AppState>) -> Result<String, (StatusCode, String)> {
    let bundle = crate::system_configs::build_system_configs_data(&state.db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    serde_json::to_string(&bundle)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Deserialize)]
pub struct ChatHistoryBody {
    pub device_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
}

pub async fn save_chat(
    State(state): State<AppState>,
    Json(body): Json<ChatHistoryBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .db
        .save_chat(&body.device_id, &body.session_id, &body.role, &body.content)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_ok(serde_json::json!({ "message": "ok" })))
}

pub async fn pool_stats(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    if let Some(stats) = body.get("stats") {
        state.pool_stats.save(stats.clone());
    } else if body.is_object() && !body.as_object().unwrap().is_empty() {
        state.pool_stats.save(body);
    }
    Json(serde_json::json!({
        "message": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

#[derive(Deserialize)]
pub struct DevicePresenceBody {
    pub device_id: String,
    pub online: bool,
}

#[derive(Deserialize)]
pub struct DeviceTouchBody {
    pub device_id: String,
}

pub async fn device_touch(
    State(state): State<AppState>,
    Json(body): Json<DeviceTouchBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let device_id = body.device_id.trim();
    if device_id.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "device_id 不能为空"));
    }
    let ok = state
        .db
        .touch_device_last_active(device_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
    }
    Ok(json_ok(serde_json::json!({ "device_id": device_id })))
}

pub async fn device_presence(
    State(state): State<AppState>,
    Json(body): Json<DevicePresenceBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let device_id = body.device_id.trim();
    if device_id.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "device_id 不能为空"));
    }
    let ok = state
        .db
        .set_device_presence(device_id, body.online)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "设备不存在"));
    }
    Ok(json_ok(serde_json::json!({
        "device_id": device_id,
        "online": body.online,
    })))
}

#[derive(Deserialize)]
pub struct SwitchRoleBody {
    pub role_name: String,
}

pub async fn switch_device_role(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(body): Json<SwitchRoleBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let role_name = body.role_name.trim();
    if role_name.is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "role_name 不能为空"));
    }
    let device = state
        .db
        .find_device_by_device_id(&device_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "设备不存在"))?;
    let user_id = device
        .user_id
        .ok_or_else(|| json_error(StatusCode::BAD_REQUEST, "设备尚未绑定用户"))?;
    let role = state
        .db
        .find_role_by_name(user_id, role_name)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "未找到匹配的角色"))?;
    state
        .db
        .set_device_role(device.id, Some(role.id), &role.name)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_ok(serde_json::json!({ "role_name": role.name })))
}

pub async fn restore_device_default_role(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let device = state
        .db
        .find_device_by_device_id(&device_id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "设备不存在"))?;
    state
        .db
        .clear_device_role(device.id)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_ok(serde_json::json!({ "role_name": "default" })))
}

#[derive(Deserialize)]
pub struct KnowledgeSearchBody {
    #[serde(default)]
    pub knowledge_base_ids: Vec<i64>,
    pub query: String,
    #[serde(default = "search_default_top_k")]
    pub top_k: i64,
    #[serde(default = "search_default_threshold")]
    pub threshold: f64,
}

fn search_default_top_k() -> i64 {
    3
}

fn search_default_threshold() -> f64 {
    0.2
}

pub async fn knowledge_search(
    State(state): State<AppState>,
    Json(body): Json<KnowledgeSearchBody>,
) -> Json<serde_json::Value> {
    let hits = crate::knowledge_search::search_knowledge_bases(
        &state.db,
        &body.knowledge_base_ids,
        &body.query,
        body.top_k as usize,
        body.threshold,
    )
    .await;
    let results: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "title": h.title,
                "content": h.content,
                "score": h.score,
                "document_id": h.document_id,
                "knowledge_base_id": h.knowledge_base_id,
            })
        })
        .collect();
    Json(serde_json::json!({ "results": results }))
}
