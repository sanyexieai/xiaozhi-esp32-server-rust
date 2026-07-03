use axum::{extract::State, http::StatusCode, Json};

use crate::app::{json_error, json_ok, AppState};
use crate::auth::user_json;
use crate::extractors::AuthUser;

#[derive(serde::Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    #[serde(default, rename = "captchaId")]
    pub captcha_id: String,
    #[serde(default, rename = "captchaAnswer")]
    pub captcha_answer: String,
}

#[derive(serde::Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub email: String,
    #[serde(default, rename = "captchaId")]
    pub captcha_id: String,
    #[serde(default, rename = "captchaAnswer")]
    pub captcha_answer: String,
}

fn verify_captcha(state: &AppState, id: &str, answer: &str, required: bool) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if id.is_empty() {
        if required {
            return Err(json_error(StatusCode::BAD_REQUEST, "请完成人机验证"));
        }
        return Ok(());
    }
    if !state.captcha.verify(id, answer) {
        return Err(json_error(StatusCode::BAD_REQUEST, "验证码错误"));
    }
    Ok(())
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let captcha_required = state.app_config.read().auth.login_captcha_enabled;
    verify_captcha(
        &state,
        &req.captcha_id,
        &req.captcha_answer,
        captcha_required,
    )?;

    let user = state
        .db
        .find_user_by_username(&req.username)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::UNAUTHORIZED, "用户名或密码错误"))?;

    if !crate::auth::verify_password(&req.password, &user.password_hash) {
        return Err(json_error(StatusCode::UNAUTHORIZED, "用户名或密码错误"));
    }

    let token = crate::auth::create_token(user.id, &user.username, &user.role)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    Ok(json_ok(serde_json::json!({
        "token": token,
        "user": user_json(&user),
    })))
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if req.username.len() < 3 {
        return Err(json_error(StatusCode::BAD_REQUEST, "用户名至少 3 个字符"));
    }
    if req.password.len() < 6 {
        return Err(json_error(StatusCode::BAD_REQUEST, "密码至少 6 个字符"));
    }
    verify_captcha(&state, &req.captcha_id, &req.captcha_answer, true)?;

    if state
        .db
        .find_user_by_username(&req.username)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .is_some()
    {
        return Err(json_error(StatusCode::BAD_REQUEST, "用户名已存在"));
    }

    let hash = crate::auth::hash_password(&req.password)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    state
        .db
        .create_user(&req.username, &hash, &req.email, "user")
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    Ok(json_ok(serde_json::json!({ "message": "注册成功" })))
}

pub async fn profile(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user = state
        .db
        .find_user_by_id(claims.sub)
        .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| json_error(StatusCode::NOT_FOUND, "用户不存在"))?;
    Ok(json_ok(serde_json::json!({ "user": user_json(&user) })))
}
