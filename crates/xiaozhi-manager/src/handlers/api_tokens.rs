use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{json_data, json_error, AppState};
use crate::extractors::AuthUser;

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Json<Value> {
    let tokens = state.db.list_api_tokens(claims.sub).unwrap_or_default();
    json_data(tokens)
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if req.name.trim().is_empty() {
        return Err(json_error(StatusCode::BAD_REQUEST, "Token 名称不能为空"));
    }
    let raw_token = format!("xz_{}", uuid::Uuid::new_v4());
    let prefix: String = raw_token.chars().take(8).collect();
    let hash = crate::auth::hash_password(&raw_token)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let id = state
        .db
        .create_api_token(
            claims.sub,
            &req.name,
            &hash,
            &prefix,
            req.expires_at.as_deref(),
        )
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(json_data(json!({
        "id": id,
        "token": raw_token,
        "message": "创建成功，请妥善保存 Token（仅显示一次）",
    })))
}

pub async fn delete(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let ok = state
        .db
        .delete_api_token(id, claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if !ok {
        return Err(json_error(StatusCode::NOT_FOUND, "Token 不存在"));
    }
    Ok(json_data(json!({ "message": "删除成功" })))
}

pub async fn mcp_service_options(State(state): State<AppState>) -> Json<Value> {
    crate::handlers::mcp::service_options(State(state)).await
}
