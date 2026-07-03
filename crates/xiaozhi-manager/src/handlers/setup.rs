use axum::{extract::State, Json};
use serde::Deserialize;

use crate::app::{json_error, json_ok, AppState};

#[derive(Deserialize)]
pub struct SetupRequest {
    pub admin_username: String,
    pub admin_password: String,
    pub admin_email: String,
}

pub async fn status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let count = state.db.admin_count().unwrap_or(0);
    Json(serde_json::json!({
        "needs_setup": count == 0,
        "message": if count == 0 { "需要创建管理员账户" } else { "系统已初始化" }
    }))
}

pub async fn local_ip() -> Json<serde_json::Value> {
    let ip = crate::network::primary_lan_ip();
    Json(serde_json::json!({
        "ip": ip,
    }))
}

pub async fn initialize(
    State(state): State<AppState>,
    Json(req): Json<SetupRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    if state.db.admin_count().unwrap_or(0) > 0 {
        return Err(json_error(axum::http::StatusCode::BAD_REQUEST, "系统已初始化"));
    }
    if req.admin_username.len() < 3 {
        return Err(json_error(axum::http::StatusCode::BAD_REQUEST, "用户名至少 3 个字符"));
    }
    if req.admin_password.len() < 6 {
        return Err(json_error(axum::http::StatusCode::BAD_REQUEST, "密码至少 6 个字符"));
    }

    let hash = crate::auth::hash_password(&req.admin_password)
        .map_err(|e| json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    state
        .db
        .create_user(&req.admin_username, &hash, &req.admin_email, "admin")
        .map_err(|e| json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    Ok(json_ok(serde_json::json!({
        "message": "初始化成功",
        "admin": {
            "username": req.admin_username,
            "email": req.admin_email,
            "role": "admin"
        }
    })))
}
