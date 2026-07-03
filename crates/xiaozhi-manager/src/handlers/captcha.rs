use axum::{extract::State, Json};
use serde_json::Value;

use crate::app::AppState;

pub async fn captcha_status(State(state): State<AppState>) -> Json<Value> {
    let enabled = state.app_config.read().auth.login_captcha_enabled;
    Json(serde_json::json!({ "enabled": enabled }))
}

pub async fn captcha_challenge(State(state): State<AppState>) -> Json<Value> {
    let (id, prompt) = state.captcha.new_challenge();
    Json(serde_json::json!({ "captchaId": id, "prompt": prompt }))
}
